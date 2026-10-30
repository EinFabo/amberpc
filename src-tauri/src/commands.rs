//! What the settings window and the bubble can ask for. Anything that may take a while runs
//! off the main thread, so the windows never freeze.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use amber_core::engine::OpenAiTranscriber;
use amber_core::local::{self, DownloadError};
use amber_core::providers;
use amber_core::settings::{EngineKind, Settings};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::bubble::{self, BubbleConfig};
use crate::capture::{self, Recorder};
use crate::state::Core;
use crate::whisper;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    id: &'static str,
    name: &'static str,
    base_url: &'static str,
    models: Vec<ModelInfo>,
    key_url: &'static str,
    needs_key: bool,
    has_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    id: &'static str,
    label: &'static str,
    note: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    settings: Settings,
    version: &'static str,
    providers: Vec<ProviderInfo>,
    phase: crate::dictation::Phase,
}

#[tauri::command]
pub fn get_overview(core: State<Core>) -> Overview {
    Overview {
        settings: core.settings(),
        version: env!("CARGO_PKG_VERSION"),
        providers: providers_info(&core),
        phase: core.dictation.phase(),
    }
}

fn providers_info(core: &Core) -> Vec<ProviderInfo> {
    providers::ALL
        .iter()
        .map(|p| ProviderInfo {
            id: p.id,
            name: p.name,
            base_url: p.base_url,
            models: p.models.iter().map(|m| ModelInfo { id: m.id, label: m.label, note: m.note }).collect(),
            key_url: p.key_url,
            needs_key: p.needs_key,
            has_key: core.has_key(p.id),
        })
        .collect()
}

/// Saves the settings and applies what changed. A hotkey that cannot be registered is
/// rolled back and reported; everything else is kept.
#[tauri::command]
pub fn save_settings(app: AppHandle, core: State<Core>, settings: Settings) -> Result<Settings, String> {
    let before = core.settings();
    if let Some(base) = (settings.server.provider == providers::CUSTOM.id).then(|| settings.server.base_url.trim()).filter(|b| !b.is_empty()) {
        providers::check_base_url(base)?;
    }
    let mut after = core.update_settings(|s| *s = settings);

    let mut problem = None;
    if after.hotkey != before.hotkey {
        if let Err(e) = crate::hotkeys::register(&app, &after) {
            problem = Some(format!("Hotkey not changed: {e}"));
            after = core.update_settings(|s| s.hotkey = before.hotkey.clone());
            let _ = crate::hotkeys::register(&app, &after);
        }
    }
    if after.bubble != before.bubble || after.sounds != before.sounds {
        bubble::apply(&app);
    }
    if after.autostart != before.autostart {
        crate::autostart::set(after.autostart);
    }
    let local_changed = after.local != before.local || after.engine != before.engine;
    if local_changed {
        let app2 = app.clone();
        let after2 = after.clone();
        std::thread::spawn(move || {
            let core = app2.state::<Core>();
            if after2.engine == EngineKind::Server {
                core.whisper.stop(&app2);
            } else if after2.local.keep_loaded {
                let _ = core.whisper.ensure(&app2);
            }
        });
    }
    // Also a profile that was the only reason the local model runs ("Always"), or the first.
    if after.cleanup != before.cleanup || crate::llm::wants_local(&after) != crate::llm::wants_local(&before) {
        // ensure() reads the settings under its lock: it loads, swaps or stops the model,
        // whichever the latest settings want.
        let app = app.clone();
        std::thread::spawn(move || {
            let _ = app.state::<Core>().llm.ensure(&app);
        });
    }
    core.memory.set_enabled(after.history);
    if before.history && !after.history {
        // Switched off: what was kept goes too.
        core.memory.clear_history();
    } else if after.history_days < before.history_days {
        core.memory.prune(after.history_days);
    }
    if after.phone != before.phone {
        // Stopping may wait for a phone dictation in progress.
        let app = app.clone();
        std::thread::spawn(move || {
            crate::phone::sync(&app);
            crate::tunnel::sync(&app);
        });
    }
    let _ = app.emit("settings-changed", &after);
    match problem {
        Some(p) => Err(p),
        None => Ok(after),
    }
}

#[tauri::command]
pub fn set_api_key(core: State<Core>, provider: String, key: Option<String>) -> Result<bool, String> {
    providers::by_id(&provider).ok_or("Unknown provider.")?;
    core.set_key(&provider, key)?;
    Ok(core.has_key(&provider))
}

/// Asks the server for its model list: address reachable, key accepted? Returns the round
/// trip in milliseconds. [key] none = the stored one.
#[tauri::command]
pub async fn test_server(app: AppHandle, provider: String, base_url: String, key: Option<String>) -> Result<u64, String> {
    let p = providers::by_id(&provider).ok_or("Unknown provider.")?;
    let base = providers::effective_base_url(p, &base_url);
    providers::check_base_url(&base)?;
    let key = key.filter(|k| !k.trim().is_empty()).or_else(|| app.state::<Core>().key(p.id)).unwrap_or_default();
    if p.needs_key && key.is_empty() {
        return Err(format!("Enter your {} API key first.", p.name));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let t0 = Instant::now();
        OpenAiTranscriber::new(&base, &key, "").check().map(|_| t0.elapsed().as_millis() as u64).map_err(|e| e.message)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct Microphones {
    devices: Vec<String>,
    default: Option<String>,
}

#[tauri::command]
pub async fn list_microphones() -> Microphones {
    tauri::async_runtime::spawn_blocking(|| {
        let (devices, default) = capture::list();
        Microphones { devices, default }
    })
    .await
    .unwrap_or(Microphones { devices: vec![], default: None })
}

/// The settings window's level meter. Not while dictating; ends itself after a minute.
#[tauri::command]
pub async fn mic_test(app: AppHandle, device: Option<String>) -> Result<(), String> {
    let core = app.state::<Core>();
    drop(core.mic_test.lock().unwrap().take());
    let Some(device) = device else { return Ok(()) };
    if core.dictation.is_active() {
        return Err("A dictation is running.".into());
    }
    let app2 = app.clone();
    let recorder = tauri::async_runtime::spawn_blocking(move || {
        Recorder::start(&device, move |level| {
            let _ = app2.emit_to(bubble::SETTINGS, "mic-level", level);
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    let started = Instant::now();
    *core.mic_test.lock().unwrap() = Some((recorder, started));
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let core = app.state::<Core>();
        let mut slot = core.mic_test.lock().unwrap();
        // Only this test's recorder, not one started later.
        if slot.as_ref().is_some_and(|(_, s)| *s == started) {
            drop(slot.take());
        }
    });
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalItem {
    id: &'static str,
    label: &'static str,
    note: &'static str,
    size_mb: u32,
    installed: bool,
    wants_gpu: bool,
    /// Kept for existing downloads; shown only while installed.
    replaced: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalInfo {
    runtimes: Vec<LocalItem>,
    models: Vec<LocalItem>,
    has_nvidia: bool,
    recommended: (&'static str, &'static str),
    /// Model id -> measured milliseconds for 10 s of speech on the current runtime.
    measured: std::collections::HashMap<&'static str, u64>,
    status: whisper::Status,
    downloads: Vec<String>,
    folder: String,
    llm_runtimes: Vec<LocalItem>,
    llm_models: Vec<LocalItem>,
    llm_status: whisper::Status,
}

#[tauri::command]
pub fn local_info(core: State<Core>) -> LocalInfo {
    let layout = &core.paths.layout;
    let has_nvidia = has_nvidia();
    LocalInfo {
        runtimes: local::RUNTIMES
            .iter()
            .map(|r| LocalItem { id: r.id, label: r.label, note: r.note, size_mb: r.size_mb, installed: layout.runtime_installed(r), wants_gpu: r.id == "cuda", replaced: false })
            .collect(),
        models: local::MODELS
            .iter()
            .map(|m| LocalItem { id: m.id, label: m.label, note: m.note, size_mb: m.size_mb, installed: layout.model_installed(m), wants_gpu: m.wants_gpu, replaced: m.replaced })
            .collect(),
        has_nvidia,
        recommended: local::recommend(has_nvidia, threads(), total_ram_mb()),
        measured: local::MODELS
            .iter()
            .filter_map(|m| {
                let key = amber_core::estimate::key("local", &core.settings().local.runtime, m.id);
                // What a 10-second dictation takes on this PC, once measured.
                core.estimates.rate(&key).map(|r| (m.id, 200 + (10_000.0 * r.per_ms) as u64))
            })
            .collect(),
        status: core.whisper.status(),
        downloads: core.downloads.lock().unwrap().keys().cloned().collect(),
        folder: layout.root.display().to_string(),
        llm_runtimes: local::LLM_RUNTIMES
            .iter()
            .map(|r| LocalItem { id: r.id, label: r.label, note: r.note, size_mb: r.size_mb(), installed: layout.llm_runtime_installed(r), wants_gpu: r.id == "cuda", replaced: false })
            .collect(),
        llm_models: local::LLM_MODELS
            .iter()
            .map(|m| LocalItem { id: m.id, label: m.label, note: m.note, size_mb: m.size_mb, installed: layout.llm_model_installed(m), wants_gpu: true, replaced: false })
            .collect(),
        llm_status: core.llm.status(),
    }
}

fn threads() -> u32 {
    std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4)
}

/// Installed memory, for the model suggestion.
#[cfg(windows)]
fn total_ram_mb() -> u64 {
    use windows::Win32::System::SystemInformation::{GetPhysicallyInstalledSystemMemory, GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // What is built in (8 GB), not what Windows can use after the graphics took its share
    // (often 7.2-7.7 GB on laptops) - the suggestion is about the machine.
    let mut kb = 0u64;
    if unsafe { GetPhysicallyInstalledSystemMemory(&mut kb) }.is_ok() && kb > 0 {
        return kb / 1024;
    }
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    match unsafe { GlobalMemoryStatusEx(&mut status) } {
        Ok(()) => status.ullTotalPhys / (1 << 20),
        Err(_) => 0,
    }
}

#[cfg(not(windows))]
fn total_ram_mb() -> u64 {
    0
}

/// The NVIDIA driver puts its CUDA library into System32.
fn has_nvidia() -> bool {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    std::path::Path::new(&root).join("System32").join("nvcuda.dll").exists()
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadEvent {
    key: String,
    done: u64,
    total: Option<u64>,
    /// "running", "done", "failed", "cancelled"
    state: &'static str,
    message: Option<String>,
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Runtime,
    Model,
    LlmRuntime,
    LlmModel,
    /// cloudflared, for the phone API from anywhere (id "cloudflared").
    Tunnel,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Runtime => "runtime",
            Kind::Model => "model",
            Kind::LlmRuntime => "llmRuntime",
            Kind::LlmModel => "llmModel",
            Kind::Tunnel => "tunnel",
        }
    }
    fn is_llm(self) -> bool {
        matches!(self, Kind::LlmRuntime | Kind::LlmModel)
    }
}

fn key_of(kind: Kind, id: &str) -> String {
    format!("{}:{id}", kind.name())
}

/// One file to fetch: where from, its checksum, where to, and roughly how big.
struct Fetch {
    url: String,
    sha256: &'static str,
    file: std::path::PathBuf,
    size: u64,
}

/// Unpacks runtime archives once all of them are verified.
type Install = Box<dyn FnOnce(&AppHandle) -> Result<(), DownloadError> + Send>;

fn plan(core: &Core, kind: Kind, id: &str) -> Result<(Vec<Fetch>, Option<Install>), String> {
    let layout = core.paths.layout.clone();
    let downloads = layout.root.join("downloads");
    const MB: u64 = 1 << 20;
    Ok(match kind {
        Kind::Runtime => {
            let rt = local::runtime(id).ok_or("Unknown runtime.")?;
            let zip = downloads.join(format!("whisper-{}-{}.zip", rt.id, local::WHISPER_CPP_BUILD));
            let fetch = Fetch { url: rt.url.into(), sha256: rt.sha256, file: zip.clone(), size: rt.size_mb as u64 * MB };
            let install: Install = Box::new(move |app| {
                // A runtime in use cannot be replaced while its server runs.
                app.state::<Core>().whisper.stop_runtime(app, rt.id);
                let r = local::install_runtime(&zip, &layout.runtime_dir(rt), rt);
                let _ = std::fs::remove_file(&zip);
                r
            });
            (vec![fetch], Some(install))
        }
        Kind::Model => {
            let m = local::model(id).ok_or("Unknown model.")?;
            (vec![Fetch { url: m.url(), sha256: m.sha256, file: layout.model_path(m), size: m.size_mb as u64 * MB }], None)
        }
        Kind::Tunnel => {
            let t = &local::CLOUDFLARED;
            (vec![Fetch { url: t.url.into(), sha256: t.sha256, file: layout.cloudflared_exe(), size: t.size_mb as u64 * MB }], None)
        }
        Kind::LlmRuntime => {
            let rt = local::llm_runtime(id).ok_or("Unknown runtime.")?;
            let fetches: Vec<Fetch> = rt
                .archives
                .iter()
                .enumerate()
                .map(|(i, a)| Fetch {
                    url: a.url.into(),
                    sha256: a.sha256,
                    file: downloads.join(format!("llama-{}-{}-{i}.zip", rt.id, local::LLAMA_CPP_BUILD)),
                    size: a.size_mb as u64 * MB,
                })
                .collect();
            let zips: Vec<std::path::PathBuf> = fetches.iter().map(|f| f.file.clone()).collect();
            let install: Install = Box::new(move |app| {
                app.state::<Core>().llm.stop_runtime(app, rt.id);
                let paths: Vec<&std::path::Path> = zips.iter().map(|z| z.as_path()).collect();
                let r = local::install_llm_runtime(&paths, &layout.llm_runtime_dir(rt), rt);
                for z in &zips {
                    let _ = std::fs::remove_file(z);
                }
                r
            });
            (fetches, Some(install))
        }
        Kind::LlmModel => {
            let m = local::llm_model(id).ok_or("Unknown model.")?;
            (vec![Fetch { url: m.url.into(), sha256: m.sha256, file: layout.llm_model_path(m), size: m.size_mb as u64 * MB }], None)
        }
    })
}

/// Downloads a runtime or a model in the background; progress arrives as "download" events.
/// [select]: make it the one in use once it is installed - done here, not in the window,
/// which may be closed long before a large model arrives.
#[tauri::command]
pub fn download(app: AppHandle, core: State<Core>, kind: Kind, id: String, select: bool) -> Result<(), String> {
    let key = key_of(kind, &id);
    let (fetches, install) = plan(&core, kind, &id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut downloads = core.downloads.lock().unwrap();
        if downloads.contains_key(&key) {
            return Ok(());
        }
        downloads.insert(key.clone(), cancel.clone());
    }
    std::thread::spawn(move || {
        let emit = |done: u64, total: Option<u64>, state: &'static str, message: Option<String>| {
            let _ = app.emit("download", DownloadEvent { key: key.clone(), done, total, state, message });
        };
        crate::diag::info(format!("download {key} started"));
        // Several archives show as one bar over their combined size.
        let combined: u64 = fetches.iter().map(|f| f.size).sum();
        let single = fetches.len() == 1;
        let mut offset = 0u64;
        let mut result = Ok(());
        for f in &fetches {
            result = local::download_verified(&f.url, f.sha256, &f.file, &cancel, &mut |d, t| {
                if single {
                    emit(d, t, "running", None)
                } else {
                    emit(offset + d, Some(combined), "running", None)
                }
            });
            if result.is_err() {
                break;
            }
            offset += f.size;
        }
        let unpacks = install.is_some();
        if result.is_ok() {
            if let Some(install) = install {
                result = install(&app);
            }
        }
        if result.is_err() && unpacks {
            // A runtime's archives are useless alone: an earlier one that did arrive goes too.
            for f in &fetches {
                let _ = std::fs::remove_file(&f.file);
            }
        }
        app.state::<Core>().downloads.lock().unwrap().remove(&key);
        match result {
            Ok(()) => {
                crate::diag::info(format!("download {key} installed"));
                emit(0, None, "done", None);
                let core = app.state::<Core>();
                if select {
                    let after = core.update_settings(|s| match kind {
                        Kind::Runtime => s.local.runtime = id.clone(),
                        Kind::Model => s.local.model = id.clone(),
                        Kind::LlmRuntime => s.cleanup.local_runtime = id.clone(),
                        Kind::LlmModel => s.cleanup.local_model = id.clone(),
                        Kind::Tunnel => {}
                    });
                    let _ = app.emit("settings-changed", after);
                }
                // If this completed what the settings ask for, load it right away.
                let s = core.settings();
                if kind == Kind::Tunnel {
                    crate::tunnel::sync(&app);
                } else if kind.is_llm() {
                    if crate::llm::wants_local(&s) {
                        let _ = core.llm.ensure(&app);
                    }
                } else if s.engine == EngineKind::Local && s.local.keep_loaded {
                    let _ = core.whisper.ensure(&app);
                }
            }
            Err(DownloadError::Cancelled) => emit(0, None, "cancelled", None),
            Err(e) => {
                crate::diag::error(format!("download {key} failed: {e}"));
                emit(0, None, "failed", Some(e.to_string()));
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_download(core: State<Core>, kind: Kind, id: String) {
    if let Some(flag) = core.downloads.lock().unwrap().get(&key_of(kind, &id)) {
        flag.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
pub async fn remove_download(app: AppHandle, kind: Kind, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let core = app.state::<Core>();
        let layout = &core.paths.layout;
        match kind {
            Kind::Runtime => core.whisper.stop_runtime(&app, &id),
            Kind::Model => core.whisper.stop_model(&app, &id),
            Kind::LlmRuntime => core.llm.stop_runtime(&app, &id),
            Kind::LlmModel => core.llm.stop_model(&app, &id),
            Kind::Tunnel => core.tunnel.stop(),
        }
        match kind {
            Kind::Runtime => std::fs::remove_dir_all(layout.runtime_dir(local::runtime(&id).ok_or("Unknown runtime.")?)),
            Kind::Model => std::fs::remove_file(layout.model_path(local::model(&id).ok_or("Unknown model.")?)),
            Kind::LlmRuntime => std::fs::remove_dir_all(layout.llm_runtime_dir(local::llm_runtime(&id).ok_or("Unknown runtime.")?)),
            Kind::LlmModel => std::fs::remove_file(layout.llm_model_path(local::llm_model(&id).ok_or("Unknown model.")?)),
            Kind::Tunnel => std::fs::remove_file(layout.cloudflared_exe()),
        }
        .map_err(|e| format!("Could not delete: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Loads the local model now instead of on the first dictation.
#[tauri::command]
pub fn start_local(app: AppHandle) {
    std::thread::spawn(move || {
        let core = app.state::<Core>();
        let _ = core.whisper.ensure(&app);
    });
}

#[tauri::command]
pub fn bubble_pointer(app: AppHandle, core: State<Core>, kind: String) {
    let now = core.now_ms();
    let recording = core.dictation.is_recording();
    let action = {
        let mut g = core.pointer_gesture.lock().unwrap();
        match kind.as_str() {
            "press" => g.press(now, recording),
            "release" => g.release(now, recording),
            "drag" => match g.drag(now, recording) {
                Some(a) => {
                    bubble::start_drag(&app);
                    a
                }
                // Talking for a while already: the mouse only slipped.
                None => return,
            },
            _ => return,
        }
    };
    crate::hotkeys::apply(&app, action);
}

#[tauri::command]
pub fn bubble_config(core: State<Core>) -> BubbleConfig {
    bubble::config(&core)
}

/// Async: creating a window inside a synchronous command deadlocks WebView2 on Windows.
#[tauri::command]
pub async fn open_settings(app: AppHandle, page: Option<String>) {
    bubble::open_settings(&app, page.as_deref());
}

/// For the settings window's test button.
#[tauri::command]
pub fn toggle_dictation(app: AppHandle) {
    crate::dictation::toggle(&app);
}

#[tauri::command]
pub fn get_log() -> Vec<crate::diag::Entry> {
    crate::diag::entries()
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<crate::updates::UpdateInfo>, String> {
    crate::updates::check(&app).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<bool, String> {
    crate::updates::install(&app, false).await
}

/// Opens a link in the browser - only https, only to the providers' key pages and the
/// project page.
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let allowed = providers::ALL.iter().any(|p| !p.key_url.is_empty() && p.key_url == url) || url.starts_with("https://github.com/EinFabo/amberpc");
    if !allowed {
        return Err("Link not allowed.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Whether the phone API runs, at which addresses (key included), and a QR code to scan.
#[tauri::command]
pub async fn phone_info(app: AppHandle) -> crate::phone::Info {
    crate::phone::info(&app.state::<Core>())
}

/// A new access key: the addresses the phone has stop working.
#[tauri::command]
pub async fn phone_new_key(app: AppHandle) -> Result<crate::phone::Info, String> {
    let core = app.state::<Core>();
    crate::phone::new_key(&core)?;
    crate::diag::info("phone API: new key");
    Ok(crate::phone::info(&core))
}

/// The Cloudflare tunnel's token - pasted alone or as the whole install command Cloudflare
/// shows; None removes it.
#[tauri::command]
pub fn set_tunnel_token(app: AppHandle, core: State<Core>, token: Option<String>) -> Result<(), String> {
    let token = match token.filter(|t| !t.trim().is_empty()) {
        Some(pasted) => Some(amber_core::api::tunnel_token(&pasted).ok_or(
            "That is not a tunnel token - copy the token (it starts with \"eyJ\") or the whole install command from Cloudflare.",
        )?),
        None => None,
    };
    core.set_key(crate::tunnel::TOKEN_ID, token)?;
    std::thread::spawn(move || crate::tunnel::sync(&app));
    Ok(())
}

/// Loads the cleanup model now instead of on the first dictation.
#[tauri::command]
pub fn start_llm(app: AppHandle) {
    app.state::<Core>().llm.warm(&app, true);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// The text after the fillers rule.
    fillers: String,
    /// After the model, if the smart cleanup is on and worked.
    smart: Option<String>,
    error: Option<String>,
    ms: u64,
}

/// The settings page's "Try it": what the cleanup makes of a typed text, in a chosen style.
#[tauri::command]
pub async fn cleanup_preview(app: AppHandle, text: String, style: amber_core::profiles::WritingStyle) -> Preview {
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<Core>().settings();
        let fillers = crate::llm::by_rule(&s, &text);
        if !s.cleanup.smart {
            return Preview { fillers, smart: None, error: None, ms: 0 };
        }
        let t0 = Instant::now();
        let ask = amber_core::cleanup::Ask { style, instruction: "", vocabulary: &s.vocabulary };
        let result = crate::llm::cleaner(&app, &s, true).and_then(|c| c.clean(&fillers, &ask).map_err(|e| e.message));
        let ms = t0.elapsed().as_millis() as u64;
        match result {
            Ok(smart) => Preview { fillers, smart: Some(smart), error: None, ms },
            Err(e) => Preview { fillers, smart: None, error: Some(e), ms },
        }
    })
    .await
    .unwrap_or(Preview { fillers: String::new(), smart: None, error: Some("Preview failed.".into()), ms: 0 })
}

#[derive(Serialize)]
pub struct Program {
    exe: String,
    title: String,
}

/// Programs with a window open right now, for adding a profile.
#[tauri::command]
pub async fn running_programs() -> Vec<Program> {
    tauri::async_runtime::spawn_blocking(|| crate::insert::running_programs().into_iter().map(|(exe, title)| Program { exe, title }).collect())
        .await
        .unwrap_or_default()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProvider {
    id: &'static str,
    name: &'static str,
    models: Vec<ModelInfo>,
    has_key: bool,
}

/// The chat models per provider for the cleanup page.
#[tauri::command]
pub fn chat_providers(core: State<Core>) -> Vec<ChatProvider> {
    providers::ALL
        .iter()
        .map(|p| ChatProvider {
            id: p.id,
            name: p.name,
            models: p.chat_models.iter().map(|m| ModelInfo { id: m.id, label: m.label, note: m.note }).collect(),
            has_key: core.has_key(p.id),
        })
        .collect()
}

#[tauri::command]
pub fn history_list(core: State<Core>) -> Vec<amber_core::history::Entry> {
    core.memory.history()
}

#[tauri::command]
pub fn history_delete(core: State<Core>, at: u64) {
    core.memory.forget(at);
}

#[tauri::command]
pub fn history_clear(core: State<Core>) {
    core.memory.clear_history();
}

#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    crate::insert::to_clipboard(&text)
}

#[tauri::command]
pub async fn recognize_again(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || crate::dictation::recognize_again(&app)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn stats_summary(core: State<Core>) -> amber_core::stats::Summary {
    let today = amber_core::stats::day_of(crate::memory::now_ms(), crate::memory::utc_offset_minutes());
    amber_core::stats::summary(&core.memory.ledger.all(), today, amber_core::stats::DEFAULT_TYPING_WPM, 14)
}

#[tauri::command]
pub fn stats_clear(core: State<Core>) {
    core.memory.ledger.clear();
}

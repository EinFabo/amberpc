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
        crate::sync_autostart(&app, after.autostart);
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
                let _ = core.whisper.ensure(&app2, &core.paths.layout, &after2.local);
            }
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalInfo {
    runtimes: Vec<LocalItem>,
    models: Vec<LocalItem>,
    has_nvidia: bool,
    recommended: (&'static str, &'static str),
    status: whisper::Status,
    downloads: Vec<String>,
    folder: String,
}

#[tauri::command]
pub fn local_info(core: State<Core>) -> LocalInfo {
    let layout = &core.paths.layout;
    let has_nvidia = has_nvidia();
    LocalInfo {
        runtimes: local::RUNTIMES
            .iter()
            .map(|r| LocalItem { id: r.id, label: r.label, note: r.note, size_mb: r.size_mb, installed: layout.runtime_installed(r), wants_gpu: r.id == "cuda" })
            .collect(),
        models: local::MODELS
            .iter()
            .map(|m| LocalItem { id: m.id, label: m.label, note: m.note, size_mb: m.size_mb, installed: layout.model_installed(m), wants_gpu: m.wants_gpu })
            .collect(),
        has_nvidia,
        recommended: local::recommend(has_nvidia),
        status: core.whisper.status(),
        downloads: core.downloads.lock().unwrap().keys().cloned().collect(),
        folder: layout.root.display().to_string(),
    }
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
}

fn key_of(kind: Kind, id: &str) -> String {
    format!("{}:{id}", if kind == Kind::Runtime { "runtime" } else { "model" })
}

/// Downloads a runtime or a model in the background; progress arrives as "download" events.
#[tauri::command]
pub fn download(app: AppHandle, core: State<Core>, kind: Kind, id: String) -> Result<(), String> {
    let key = key_of(kind, &id);
    let (url, sha, dest, staging) = match kind {
        Kind::Runtime => {
            let rt = local::runtime(&id).ok_or("Unknown runtime.")?;
            let zip = core.paths.layout.root.join("downloads").join(format!("whisper-{}-{}.zip", rt.id, local::WHISPER_CPP_BUILD));
            (rt.url.to_string(), rt.sha256, core.paths.layout.runtime_dir(rt), Some(zip))
        }
        Kind::Model => {
            let m = local::model(&id).ok_or("Unknown model.")?;
            (m.url(), m.sha256, core.paths.layout.model_path(m), None)
        }
    };
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
        let file = staging.clone().unwrap_or_else(|| dest.clone());
        let mut result = local::download_verified(&url, sha, &file, &cancel, &mut |d, t| emit(d, t, "running", None));
        if result.is_ok() {
            if let Some(zip) = &staging {
                // A runtime in use cannot be replaced while its server runs.
                app.state::<Core>().whisper.stop(&app);
                result = local::install_runtime(zip, &dest, sha);
                let _ = std::fs::remove_file(zip);
            }
        }
        app.state::<Core>().downloads.lock().unwrap().remove(&key);
        match result {
            Ok(()) => {
                crate::diag::info(format!("download {key} installed"));
                emit(0, None, "done", None);
                // If this completed what the settings ask for, load it right away.
                let core = app.state::<Core>();
                let s = core.settings();
                if s.engine == EngineKind::Local && s.local.keep_loaded {
                    let _ = core.whisper.ensure(&app, &core.paths.layout, &s.local);
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
        let s = core.settings();
        let in_use = match kind {
            Kind::Runtime => s.local.runtime == id,
            Kind::Model => s.local.model == id,
        };
        if in_use {
            core.whisper.stop(&app);
        }
        match kind {
            Kind::Runtime => std::fs::remove_dir_all(layout.runtime_dir(local::runtime(&id).ok_or("Unknown runtime.")?)),
            Kind::Model => std::fs::remove_file(layout.model_path(local::model(&id).ok_or("Unknown model.")?)),
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
        let _ = core.whisper.ensure(&app, &core.paths.layout, &core.settings().local);
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
            "drag" => {
                let a = g.drag(recording);
                bubble::start_drag(&app);
                a
            }
            _ => return,
        }
    };
    crate::hotkeys::apply(&app, action);
}

#[tauri::command]
pub fn bubble_config(core: State<Core>) -> BubbleConfig {
    bubble::config(&core)
}

#[tauri::command]
pub fn open_settings(app: AppHandle, page: Option<String>) {
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

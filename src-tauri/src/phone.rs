//! The phone API in the app: starts and stops `amber_core::api`'s server as the settings
//! say, hands it the engine the PC dictates with, and keeps its access key with the other
//! keys (DPAPI).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use amber_core::api::{self, Hooks, Server};
use amber_core::engine::ErrorKind;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::Core;

/// Stored next to the providers' API keys.
const KEY_ID: &str = "phone-api";

#[derive(Default)]
pub struct PhoneApi {
    running: Mutex<Option<(Server, u16)>>,
    error: Mutex<Option<String>>,
}

impl PhoneApi {
    /// The port Amber's own server listens on right now - the tunnel forwards only to it.
    pub fn running_port(&self) -> Option<u16> {
        self.running.lock().unwrap().as_ref().map(|(_, port)| *port)
    }
}

/// Starts, restarts or stops the server to match the settings. Reads the settings under the
/// lock, so of two quick changes the later one always wins. Stopping only closes the port
/// (answers in progress finish on their own threads), so this never waits long.
pub fn sync(app: &AppHandle) {
    let core = app.state::<Core>();
    let phone = &core.phone;
    let mut running = phone.running.lock().unwrap();
    let settings = core.settings().phone;
    if running.as_ref().is_some_and(|(_, port)| settings.enabled && *port == settings.port) {
        return;
    }
    if let Some((server, _)) = running.take() {
        server.stop();
        crate::diag::info("phone API stopped");
    }
    *phone.error.lock().unwrap() = None;
    if !settings.enabled {
        return;
    }
    key(&core);

    let (k, e, t, r, c, m) = (app.clone(), app.clone(), app.clone(), app.clone(), app.clone(), app.clone());
    let hooks = Hooks {
        service: "amber-pc",
        version: env!("CARGO_PKG_VERSION"),
        // Read per request, so a new key takes effect at once.
        keys: Arc::new(move || access_keys(&k.state::<Core>())),
        // The PC keeps one speech model loaded for its own dictations: whatever the phone names,
        // it gets that one.
        // The model the phone chose (already one its key may use), or the PC's own.
        engine: Arc::new(move |model| crate::dictation::transcriber_for(&e, &e.state::<Core>().settings(), model).map(|p| p.transcriber)),
        models: Arc::new(move || models(&m.state::<Core>())),
        // Like the PC's own dictations: fillers out, the model's tidy-up if it is on and
        // ready - never a wait for a model that is still loading.
        record: Arc::new(move |text, audio_ms, wait_ms| {
            let core = r.state::<Core>();
            core.memory.remember(&core.settings(), text, "phone", "phone API", true, audio_ms, wait_ms);
        }),
        tidy: Arc::new(move |text| {
            let settings = t.state::<Core>().settings();
            if settings.phone.cleanup {
                // The phone app has its own styles per app; here only the PC's general cleanup.
                // Without snippets: a phrase must not hand the PC's saved texts (an address, a
                // signature) to whoever holds the phone's address.
                crate::llm::tidy(&t, &settings, &text, None, false).0
            } else {
                text
            }
        }),
        // The phone's own cleanup, on the PC's cleanup model - local or a provider, as the
        // Cleanup page says. Waits for a local model to load; the phone keeps its text if that
        // takes longer than it waits.
        chat: Arc::new(move |request| {
            let settings = c.state::<Core>().settings();
            let cleaner = crate::llm::cleaner_for(&c, &settings, request["model"].as_str()).map_err(|e| (503, e))?;
            cleaner.forward(request).map_err(|e| {
                let status = match e.kind {
                    ErrorKind::Timeout => 504,
                    ErrorKind::RateLimited => 429,
                    ErrorKind::Unreachable | ErrorKind::Unavailable => 503,
                    _ => 502,
                };
                (status, e.message)
            })
        }),
        log: Arc::new(|m: String| crate::diag::info(m)),
    };
    match api::start(SocketAddr::from(([0, 0, 0, 0], settings.port)), hooks) {
        Ok(server) => {
            crate::diag::info(format!("phone API listening on port {}", settings.port));
            *running = Some((server, settings.port));
        }
        Err(e) => {
            let message = if e.kind() == std::io::ErrorKind::AddrInUse {
                format!("Port {} is already used by another program - pick another one.", settings.port)
            } else {
                format!("The phone API could not start: {e}")
            };
            crate::diag::warn(&message);
            *phone.error.lock().unwrap() = Some(message);
        }
    }
}

/// The first access key's secret (the one of 0.4.x), created on first use.
fn key(core: &Core) -> String {
    secret(core, KEY_ID)
}

/// A key's secret from the PC's secret store, created on first use.
fn secret(core: &Core, id: &str) -> String {
    if let Some(k) = core.key(id) {
        return k;
    }
    let k = api::new_key();
    if let Err(e) = core.set_key(id, Some(k.clone())) {
        crate::diag::error(format!("phone API key not saved: {e}"));
    }
    k
}

/// The ways in: the keys in the settings with their secrets.
fn access_keys(core: &Core) -> Vec<api::AccessKey> {
    core.settings()
        .phone
        .keys
        .iter()
        .map(|k| api::AccessKey { name: k.name.clone(), secret: secret(core, &k.id), models: k.models.clone(), per_day: k.per_day })
        .collect()
}

/// A new secret for the key [id] (the first if None): its old addresses stop working.
pub fn new_key(core: &Core, id: Option<&str>) -> Result<(), String> {
    let id = id.map(str::to_string).unwrap_or_else(|| core.settings().phone.keys.first().map_or(KEY_ID.to_string(), |k| k.id.clone()));
    core.set_key(&id, Some(api::new_key()))
}

/// Removes the key [id] and its secret - not the last one: nobody could get in.
pub fn remove_key(core: &Core, id: &str) -> Result<(), String> {
    if core.settings().phone.keys.len() <= 1 {
        return Err("That is the last key - add another first.".into());
    }
    core.update_settings(|s| s.phone.keys.retain(|k| k.id != id));
    core.set_key(id, None)
}

/// A key as the Phone page shows it: what it may, and its address.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyInfo {
    id: String,
    name: String,
    models: Option<Vec<String>>,
    per_day: Option<u32>,
    /// In the home network (the first address), and from anywhere when the tunnel is on.
    url: Option<String>,
    qr: Option<String>,
    public_url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    name: String,
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    running: bool,
    error: Option<String>,
    addresses: Vec<Address>,
    /// The first address as a QR code (SVG), to scan with the phone's camera.
    qr: Option<String>,
    /// From anywhere: cloudflared's state, whether it is downloaded and has its token, and the
    /// public address with its QR code.
    tunnel: crate::tunnel::Status,
    tunnel_downloaded: bool,
    tunnel_token: bool,
    public_url: Option<String>,
    public_qr: Option<String>,
    /// Every key with its address; the first is the one shown above.
    keys: Vec<KeyInfo>,
    /// What the phone may choose among - for picking a key's models.
    models: Vec<api::ModelInfo>,
}

pub fn info(core: &Core) -> Info {
    let settings = core.settings().phone;
    let running = core.phone.running.lock().unwrap().is_some();
    let error = core.phone.error.lock().unwrap().clone();
    let interfaces: Vec<(String, std::net::IpAddr)> =
        if running { if_addrs::get_if_addrs().unwrap_or_default().into_iter().map(|i| (i.name.clone(), i.ip())).collect() } else { Vec::new() };
    let home_ips = api::phone_addresses(&interfaces);
    let addresses: Vec<Address> = if running {
        let key = key(core);
        home_ips.iter().map(|(name, ip)| Address { name: name.clone(), url: api::phone_url(*ip, settings.port, &key) }).collect()
    } else {
        Vec::new()
    };
    let qr = addresses.first().and_then(|a| qr_svg(&a.url));
    let public_url = (settings.tunnel && !settings.public_host.is_empty()).then(|| api::public_url(&settings.public_host, &key(core)));
    let public_qr = public_url.as_deref().and_then(qr_svg);
    let keys = settings
        .keys
        .iter()
        .map(|k| {
            let secret = secret(core, &k.id);
            let url = home_ips.first().map(|(_, ip)| api::phone_url(*ip, settings.port, &secret));
            KeyInfo {
                id: k.id.clone(),
                name: k.name.clone(),
                models: k.models.clone(),
                per_day: k.per_day,
                qr: url.as_deref().and_then(qr_svg),
                url,
                public_url: (settings.tunnel && !settings.public_host.is_empty()).then(|| api::public_url(&settings.public_host, &secret)),
            }
        })
        .collect();
    Info {
        running,
        error,
        addresses,
        qr,
        tunnel: core.tunnel.status(),
        tunnel_downloaded: core.paths.layout.cloudflared_installed(),
        tunnel_token: core.has_key(crate::tunnel::TOKEN_ID),
        public_url,
        public_qr,
        keys,
        models: models(core),
    }
}

/// What the phone may choose among: every downloaded model and the cloud models set in the
/// app (when they can run), the PC's own as the default.
fn models(core: &Core) -> Vec<api::ModelInfo> {
    use amber_core::local;
    use amber_core::providers;
    let s = core.settings();
    let layout = &core.paths.layout;
    let rt = local::runtime(&s.local.runtime).filter(|rt| layout.runtime_installed(rt));
    let llm_rt = local::llm_runtime(&s.cleanup.local_runtime).filter(|rt| layout.llm_runtime_installed(rt));
    let ready = |provider: &str| providers::by_id(provider).is_some_and(|p| !p.needs_key || core.has_key(p.id));
    let installed = amber_core::offer::Installed {
        speech: if rt.is_some() { local::MODELS.iter().filter(|m| layout.model_installed(m)).map(|m| m.id.to_string()).collect() } else { Vec::new() },
        speech_runtime: rt.map_or("this PC", |r| r.label).to_string(),
        llms: if llm_rt.is_some() { local::LLM_MODELS.iter().filter(|m| layout.llm_model_installed(m)).map(|m| m.id.to_string()).collect() } else { Vec::new() },
        llm_runtime: llm_rt.map_or("this PC", |r| r.label).to_string(),
        cloud_speech: ready(&s.server.provider),
        cloud_cleanup: ready(&s.cleanup.provider),
    };
    amber_core::offer::pc_models(&s, &installed)
}

fn qr_svg(text: &str) -> Option<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(code.render::<svg::Color>().min_dimensions(200, 200).dark_color(svg::Color("#0a0a0a")).light_color(svg::Color("#ffffff")).build())
}

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
        key: Arc::new(move || k.state::<Core>().key(KEY_ID).unwrap_or_default()),
        // The PC keeps one speech model loaded for its own dictations: whatever the phone names,
        // it gets that one.
        engine: Arc::new(move |_model| crate::dictation::transcriber(&e, &e.state::<Core>().settings()).map(|p| p.transcriber)),
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
            let cleaner = crate::llm::cleaner(&c, &settings, true).map_err(|e| (503, e))?;
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

/// The access key, created on first use.
fn key(core: &Core) -> String {
    if let Some(k) = core.key(KEY_ID) {
        return k;
    }
    let k = api::new_key();
    if let Err(e) = core.set_key(KEY_ID, Some(k.clone())) {
        crate::diag::error(format!("phone API key not saved: {e}"));
    }
    k
}

pub fn new_key(core: &Core) -> Result<(), String> {
    core.set_key(KEY_ID, Some(api::new_key()))
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
}

pub fn info(core: &Core) -> Info {
    let settings = core.settings().phone;
    let running = core.phone.running.lock().unwrap().is_some();
    let error = core.phone.error.lock().unwrap().clone();
    let addresses: Vec<Address> = if running {
        let key = key(core);
        let interfaces: Vec<(String, std::net::IpAddr)> =
            if_addrs::get_if_addrs().unwrap_or_default().into_iter().map(|i| (i.name.clone(), i.ip())).collect();
        api::phone_addresses(&interfaces)
            .into_iter()
            .map(|(name, ip)| Address { name, url: api::phone_url(ip, settings.port, &key) })
            .collect()
    } else {
        Vec::new()
    };
    let qr = addresses.first().and_then(|a| qr_svg(&a.url));
    let public_url = (settings.tunnel && !settings.public_host.is_empty()).then(|| api::public_url(&settings.public_host, &key(core)));
    let public_qr = public_url.as_deref().and_then(qr_svg);
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
    }
}

/// What the phone's choice of models shows: the speech model and the cleanup model this PC
/// uses - the cleanup only once it can run.
fn models(core: &Core) -> Vec<api::ModelInfo> {
    use amber_core::local;
    use amber_core::providers;
    use amber_core::settings::EngineKind;
    let s = core.settings();
    let layout = &core.paths.layout;
    let mut out = Vec::new();
    let speech = match s.engine {
        EngineKind::Local => local::model(&s.local.model).map(|m| {
            let on = local::runtime(&s.local.runtime).map_or("this PC", |r| r.label);
            (m.id.to_string(), m.label.to_string(), format!("on this PC · {on}"))
        }),
        EngineKind::Server => providers::by_id(&s.server.provider).map(|p| (s.server.model.clone(), s.server.model.clone(), format!("through this PC · {}", p.name))),
    };
    if let Some((id, label, note)) = speech {
        out.push(api::ModelInfo { id, kind: api::ModelKind::Speech, label, note, default: true });
    }
    let c = &s.cleanup;
    let cleanup = match c.engine {
        EngineKind::Local => match (local::llm_runtime(&c.local_runtime), local::llm_model(&c.local_model)) {
            (Some(rt), Some(m)) if layout.llm_runtime_installed(rt) && layout.llm_model_installed(m) => {
                Some((m.id.to_string(), m.label.to_string(), format!("on this PC · {}", rt.label)))
            }
            _ => None,
        },
        EngineKind::Server => providers::by_id(&c.provider)
            .filter(|p| !c.model.trim().is_empty() && (!p.needs_key || core.has_key(p.id)))
            .map(|p| {
                let label = p.chat_model(&c.model).map_or(c.model.clone(), |m| m.label.to_string());
                (c.model.clone(), label, format!("through this PC · {}", p.name))
            }),
    };
    if let Some((id, label, note)) = cleanup {
        out.push(api::ModelInfo { id, kind: api::ModelKind::Cleanup, label, note, default: true });
    }
    out
}

fn qr_svg(text: &str) -> Option<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(code.render::<svg::Color>().min_dimensions(200, 200).dark_color(svg::Color("#0a0a0a")).light_color(svg::Color("#ffffff")).build())
}

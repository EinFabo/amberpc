//! The phone API in the app: starts and stops `amber_core::api`'s server as the settings
//! say, hands it the engine the PC dictates with, and keeps its access key with the other
//! keys (DPAPI).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use amber_core::api::{self, Hooks, Server};
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

    let (k, e, t) = (app.clone(), app.clone(), app.clone());
    let hooks = Hooks {
        // Read per request, so a new key takes effect at once.
        key: Arc::new(move || k.state::<Core>().key(KEY_ID).unwrap_or_default()),
        engine: Arc::new(move || crate::dictation::transcriber(&e, &e.state::<Core>().settings())),
        // Like the PC's own dictations: fillers out, the model's tidy-up if it is on and
        // ready - never a wait for a model that is still loading.
        tidy: Arc::new(move |text| {
            let settings = t.state::<Core>().settings();
            if settings.phone.cleanup {
                crate::llm::tidy(&t, &settings, &text).0
            } else {
                text
            }
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
    Info { running, error, addresses, qr }
}

fn qr_svg(text: &str) -> Option<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(code.render::<svg::Color>().min_dimensions(200, 200).dark_color(svg::Color("#0a0a0a")).light_color(svg::Color("#ffffff")).build())
}

//! The Stream Deck plugin's way in: `amber_core::control` on 127.0.0.1 with a fresh token
//! every start, handed over in `streamdeck.json` next to the settings. A key held down
//! dictates until it comes up; a short tap starts, the next one stops.

use std::sync::{Arc, Mutex};

use amber_core::control::{self, Hooks, HANDSHAKE_FILE};
use amber_core::http::Server;
use tauri::{AppHandle, Manager};

use crate::state::Core;

#[derive(Default)]
pub struct Control {
    server: Mutex<Option<Server>>,
}

pub fn start(app: &AppHandle) {
    let core = app.state::<Core>();
    let token = amber_core::api::new_key();
    let (p, r, t, s) = (app.clone(), app.clone(), app.clone(), app.clone());
    let hooks = Hooks {
        press: Arc::new(move || key(&p, true)),
        release: Arc::new(move || key(&r, false)),
        toggle: Arc::new(move || crate::dictation::toggle(&t)),
        status: Arc::new(move || {
            serde_json::json!({ "phase": s.state::<Core>().dictation.phase(), "version": env!("CARGO_PKG_VERSION") })
        }),
    };
    match control::start(token.clone(), hooks) {
        Ok(server) => {
            let file = core.paths.settings.with_file_name(HANDSHAKE_FILE);
            let text = control::handshake(server.addr.port(), &token).to_string();
            if let Err(e) = std::fs::write(&file, text) {
                crate::diag::warn(format!("Stream Deck handshake not written: {e}"));
            }
            crate::diag::info(format!("control port for the Stream Deck on 127.0.0.1:{}", server.addr.port()));
            *core.control.server.lock().unwrap() = Some(server);
        }
        Err(e) => crate::diag::warn(format!("control port not started: {e}")),
    }
}

fn key(app: &AppHandle, down: bool) {
    let core = app.state::<Core>();
    let now = core.now_ms();
    let recording = core.dictation.is_recording();
    let action = {
        let mut g = core.deck_gesture.lock().unwrap();
        if down {
            g.press(now, recording)
        } else {
            g.release(now, recording)
        }
    };
    crate::hotkeys::apply(app, action);
}

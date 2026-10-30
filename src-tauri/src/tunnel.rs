//! The phone API from anywhere: a Cloudflare Tunnel, run by cloudflared with the user's token.
//! The user creates the tunnel in Cloudflare - a public hostname on their own domain, pointing
//! at http://localhost:<port> - and Amber keeps cloudflared running while the phone API and
//! the switch are on. The token stays with the DPAPI-protected secrets and reaches cloudflared
//! as an environment variable, never on its command line; cloudflared ends with Amber (the
//! job object in `process`).
//!
//! Requests come through cloudflared from 127.0.0.1. What keeps strangers out is the access
//! key in the address: 20 random characters, compared in constant time - and HTTPS all the
//! way to Cloudflare.

use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::process::{self, Output};
use crate::state::Core;

/// Where the token lives among the secrets.
pub const TOKEN_ID: &str = "cloudflare-tunnel";
/// After cloudflared ended on its own: the next try.
const RETRY: Duration = Duration::from_secs(30);

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// "off", "setup" (something is missing), "starting", "connected", "failed"
    pub state: &'static str,
    pub message: Option<String>,
}

#[derive(Default)]
pub struct Tunnel {
    child: Mutex<Option<Child>>,
    output: Output,
    status: Mutex<Option<Status>>,
    /// Bumped with every start and stop: the watcher of an older run lets go.
    run: AtomicU64,
}

impl Tunnel {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone().unwrap_or(Status { state: "off", message: None })
    }

    fn set(&self, state: &'static str, message: Option<String>) {
        *self.status.lock().unwrap() = Some(Status { state, message });
    }

    /// Ends cloudflared, e.g. before its file is deleted.
    pub fn stop(&self) {
        self.run.fetch_add(1, Ordering::SeqCst);
        if let Some(mut c) = self.child.lock().unwrap().take() {
            process::stop(&mut c);
            crate::diag::info("tunnel stopped");
        }
        self.set("off", None);
    }
}

/// Starts or stops cloudflared to match the settings, the token and the download.
pub fn sync(app: &AppHandle) {
    let core = app.state::<Core>();
    let t = &core.tunnel;
    let s = core.settings().phone;
    let token = core.key(TOKEN_ID);
    let exe = core.paths.layout.cloudflared_exe();
    let wanted = s.enabled && s.tunnel;
    let ready = wanted && token.is_some() && exe.is_file();

    let mut child = t.child.lock().unwrap();
    if ready && child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))) {
        return;
    }
    if let Some(mut c) = child.take() {
        process::stop(&mut c);
        crate::diag::info("tunnel stopped");
    }
    let run = t.run.fetch_add(1, Ordering::SeqCst) + 1;
    if !ready {
        let missing = if !wanted {
            None
        } else if !exe.is_file() {
            Some("cloudflared is not downloaded yet.")
        } else {
            Some("No tunnel token yet.")
        };
        t.set(if missing.is_some() { "setup" } else { "off" }, missing.map(str::to_string));
        return;
    }
    // --url: where a tunnel created on the command line sends its requests. A tunnel set up in
    // Cloudflare's dashboard brings its own routing from there, which takes precedence.
    let origin = format!("http://localhost:{}", s.port);
    let mut cmd = Command::new(&exe);
    cmd.args(["tunnel", "--no-autoupdate", "--url", &origin, "run"]).env("TUNNEL_TOKEN", token.unwrap_or_default());
    match process::spawn(cmd, &t.output) {
        Ok(c) => {
            *child = Some(c);
            t.set("starting", None);
            crate::diag::info("tunnel starting");
            watch(app, run);
        }
        Err(e) => t.set("failed", Some(format!("cloudflared could not start: {e}"))),
    }
}

/// Follows one run: connected once cloudflared registered with Cloudflare; if it ends, the
/// reason, and another try a little later.
fn watch(app: &AppHandle, run: u64) {
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        let core = app.state::<Core>();
        let t = &core.tunnel;
        if t.run.load(Ordering::SeqCst) != run {
            return;
        }
        let ended = t.child.lock().unwrap().as_mut().is_none_or(|c| !matches!(c.try_wait(), Ok(None)));
        if ended {
            let last = process::last_lines(&t.output, 3);
            crate::diag::warn(format!("tunnel ended: {last}"));
            let reason = if last.contains("token is not valid") || last.contains("Invalid tunnel secret") {
                "The tunnel token is not valid - copy it again from Cloudflare.".to_string()
            } else {
                format!("cloudflared ended: {last}")
            };
            t.set("failed", Some(reason));
            std::thread::sleep(RETRY);
            if t.run.load(Ordering::SeqCst) == run {
                sync(&app);
            }
            return;
        }
        if t.status().state == "starting" && t.output.lock().unwrap().iter().any(|l| l.contains("Registered tunnel connection")) {
            t.set("connected", None);
            crate::diag::info("tunnel connected");
        }
    });
}

//! The smart cleanup's local model: llama.cpp's `llama-server.exe` with the chosen model on
//! a free port on 127.0.0.1, managed like the whisper server (see `process`). And the
//! choice between it and a server with an API key.

use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use amber_core::cleanup::{self, ChatCleaner};
use amber_core::local;
use amber_core::providers;
use amber_core::settings::{EngineKind, Settings};
use tauri::{AppHandle, Emitter, Manager};

use crate::process::{self, Output};
use crate::state::Core;
use crate::whisper::Status;

#[derive(Clone, PartialEq, Eq, Debug)]
struct Key {
    runtime: String,
    model: String,
}

struct Running {
    child: Child,
    port: u16,
    key: Key,
}

#[derive(Default)]
pub struct Llm {
    running: Mutex<Option<Running>>,
    output: Output,
    status: Mutex<Option<Status>>,
}

/// Loading 2.4 GB from a slow disk onto a GPU, or into RAM for the CPU.
const START_TIMEOUT: Duration = Duration::from_secs(180);

/// Room for a dictation plus the prompt; one request at a time keeps memory low.
const CONTEXT: &str = "4096";

impl Llm {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone().unwrap_or(Status::of("stopped", None))
    }

    fn set_status(&self, app: &AppHandle, status: Status) {
        *self.status.lock().unwrap() = Some(status.clone());
        let _ = app.emit("llm-status", status);
    }

    /// The address of a llama-server with the configured model, started first if needed.
    /// Like the whisper server: settings are read under the lock, and a running server is
    /// only replaced once its successor is fully downloaded.
    pub fn ensure(&self, app: &AppHandle) -> Result<String, String> {
        let mut running = self.running.lock().unwrap();
        let core = app.state::<Core>();
        let layout = &core.paths.layout;
        let c = core.settings().cleanup;
        let key = Key { runtime: c.local_runtime.clone(), model: c.local_model.clone() };
        let rt = local::llm_runtime(&key.runtime).ok_or("Unknown runtime.")?;
        let model = local::llm_model(&key.model).ok_or("Unknown model.")?;
        let missing = if !layout.llm_runtime_installed(rt) {
            Some(format!("The cleanup engine ({}) is not installed yet - download it under Cleanup.", rt.label))
        } else if !layout.llm_model_installed(model) {
            Some(format!("The model {} is not downloaded yet - get it under Cleanup.", model.label))
        } else {
            None
        };

        if let Some(r) = running.as_mut() {
            let alive = matches!(r.child.try_wait(), Ok(None));
            if alive && (r.key == key || missing.is_some()) {
                return Ok(format!("http://127.0.0.1:{}", r.port));
            }
            if !alive {
                crate::diag::warn(format!("cleanup model had exited: {}", process::last_lines(&self.output, 6)));
            }
            process::stop(&mut r.child);
            *running = None;
        }
        if let Some(msg) = missing {
            self.set_status(app, Status::of("failed", Some(msg.clone())));
            return Err(msg);
        }

        self.set_status(app, Status::of("starting", Some(format!("Loading {}…", model.label))));
        let started = Instant::now();
        match self.spawn(layout, rt, model, &key) {
            Ok(r) => {
                let url = format!("http://127.0.0.1:{}", r.port);
                crate::diag::info(format!("cleanup model ready: {} on {} in {} ms", model.label, rt.label, started.elapsed().as_millis()));
                *running = Some(r);
                self.set_status(app, Status::of("ready", Some(format!("{} on {}", model.label, rt.label))));
                Ok(url)
            }
            Err(e) => {
                crate::diag::error(format!("cleanup model failed to start: {e}"));
                self.set_status(app, Status::of("failed", Some(e.clone())));
                Err(e)
            }
        }
    }

    fn spawn(&self, layout: &local::Layout, rt: &local::LlmRuntime, model: &local::LlmModel, key: &Key) -> Result<Running, String> {
        let port = process::free_port().map_err(|e| format!("No free port: {e}"))?;
        let gpu_layers = if rt.id == "cuda" { "999" } else { "0" };
        let mut cmd = Command::new(layout.llm_server_exe(rt));
        // Run in the models folder and pass only the file name: the path of a user folder
        // with unusual characters could arrive mangled.
        let model_path = layout.llm_model_path(model);
        cmd.current_dir(model_path.parent().unwrap_or(&layout.root))
            .args(["--model", model.file, "--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["--ctx-size", CONTEXT, "--parallel", "1", "--n-gpu-layers", gpu_layers])
            .args(["--threads", &process::default_threads().to_string()]);
        let mut child = process::spawn(cmd, &self.output).map_err(|e| format!("The cleanup engine could not be started: {e}"))?;

        // llama-server opens its port at once and answers /health with 503 until the model
        // has loaded.
        let base = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("The cleanup engine stopped while loading ({status}). {}", self.explain()));
            }
            if cleanup::local_server_ready(&base) {
                return Ok(Running { child, port, key: key.clone() });
            }
            if Instant::now() > deadline {
                process::stop(&mut child);
                return Err("The cleanup engine took too long to load its model.".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn explain(&self) -> String {
        let out = process::last_lines(&self.output, 6);
        let lower = out.to_lowercase();
        if lower.contains("out of memory") || lower.contains("failed to allocate") || lower.contains("cudamalloc") {
            format!("Not enough graphics memory - try the CPU, or a smaller speech model. ({out})")
        } else {
            out
        }
    }

    pub fn stop(&self, app: &AppHandle) {
        if let Some(mut r) = self.running.lock().unwrap().take() {
            process::stop(&mut r.child);
            crate::diag::info("cleanup model stopped");
        }
        self.set_status(app, Status::of("stopped", None));
    }

    pub fn stop_runtime(&self, app: &AppHandle, runtime: &str) {
        let uses = self.running.lock().unwrap().as_ref().is_some_and(|r| r.key.runtime == runtime);
        if uses {
            self.stop(app);
        }
    }

    pub fn stop_model(&self, app: &AppHandle, model: &str) {
        let uses = self.running.lock().unwrap().as_ref().is_some_and(|r| r.key.model == model);
        if uses {
            self.stop(app);
        }
    }

}

/// Whether the smart cleanup wants the local model loaded.
pub fn wants_local(s: &Settings) -> bool {
    s.cleanup.smart && s.cleanup.engine == EngineKind::Local
}

/// The client for the smart cleanup the settings ask for - or why not.
pub fn cleaner(app: &AppHandle, s: &Settings) -> Result<ChatCleaner, String> {
    let c = &s.cleanup;
    match c.engine {
        EngineKind::Local => {
            let url = app.state::<Core>().llm.ensure(app)?;
            // llama-server serves the one model it loaded, whatever the request names.
            Ok(ChatCleaner::new(&url, "", c.local_model.as_str(), None))
        }
        EngineKind::Server => {
            let provider = providers::by_id(&c.provider).ok_or("Unknown cleanup provider.")?;
            // An own server for the cleanup defaults to the speech engine's address.
            let custom = if c.base_url.trim().is_empty() { &s.server.base_url } else { &c.base_url };
            let base = providers::effective_base_url(provider, custom);
            providers::check_base_url(&base)?;
            let key = app.state::<Core>().key(provider.id).unwrap_or_default();
            if provider.needs_key && key.is_empty() {
                return Err(format!("No API key for {} - add it on the Engine page.", provider.name));
            }
            if c.model.trim().is_empty() {
                return Err("No chat model chosen for the cleanup.".into());
            }
            let extra = provider.chat_model(&c.model).and_then(|m| m.extra).and_then(|e| serde_json::from_str(e).ok());
            Ok(ChatCleaner::new(&base, &key, &c.model, extra))
        }
    }
}

/// What the dictation gets: the recognized text with fillers stripped and, if switched on,
/// tidied by the model. A failing model leaves the text as it was - never lost, never stuck.
pub fn tidy(app: &AppHandle, s: &Settings, recognized: &str) -> (String, Option<String>) {
    let text = if s.cleanup.fillers { amber_core::text::strip_fillers(recognized) } else { recognized.to_string() };
    if !s.cleanup.smart || text.is_empty() {
        return (text, None);
    }
    let started = Instant::now();
    match cleaner(app, s).and_then(|c| c.clean(&text).map_err(|e| format!("{} ({})", e.message, e.detail))) {
        Ok(cleaned) => {
            crate::diag::info(format!("cleanup: {} -> {} chars in {} ms", text.chars().count(), cleaned.chars().count(), started.elapsed().as_millis()));
            (cleaned, None)
        }
        Err(e) => {
            crate::diag::warn(format!("cleanup skipped: {e}"));
            (text, Some(e))
        }
    }
}

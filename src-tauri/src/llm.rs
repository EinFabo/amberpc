//! The smart cleanup's local model: llama.cpp's `llama-server.exe` with the chosen model on
//! a free port on 127.0.0.1, managed like the whisper server (see `process`). And the
//! choice between it and a server with an API key.

use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use amber_core::cleanup::{Ask, ChatCleaner};
use amber_core::profiles::{self, CleanupChoice, Profile};
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
    /// Its `--api-key`: without one, any web page could use the model by scanning loopback.
    api_key: String,
}

impl Running {
    fn endpoint(&self) -> Endpoint {
        Endpoint { url: format!("http://127.0.0.1:{}", self.port), key: self.api_key.clone() }
    }
}

/// Where the running model answers, and the key it wants.
pub struct Endpoint {
    pub url: String,
    pub key: String,
}

#[derive(Default)]
pub struct Llm {
    running: Mutex<Option<Running>>,
    output: Output,
    status: Mutex<Option<Status>>,
    /// A background load is under way.
    loading: AtomicBool,
    /// When the last start failed - a model that does not fit is not retried at every
    /// dictation.
    failed_at: Mutex<Option<Instant>>,
}

/// How long a failed start keeps background loads from trying again.
const RETRY_AFTER: Duration = Duration::from_secs(180);

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

    /// The running server, if it is the configured one and done loading - never waits. If
    /// not, a load starts in the background (unless the last one failed a moment ago).
    pub fn ready(&self, app: &AppHandle) -> Option<Endpoint> {
        let c = app.state::<Core>().settings().cleanup;
        if let Ok(mut running) = self.running.try_lock() {
            if let Some(r) = running.as_mut() {
                if matches!(r.child.try_wait(), Ok(None)) && r.key.runtime == c.local_runtime && r.key.model == c.local_model {
                    return Some(r.endpoint());
                }
            }
        }
        self.warm(app, false);
        None
    }

    /// Loads in the background. [force]: also right after a failure - the user asked.
    pub fn warm(&self, app: &AppHandle, force: bool) {
        let recently_failed = self.failed_at.lock().unwrap().is_some_and(|t| t.elapsed() < RETRY_AFTER);
        if (recently_failed && !force) || self.loading.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || {
            let llm = &app.state::<Core>().llm;
            let _ = llm.ensure(&app);
            llm.loading.store(false, Ordering::SeqCst);
        });
    }

    /// A llama-server with the configured model, started first if needed - or stopped, if
    /// the settings no longer want it. Blocks while the model loads. Like the whisper server:
    /// settings are read under the lock, and a running server is only replaced once its
    /// successor is fully downloaded.
    pub fn ensure(&self, app: &AppHandle) -> Result<Endpoint, String> {
        let mut running = self.running.lock().unwrap();
        let core = app.state::<Core>();
        let layout = &core.paths.layout;
        let settings = core.settings();
        if !wants_local(&settings) {
            // Switched off (or to a server) while this waited for the lock.
            if let Some(mut r) = running.take() {
                process::stop(&mut r.child);
                crate::diag::info("cleanup model stopped");
            }
            self.set_status(app, Status::of("stopped", None));
            return Err("The smart cleanup on this PC is off.".into());
        }
        let c = settings.cleanup;
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
                return Ok(r.endpoint());
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
                let endpoint = r.endpoint();
                crate::diag::info(format!("cleanup model ready: {} on {} in {} ms", model.label, rt.label, started.elapsed().as_millis()));
                *running = Some(r);
                *self.failed_at.lock().unwrap() = None;
                self.set_status(app, Status::of("ready", Some(format!("{} on {}", model.label, rt.label))));
                Ok(endpoint)
            }
            Err(e) => {
                crate::diag::error(format!("cleanup model failed to start: {e}"));
                *self.failed_at.lock().unwrap() = Some(Instant::now());
                self.set_status(app, Status::of("failed", Some(e.clone())));
                Err(e)
            }
        }
    }

    fn spawn(&self, layout: &local::Layout, rt: &local::LlmRuntime, model: &local::LlmModel, key: &Key) -> Result<Running, String> {
        let port = process::free_port().map_err(|e| format!("No free port: {e}"))?;
        let api_key = amber_core::api::new_key();
        let mut cmd = Command::new(layout.llm_server_exe(rt));
        // Run in the models folder and pass only the file name: the path of a user folder
        // with unusual characters could arrive mangled. No --n-gpu-layers: left unset,
        // llama.cpp fits as many layers as the graphics memory next to Whisper allows.
        let model_path = layout.llm_model_path(model);
        cmd.current_dir(model_path.parent().unwrap_or(&layout.root))
            .args(["--model", model.file, "--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["--ctx-size", CONTEXT, "--parallel", "1", "--api-key", &api_key])
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
            if amber_core::engine::local_server_ready(&base) {
                return Ok(Running { child, port, key: key.clone(), api_key });
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

/// Whether the smart cleanup wants the local model loaded: switched on, or always on for
/// some program.
pub fn wants_local(s: &Settings) -> bool {
    let anywhere = s.cleanup.smart || s.profiles.iter().any(|p| p.cleanup == CleanupChoice::Always);
    anywhere && s.cleanup.engine == EngineKind::Local
}

/// The client for the smart cleanup the settings ask for - or why not. [wait]: for the local
/// model, wait while it loads (the settings page's preview) instead of skipping (a dictation).
pub fn cleaner(app: &AppHandle, s: &Settings, wait: bool) -> Result<ChatCleaner, String> {
    let c = &s.cleanup;
    match c.engine {
        EngineKind::Local => {
            let llm = &app.state::<Core>().llm;
            let endpoint = if wait {
                llm.ensure(app)?
            } else {
                llm.ready(app).ok_or("the cleanup model is still loading")?
            };
            // llama-server serves the one model it loaded, whatever the request names.
            Ok(ChatCleaner::new(&endpoint.url, &endpoint.key, c.local_model.as_str(), None))
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

/// What the dictation gets, in this order: spoken punctuation ("neue Zeile", "Komma"),
/// fillers out, German commas, and - when the smart cleanup runs for [profile]'s program - the
/// model's tidy-up in that program's style. A failing model leaves the text as it was: never
/// lost, never stuck. Returns why the model was skipped, if it was.
pub fn tidy(app: &AppHandle, s: &Settings, recognized: &str, profile: Option<&Profile>) -> (String, Option<String>) {
    let text = by_rule(s, recognized);
    if !profiles::smart_cleanup(s.cleanup.smart, profile) || text.is_empty() {
        return (text, None);
    }
    let started = Instant::now();
    let ask = Ask {
        style: profile.map(|p| p.style).unwrap_or_default(),
        instruction: profile.map(|p| p.instruction.as_str()).unwrap_or(""),
        vocabulary: &s.vocabulary,
    };
    // Never waits for a model that is still loading: the text goes in as recognized.
    match cleaner(app, s, false).and_then(|c| c.clean(&text, &ask).map_err(|e| format!("{} ({})", e.message, e.detail))) {
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

/// The steps that are rules: instant, local, nothing invented.
pub fn by_rule(s: &Settings, recognized: &str) -> String {
    let mut text = if s.cleanup.spoken_punctuation { amber_core::spoken::apply(recognized) } else { recognized.to_string() };
    if s.cleanup.fillers {
        text = amber_core::text::strip_fillers(&text);
    }
    let language = (s.language != "auto").then_some(s.language.as_str());
    if s.cleanup.german_commas && amber_core::commas::is_german(language, &text) {
        text = amber_core::commas::apply(&text);
    }
    text
}

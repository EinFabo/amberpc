//! The local engine: whisper.cpp's `whisper-server.exe`, started with the chosen model on a
//! free port on 127.0.0.1. It keeps the model in memory between dictations; see `process`
//! for how it is tied to the app.

use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::process::{self, Output};

use amber_core::local::{self, Layout};
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// "stopped", "starting", "ready", "failed"
    pub state: &'static str,
    pub message: Option<String>,
}

impl Status {
    pub fn of(state: &'static str, message: Option<String>) -> Self {
        Status { state, message }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Key {
    runtime: String,
    model: String,
    threads: u32,
}

struct Running {
    child: Child,
    port: u16,
    key: Key,
}

#[derive(Default)]
pub struct Whisper {
    running: Mutex<Option<Running>>,
    /// The last lines the server printed - the reason when it dies.
    output: Output,
    status: Mutex<Option<Status>>,
}

/// How long a model may take to load - large-v3 on a slow CPU from a slow disk.
const START_TIMEOUT: Duration = Duration::from_secs(120);

impl Whisper {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone().unwrap_or(Status::of("stopped", None))
    }

    fn set_status(&self, app: &tauri::AppHandle, status: Status) {
        use tauri::Emitter;
        *self.status.lock().unwrap() = Some(status.clone());
        let _ = app.emit("local-status", status);
    }

    /// Returns the address of a server running the configured model, starting (or
    /// restarting) it first if needed. Blocks while the model loads.
    ///
    /// Reads the settings only once it holds the lock, so two quick changes in a row end with
    /// the newer one loaded. A running server is only replaced once its successor is fully
    /// downloaded - choosing a model that is still on its way keeps the old one working.
    pub fn ensure(&self, app: &tauri::AppHandle) -> Result<String, String> {
        use tauri::Manager;
        let mut running = self.running.lock().unwrap();
        let core = app.state::<crate::state::Core>();
        let layout = &core.paths.layout;
        let settings = core.settings().local;
        let key = Key { runtime: settings.runtime.clone(), model: settings.model.clone(), threads: settings.threads };

        let rt = local::runtime(&key.runtime).ok_or("Unknown runtime.")?;
        let model = local::model(&key.model).ok_or("Unknown model.")?;
        let missing = if !layout.runtime_installed(rt) {
            Some(format!("The local engine ({}) is not installed yet - download it in the settings.", rt.label))
        } else if !layout.model_installed(model) {
            Some(format!("The model {} is not downloaded yet - get it in the settings.", model.label))
        } else {
            None
        };

        if let Some(r) = running.as_mut() {
            let alive = matches!(r.child.try_wait(), Ok(None));
            if alive && (r.key == key || missing.is_some()) {
                // The configured one, or the previous one while the new one is not there yet.
                return Ok(format!("http://127.0.0.1:{}", r.port));
            }
            if !alive {
                crate::diag::warn(format!("local engine had exited: {}", self.last_output()));
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
                crate::diag::info(format!("local engine ready: {} on {} in {} ms", model.label, rt.label, started.elapsed().as_millis()));
                *running = Some(r);
                let gpu = if rt.id == "cuda" { self.gpu_in_use() } else { None };
                let message = if gpu == Some(false) {
                    // whisper.cpp falls back to the CPU without complaint; say it out loud.
                    crate::diag::warn(format!("GPU not used, running on the CPU: {}", self.last_output()));
                    format!("{} - running on the CPU: the GPU could not be used (update the NVIDIA driver?)", model.label)
                } else {
                    if rt.id == "cuda" && gpu.is_none() {
                        crate::diag::info("local engine did not report its backend");
                    }
                    format!("{} on {}", model.label, rt.label)
                };
                self.set_status(app, Status::of("ready", Some(message)));
                Ok(url)
            }
            Err(e) => {
                crate::diag::error(format!("local engine failed to start: {e}"));
                self.set_status(app, Status::of("failed", Some(e.clone())));
                Err(e)
            }
        }
    }

    fn spawn(&self, layout: &Layout, rt: &local::Runtime, model: &local::Model, key: &Key) -> Result<Running, String> {
        let port = process::free_port().map_err(|e| format!("No free port: {e}"))?;
        let threads = if key.threads == 0 { process::default_threads() } else { key.threads };
        let mut cmd = Command::new(layout.server_exe(rt));
        // whisper-server reads its arguments in the system code page; a user folder with other
        // characters would arrive mangled. So it runs in the models folder and gets only the
        // file name - its DLLs still load from the exe's own folder.
        let model_path = layout.model_path(model);
        cmd.current_dir(model_path.parent().unwrap_or(&layout.root))
            .args(["--model", model.file])
            .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--threads", &threads.to_string()]);
        let child = process::spawn(cmd, &self.output).map_err(|e| format!("The local engine could not be started: {e}"))?;

        // Not the open port: this whisper-server opens it before the model has loaded and
        // answers 503 until then.
        let base = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + START_TIMEOUT;
        let mut running = Running { child, port, key: key.clone() };
        loop {
            if let Ok(Some(status)) = running.child.try_wait() {
                return Err(format!("The local engine stopped while loading ({status}). {}", self.explain()));
            }
            if amber_core::engine::local_server_ready(&base) {
                return Ok(running);
            }
            if Instant::now() > deadline {
                process::stop(&mut running.child);
                return Err("The local engine took too long to load the model.".into());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// The server prints its backend while loading, before it opens the port; the reader
    /// threads may lag a moment behind.
    fn gpu_in_use(&self) -> Option<bool> {
        for _ in 0..60 {
            let out = self.output.lock().unwrap();
            if let Some(answer) = local::gpu_in_use(out.iter().map(String::as_str)) {
                if let Some(line) = out.iter().find(|l| l.contains("CUDA0") || l.contains("no GPU found")) {
                    crate::diag::info(format!("local engine: {}", line.trim()));
                }
                return Some(answer);
            }
            drop(out);
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    fn last_output(&self) -> String {
        process::last_lines(&self.output, 6)
    }

    /// The server's last words, turned into advice where they are recognizable.
    fn explain(&self) -> String {
        let out = self.last_output();
        let lower = out.to_lowercase();
        if lower.contains("cuda") && (lower.contains("error") || lower.contains("failed") || lower.contains("no device")) {
            format!("The GPU could not be used - update the NVIDIA driver or switch to the CPU engine. ({out})")
        } else if lower.contains("out of memory") || lower.contains("failed to allocate") {
            format!("Not enough memory for this model - pick a smaller one. ({out})")
        } else {
            out
        }
    }

    pub fn stop(&self, app: &tauri::AppHandle) {
        if let Some(mut r) = self.running.lock().unwrap().take() {
            process::stop(&mut r.child);
            crate::diag::info("local engine stopped");
        }
        self.set_status(app, Status::of("stopped", None));
    }

    /// Stops the server only if it runs from [runtime] - before that runtime's files are
    /// replaced or deleted.
    pub fn stop_runtime(&self, app: &tauri::AppHandle, runtime: &str) {
        let uses = self.running.lock().unwrap().as_ref().is_some_and(|r| r.key.runtime == runtime);
        if uses {
            self.stop(app);
        }
    }

    /// Stops the server only if it has [model] loaded - before the file is deleted.
    pub fn stop_model(&self, app: &tauri::AppHandle, model: &str) {
        let uses = self.running.lock().unwrap().as_ref().is_some_and(|r| r.key.model == model);
        if uses {
            self.stop(app);
        }
    }

    /// The server died and the next dictation will restart it - or it is running fine.
    /// Never waits: while another thread loads a model (and holds the lock), the answer is
    /// simply "not ready".
    pub fn is_ready(&self) -> bool {
        match self.running.try_lock() {
            Ok(mut running) => running.as_mut().map(|r| matches!(r.child.try_wait(), Ok(None))).unwrap_or(false),
            Err(_) => false,
        }
    }
}

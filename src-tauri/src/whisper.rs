//! The local engine: whisper.cpp's `whisper-server.exe`, started with the chosen model on a
//! free port on 127.0.0.1. It keeps the model in memory between dictations. A Windows job
//! object ties it to the app, so it never outlives Amber - not even after a crash.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use amber_core::local::{self, Layout};
use amber_core::settings::LocalSettings;
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// "stopped", "starting", "ready", "failed"
    pub state: &'static str,
    pub message: Option<String>,
}

impl Status {
    fn of(state: &'static str, message: Option<String>) -> Self {
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
    output: Arc<Mutex<VecDeque<String>>>,
    status: Mutex<Option<Status>>,
    #[cfg(windows)]
    job: Mutex<Option<JobHandle>>,
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
    pub fn ensure(&self, app: &tauri::AppHandle, layout: &Layout, settings: &LocalSettings) -> Result<String, String> {
        let key = Key { runtime: settings.runtime.clone(), model: settings.model.clone(), threads: settings.threads };
        let mut running = self.running.lock().unwrap();
        if let Some(r) = running.as_mut() {
            let alive = matches!(r.child.try_wait(), Ok(None));
            if alive && r.key == key {
                return Ok(format!("http://127.0.0.1:{}", r.port));
            }
            if !alive {
                crate::diag::warn(format!("local engine had exited: {}", self.last_output()));
            }
            stop_child(r);
            *running = None;
        }

        let rt = local::runtime(&key.runtime).ok_or("Unknown runtime.")?;
        let model = local::model(&key.model).ok_or("Unknown model.")?;
        if !layout.runtime_installed(rt) {
            let msg = format!("The local engine ({}) is not installed yet - download it in the settings.", rt.label);
            self.set_status(app, Status::of("failed", Some(msg.clone())));
            return Err(msg);
        }
        if !layout.model_installed(model) {
            let msg = format!("The model {} is not downloaded yet - get it in the settings.", model.label);
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
                self.set_status(app, Status::of("ready", Some(format!("{} on {}", model.label, rt.label))));
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
        let port = free_port().map_err(|e| format!("No free port: {e}"))?;
        let threads = if key.threads == 0 { default_threads() } else { key.threads };
        let mut cmd = Command::new(layout.server_exe(rt));
        cmd.current_dir(layout.runtime_dir(rt))
            .arg("--model")
            .arg(layout.model_path(model))
            .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--threads", &threads.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| format!("The local engine could not be started: {e}"))?;
        #[cfg(windows)]
        self.attach_to_job(&child);

        self.output.lock().unwrap().clear();
        for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
                     child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>)]
            .into_iter()
            .flatten()
        {
            let out = self.output.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    let mut out = out.lock().unwrap();
                    if out.len() >= 40 {
                        out.pop_front();
                    }
                    out.push_back(line);
                }
            });
        }

        // whisper-server opens its port only after the model has loaded.
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let deadline = Instant::now() + START_TIMEOUT;
        let mut running = Running { child, port, key: key.clone() };
        loop {
            if let Ok(Some(status)) = running.child.try_wait() {
                return Err(format!("The local engine stopped while loading ({status}). {}", self.explain()));
            }
            if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                return Ok(running);
            }
            if Instant::now() > deadline {
                stop_child(&mut running);
                return Err("The local engine took too long to load the model.".into());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    fn last_output(&self) -> String {
        self.output.lock().unwrap().iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join(" | ")
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
            stop_child(&mut r);
            crate::diag::info("local engine stopped");
        }
        self.set_status(app, Status::of("stopped", None));
    }

    /// The server died and the next dictation will restart it - or it is running fine.
    pub fn is_ready(&self) -> bool {
        self.running.lock().unwrap().as_mut().map(|r| matches!(r.child.try_wait(), Ok(None))).unwrap_or(false)
    }

    #[cfg(windows)]
    fn attach_to_job(&self, child: &Child) {
        use std::os::windows::io::AsRawHandle;
        let mut job = self.job.lock().unwrap();
        if job.is_none() {
            *job = JobHandle::new();
        }
        if let Some(job) = job.as_ref() {
            job.assign(child.as_raw_handle());
        }
    }
}

fn stop_child(r: &mut Running) {
    let _ = r.child.kill();
    let _ = r.child.wait();
}

fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Physical cores are what whisper.cpp scales with; hyper-threads mostly get in the way.
/// Half the logical processors, capped at 8.
fn default_threads() -> u32 {
    let logical = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
    (logical / 2).clamp(2, 8)
}

#[cfg(windows)]
struct JobHandle(windows::Win32::Foundation::HANDLE);

// The handle is only used behind the Whisper mutex.
#[cfg(windows)]
unsafe impl Send for JobHandle {}

#[cfg(windows)]
impl JobHandle {
    fn new() -> Option<Self> {
        use windows::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .ok()?;
            Some(JobHandle(job))
        }
    }

    fn assign(&self, process: std::os::windows::io::RawHandle) {
        use windows::Win32::System::JobObjects::AssignProcessToJobObject;
        let process = windows::Win32::Foundation::HANDLE(process);
        if let Err(e) = unsafe { AssignProcessToJobObject(self.0, process) } {
            crate::diag::warn(format!("local engine not tied to the app: {e}"));
        }
    }
}

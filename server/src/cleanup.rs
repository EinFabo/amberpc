//! The cleanup: llama.cpp's own llama-server as a child process, bound to 127.0.0.1 with a
//! random key. One model at a time - a request naming another downloaded one restarts it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use amber_core::cleanup::ChatCleaner;
use amber_core::engine::ErrorKind;
use amber_core::local::LlmModel;
use serde_json::Value;

use crate::paths::llm_ready;

/// After a start failed: how long the same model answers with that reason instead of trying
/// again - an old Vulkan driver or too little memory does not fix itself per request.
const RETRY: Duration = Duration::from_secs(60);

/// A started llama-server, as far as the cleanup needs to know.
pub trait Handle: Send {
    fn alive(&mut self) -> bool;
    /// Its last output lines - the reason when it dies.
    fn output(&self) -> String;
    fn stop(&mut self);
}

/// Starts llama-server for a model file on a port with a key.
pub trait Launcher: Send + Sync {
    fn launch(&self, file: &Path, port: u16, key: &str) -> Result<Box<dyn Handle>, String>;
}

pub struct Cleanup {
    dir: PathBuf,
    launcher: Box<dyn Launcher>,
    catalog: &'static [LlmModel],
    ready_within: Duration,
    /// Held for a whole request: llama-server answers one at a time anyway, and a switch must
    /// not pull the model out from under a request.
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    /// The model whose start failed, when, and why.
    failed: Option<(String, Instant, String)>,
}

struct Running {
    id: String,
    url: String,
    key: String,
    handle: Box<dyn Handle>,
}

impl Cleanup {
    pub fn new(dir: PathBuf, exe: PathBuf, threads: u32) -> Cleanup {
        Cleanup::with(dir, Box::new(LlamaLauncher { exe, threads }), &amber_core::local::LLM_MODELS, Duration::from_secs(180))
    }

    pub fn with(dir: PathBuf, launcher: Box<dyn Launcher>, catalog: &'static [LlmModel], ready_within: Duration) -> Cleanup {
        Cleanup { dir, launcher, catalog, ready_within, state: Mutex::new(State::default()) }
    }

    /// A chat request answered by [request]'s `model` if it is downloaded, else by [default].
    pub fn answer(&self, request: &Value, default: &str) -> Result<Value, (u16, String)> {
        let id = self.chosen(request["model"].as_str(), default)?;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((failed, at, why)) = &st.failed {
            if *failed == id && at.elapsed() < RETRY {
                return Err((503, why.clone()));
            }
        }
        if !st.running.as_mut().is_some_and(|r| r.id == id && r.handle.alive()) {
            if let Some(mut old) = st.running.take() {
                old.handle.stop();
            }
            match self.start(&id) {
                Ok(r) => {
                    st.failed = None;
                    st.running = Some(r);
                }
                Err(why) => {
                    st.failed = Some((id, Instant::now(), why.clone()));
                    return Err((503, why));
                }
            }
        }
        let r = st.running.as_ref().expect("started above");
        ChatCleaner::new(&r.url, &r.key, &r.id, None).forward(request).map_err(|e| {
            let status = match e.kind {
                ErrorKind::Timeout => 504,
                ErrorKind::RateLimited => 429,
                ErrorKind::Unreachable | ErrorKind::Unavailable => 503,
                _ => 502,
            };
            (status, e.message)
        })
    }

    fn chosen(&self, requested: Option<&str>, default: &str) -> Result<String, (u16, String)> {
        let ready = |id: &str| self.catalog.iter().find(|m| m.id == id).filter(|m| llm_ready(&self.dir, m)).map(|m| m.id.to_string());
        requested
            .and_then(ready)
            .or_else(|| ready(default))
            .ok_or_else(|| (503, "no cleanup model on this server - amber-server cleanup add <id>".to_string()))
    }

    fn start(&self, id: &str) -> Result<Running, String> {
        let m = self.catalog.iter().find(|m| m.id == id).ok_or("unknown cleanup model")?;
        let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map_err(|e| format!("no free port: {e}"))?.port();
        let key = amber_core::api::new_key();
        let mut handle = self.launcher.launch(&self.dir.join(m.file), port, &key)?;
        let url = format!("http://127.0.0.1:{port}");
        let until = Instant::now() + self.ready_within;
        loop {
            if !handle.alive() {
                return Err(format!("the cleanup model {id} did not start: {}", handle.output()));
            }
            // llama-server answers 503 while it loads, 200 once ready.
            if amber_core::engine::local_server_ready(&url) {
                return Ok(Running { id: id.to_string(), url, key, handle });
            }
            if Instant::now() > until {
                handle.stop();
                return Err(format!("the cleanup model {id} took too long to load"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The real llama-server.
struct LlamaLauncher {
    exe: PathBuf,
    threads: u32,
}

impl Handle for crate::child::Child {
    fn alive(&mut self) -> bool {
        crate::child::Child::alive(self)
    }
    fn output(&self) -> String {
        crate::child::Child::output(self)
    }
    fn stop(&mut self) {
        crate::child::Child::stop(self)
    }
}

impl Launcher for LlamaLauncher {
    fn launch(&self, file: &Path, port: u16, key: &str) -> Result<Box<dyn Handle>, String> {
        let threads = if self.threads == 0 { amber_core::local::default_threads() } else { self.threads };
        let mut cmd = Command::new(&self.exe);
        cmd.arg("--model")
            .arg(file)
            .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--api-key", key])
            .args(["--ctx-size", "4096", "--parallel", "1", "--threads", &threads.to_string()]);
        // llama.cpp's Linux build keeps its libraries next to the program.
        #[cfg(unix)]
        if let Some(dir) = self.exe.parent() {
            cmd.env("LD_LIBRARY_PATH", dir);
        }
        crate::child::Child::spawn(cmd).map(|c| Box::new(c) as Box<dyn Handle>).map_err(|e| format!("the cleanup engine could not start: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    static MODELS: [LlmModel; 2] = [
        LlmModel { id: "a", file: "a.gguf", label: "A", note: "", url: "https://x/a", sha256: "0", size_mb: 1 },
        LlmModel { id: "b", file: "b.gguf", label: "B", note: "", url: "https://x/b", sha256: "0", size_mb: 1 },
    ];

    /// Stands in for llama-server: answers /health, and chat requests with the model file it
    /// was started with.
    struct Fake {
        launches: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    struct FakeHandle {
        stop: Arc<AtomicBool>,
        dead: bool,
    }

    impl Handle for FakeHandle {
        fn alive(&mut self) -> bool {
            !self.dead && !self.stop.load(Ordering::SeqCst)
        }
        fn output(&self) -> String {
            "llama: failed to allocate the model (out of memory)".into()
        }
        fn stop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    impl Launcher for Fake {
        fn launch(&self, file: &Path, port: u16, _key: &str) -> Result<Box<dyn Handle>, String> {
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            self.launches.lock().unwrap().push(name.clone());
            let stop = Arc::new(AtomicBool::new(false));
            if self.fail {
                return Ok(Box::new(FakeHandle { stop, dead: true }));
            }
            let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let flag = stop.clone();
            std::thread::spawn(move || {
                while !flag.load(Ordering::SeqCst) {
                    let Ok((mut s, _)) = listener.accept() else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    };
                    s.set_nonblocking(false).unwrap();
                    let mut buf = [0u8; 65536];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let body = if head.starts_with("GET /health") {
                        r#"{"status":"ok"}"#.to_string()
                    } else {
                        serde_json::json!({ "choices": [{ "message": { "role": "assistant", "content": name }, "finish_reason": "stop" }] }).to_string()
                    };
                    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                }
            });
            Ok(Box::new(FakeHandle { stop, dead: false }))
        }
    }

    fn setup(fail: bool, files: &[&str]) -> (tempfile::TempDir, Cleanup, Arc<Mutex<Vec<String>>>) {
        let dir = tempfile::tempdir().unwrap();
        for f in files {
            std::fs::write(dir.path().join(f), b"x").unwrap();
        }
        let launches = Arc::new(Mutex::new(Vec::new()));
        let c = Cleanup::with(dir.path().to_path_buf(), Box::new(Fake { launches: launches.clone(), fail }), &MODELS, Duration::from_secs(5));
        (dir, c, launches)
    }

    fn ask(model: &str) -> serde_json::Value {
        serde_json::json!({ "model": model, "messages": [{ "role": "user", "content": "x" }] })
    }

    fn content(v: &serde_json::Value) -> &str {
        v["choices"][0]["message"]["content"].as_str().unwrap()
    }

    #[test]
    fn chat_starts_the_model_once_and_switches_on_request() {
        let (_d, c, launches) = setup(false, &["a.gguf", "b.gguf"]);
        assert_eq!(content(&c.answer(&ask("a"), "a").unwrap()), "a.gguf");
        assert_eq!(content(&c.answer(&ask("a"), "a").unwrap()), "a.gguf");
        assert_eq!(launches.lock().unwrap().len(), 1, "started once");
        assert_eq!(content(&c.answer(&ask("b"), "a").unwrap()), "b.gguf");
        assert_eq!(*launches.lock().unwrap(), ["a.gguf", "b.gguf"]);
    }

    #[test]
    fn a_model_not_downloaded_gets_the_default() {
        let (_d, c, _) = setup(false, &["a.gguf"]);
        assert_eq!(content(&c.answer(&ask("b"), "a").unwrap()), "a.gguf");
        assert_eq!(content(&c.answer(&ask("gpt-4"), "a").unwrap()), "a.gguf");
        let (_d, c, _) = setup(false, &[]);
        let err = c.answer(&ask("a"), "a").unwrap_err();
        assert_eq!(err.0, 503);
        assert!(err.1.contains("amber-server cleanup add"), "{}", err.1);
    }

    #[test]
    fn a_failing_start_answers_503_and_waits() {
        let (_d, c, launches) = setup(true, &["a.gguf"]);
        let err = c.answer(&ask("a"), "a").unwrap_err();
        assert_eq!(err.0, 503);
        assert!(err.1.contains("out of memory"), "{}", err.1);
        assert!(c.answer(&ask("a"), "a").is_err());
        assert_eq!(launches.lock().unwrap().len(), 1, "no second start within a minute");
    }
}

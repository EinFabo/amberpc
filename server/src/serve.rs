//! `amber-server run`: the phone API on this machine.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use amber_core::api::{self, Hooks, Limits, ModelInfo, ModelKind};
use amber_core::engine::{SttError, TranscribeOptions, Transcriber};
use amber_core::local;
use amber_core::text::{by_rule, Rules};

use crate::cleanup::Cleanup;
use crate::config::Config;
use crate::paths::{llm_ready, model_ready};
use crate::speech::Speech;

/// Where the models are.
pub struct Dirs {
    pub speech: PathBuf,
    pub cleanup: PathBuf,
    pub data: PathBuf,
}

impl Dirs {
    pub fn real() -> Dirs {
        Dirs { speech: crate::paths::speech_dir(), cleanup: crate::paths::cleanup_dir(), data: crate::paths::data_dir() }
    }
}

/// Exit code for "set it up first" - systemd's unit does not restart on it.
pub const EX_CONFIG: i32 = 78;

/// Serves until the process ends. Without a config, a key or the speech model it exits with
/// [EX_CONFIG] and says what to run.
pub fn serve() -> ! {
    match start() {
        Ok(()) => loop {
            std::thread::park();
        },
        Err((code, message)) => {
            eprintln!("amber-server: {message}");
            std::process::exit(code);
        }
    }
}

fn start() -> Result<(), (i32, String)> {
    let setup = |m: String| (EX_CONFIG, m);
    // The wrong build for this CPU: restarting would not help.
    crate::cpu::cpu_check().map_err(setup)?;
    let path = Config::path();
    if !path.exists() {
        return Err(setup("not set up yet - run: amber-server setup".into()));
    }
    let config = Config::load(&path).map_err(setup)?;
    if config.key.trim().is_empty() {
        return Err(setup("no access key yet - run: amber-server setup".into()));
    }
    let dirs = Dirs::real();
    let speech = local::model(&config.speech).filter(|m| model_ready(&dirs.speech, m));
    if speech.is_none() {
        return Err(setup(format!("the speech model {} is not downloaded - run: amber-server setup", config.speech)));
    }
    let gpu = crate::catalog::machine(&config).gpu;
    let cleanup = cleanup_for(&config, &dirs, || crate::catalog::ensure_llama(&dirs.data, gpu));
    let hooks = hooks(&config, &dirs, cleanup.clone());
    match api::start_with(SocketAddr::from(([0, 0, 0, 0], config.port)), hooks, Limits::default()) {
        Ok(_server) => {
            // The server runs on its own threads for as long as the process lives.
            std::mem::forget(_server);
            log(format!("Amber Server {} listening on port {} - the phone's address: amber-server url", env!("CARGO_PKG_VERSION"), config.port));
            if let (Some(c), Some(id)) = (cleanup, config.cleanup.clone()) {
                // Loaded now: the phone gives up on a cleanup after 12 s, a load can take longer.
                std::thread::spawn(move || match c.warm(&id) {
                    Ok(()) => log(format!("cleanup model {id} ready")),
                    Err(e) => log(format!("cleanup model {id} did not start: {e}")),
                });
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            Err((1, format!("port {} is in use - pick another: amber-server config set port <number>", config.port)))
        }
        Err(e) => Err((1, format!("the API could not start: {e}"))),
    }
}

/// The cleanup, when a downloaded model is configured and llama-server is there or could be
/// fetched. Without it the server still transcribes - a failed download (no network yet at
/// boot) must not take the speech down with it.
pub fn cleanup_for(config: &Config, dirs: &Dirs, ensure_llama: impl FnOnce() -> Result<PathBuf, String>) -> Option<Arc<Cleanup>> {
    let id = config.cleanup.as_deref()?;
    if !local::llm_model(id).is_some_and(|m| llm_ready(&dirs.cleanup, m)) {
        log(format!("cleanup off: {id} is not downloaded - amber-server cleanup add {id}"));
        return None;
    }
    match ensure_llama() {
        Ok(exe) => Some(Arc::new(Cleanup::new(dirs.cleanup.clone(), exe, config.threads))),
        Err(e) => {
            log(format!("cleanup off until the next start: llama-server is missing and could not be fetched ({e})"));
            None
        }
    }
}

pub fn hooks(config: &Config, dirs: &Dirs, cleanup: Option<Arc<Cleanup>>) -> Hooks {
    let gpu = crate::catalog::machine(config).gpu;
    let speech = Arc::new(Speech::new(dirs.speech.clone(), config.threads, gpu));
    let (key, default_speech, default_cleanup) = (config.key.clone(), config.speech.clone(), config.cleanup.clone());
    let (language, vocabulary) = (config.language.clone(), config.vocabulary.clone());
    let rules = config.rules;
    let rules_language = (config.language != "auto").then(|| config.language.clone());
    let list = model_list(config, dirs, gpu, cleanup.is_some());
    Hooks {
        service: "amber-server",
        version: env!("CARGO_PKG_VERSION"),
        key: Arc::new(move || key.clone()),
        engine: Arc::new(move |requested| {
            let inner = speech.engine(requested, &default_speech)?;
            Ok(Box::new(WithDefaults { inner, language: language.clone(), vocabulary: vocabulary.clone() }) as Box<dyn Transcriber>)
        }),
        models: Arc::new(move || list.clone()),
        tidy: Arc::new(move |text| {
            by_rule(
                &text,
                Rules {
                    fillers: rules.fillers,
                    spoken_punctuation: rules.spoken_punctuation,
                    german_commas: rules.german_commas,
                    language: rules_language.as_deref(),
                },
            )
        }),
        record: Arc::new(|_, _, _| {}),
        chat: Arc::new(move |request| match (&cleanup, &default_cleanup) {
            (Some(c), Some(default)) => c.answer(request, default),
            _ => Err((503, "no cleanup model on this server - amber-server cleanup add <id>".into())),
        }),
        log: Arc::new(log),
    }
}

/// What the phone's choice shows: every downloaded model, the defaults marked - cleanup models
/// only while the cleanup runs ([cleanup_on]), else the phone would offer what answers 503.
pub fn model_list(config: &Config, dirs: &Dirs, gpu: bool, cleanup_on: bool) -> Vec<ModelInfo> {
    let on = if gpu { "GPU" } else { "CPU" };
    let speech = local::MODELS.iter().filter(|m| model_ready(&dirs.speech, m)).map(|m| ModelInfo {
        id: m.id.into(),
        kind: ModelKind::Speech,
        label: m.label.into(),
        note: format!("{} MB · {on}", m.size_mb),
        default: config.speech == m.id,
    });
    let cleanup = local::LLM_MODELS.iter().filter(|m| cleanup_on && llm_ready(&dirs.cleanup, m)).map(|m| ModelInfo {
        id: m.id.into(),
        kind: ModelKind::Cleanup,
        label: m.label.into(),
        note: format!("{} MB · {on}", m.size_mb),
        default: config.cleanup.as_deref() == Some(m.id),
    });
    speech.chain(cleanup).collect()
}

/// The server's language and terms where the phone sent none: the phone's language wins
/// unless it is "auto"; the server's vocabulary goes last, where it survives whisper's limit.
pub struct WithDefaults {
    pub inner: Box<dyn Transcriber>,
    pub language: String,
    pub vocabulary: String,
}

impl Transcriber for WithDefaults {
    fn transcribe(&self, wav: &[u8], options: &TranscribeOptions) -> Result<String, SttError> {
        let language = options.language.clone().or_else(|| (self.language != "auto").then(|| self.language.clone()));
        let prompt = match (options.prompt.as_deref().filter(|p| !p.trim().is_empty()), self.vocabulary.trim()) {
            (Some(p), "") => Some(p.to_string()),
            (Some(p), v) => Some(format!("{p}, {v}")),
            (None, "") => None,
            (None, v) => Some(v.to_string()),
        };
        self.inner.transcribe(wav, &TranscribeOptions { language, prompt })
    }
}

/// A log line with the UTC time - journald and the Windows task's log file keep them.
fn log(message: String) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    println!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} {message}", rest / 3_600, rest % 3_600 / 60, rest % 60);
}

#[cfg(test)]
mod tests {
    use super::*;
    use amber_core::engine::{SttError, TranscribeOptions, Transcriber};
    use std::sync::Mutex;

    fn dirs_with(speech: &[&str], cleanup: &[&str]) -> (tempfile::TempDir, Dirs) {
        let dir = tempfile::tempdir().unwrap();
        let d = Dirs { speech: dir.path().join("models"), cleanup: dir.path().join("llm"), data: dir.path().to_path_buf() };
        std::fs::create_dir_all(&d.speech).unwrap();
        std::fs::create_dir_all(&d.cleanup).unwrap();
        for id in speech {
            std::fs::write(d.speech.join(amber_core::local::model(id).unwrap().file), b"x").unwrap();
        }
        for id in cleanup {
            std::fs::write(d.cleanup.join(amber_core::local::llm_model(id).unwrap().file), b"x").unwrap();
        }
        (dir, d)
    }

    #[test]
    fn the_model_list_holds_what_is_downloaded() {
        let (_t, d) = dirs_with(&["futo-multilingual-74", "futo-multilingual-244"], &["qwen3-4b-instruct-2507-q4km"]);
        let config = Config { speech: "futo-multilingual-244".into(), cleanup: Some("qwen3-4b-instruct-2507-q4km".into()), ..Config::default() };
        let list = model_list(&config, &d, false, true);
        assert_eq!(list.len(), 3);
        assert_eq!(list.iter().filter(|m| m.kind == ModelKind::Speech).count(), 2);
        let defaults: Vec<&str> = list.iter().filter(|m| m.default).map(|m| m.id.as_str()).collect();
        assert_eq!(defaults, ["futo-multilingual-244", "qwen3-4b-instruct-2507-q4km"]);
        assert!(list[0].note.contains("MB") && list[0].note.contains("CPU"), "{}", list[0].note);
    }

    #[test]
    fn cleanup_models_are_listed_only_while_the_cleanup_is_on() {
        let (_t, d) = dirs_with(&["futo-multilingual-74"], &["qwen3-4b-instruct-2507-q4km"]);
        // `cleanup off` keeps the file; the phone must not offer it then.
        let config = Config { speech: "futo-multilingual-74".into(), cleanup: None, ..Config::default() };
        assert!(model_list(&config, &d, false, false).iter().all(|m| m.kind == ModelKind::Speech));
        let h = hooks(&Config { cleanup: Some("qwen3-4b-instruct-2507-q4km".into()), ..config }, &d, None);
        assert!((h.models)().iter().all(|m| m.kind == ModelKind::Speech), "configured, but not running");
    }

    #[test]
    fn a_failed_llama_download_leaves_the_speech_running() {
        let (_t, d) = dirs_with(&["futo-multilingual-74"], &["qwen3-4b-instruct-2507-q4km"]);
        let config = Config { cleanup: Some("qwen3-4b-instruct-2507-q4km".into()), ..Config::default() };
        assert!(cleanup_for(&config, &d, || Err("network down".into())).is_none());
        assert!(cleanup_for(&config, &d, || Ok("/opt/llama-server".into())).is_some());
        assert!(cleanup_for(&Config::default(), &d, || panic!("no cleanup configured")).is_none());
    }

    #[test]
    fn chat_without_a_cleanup_model_says_how_to_get_one() {
        let (_t, d) = dirs_with(&["futo-multilingual-74"], &[]);
        let h = hooks(&Config::default(), &d, None);
        let err = (h.chat)(&serde_json::json!({ "messages": [{ "role": "user", "content": "x" }] })).unwrap_err();
        assert_eq!(err.0, 503);
        assert!(err.1.contains("amber-server cleanup add"), "{}", err.1);
        assert_eq!(h.service, "amber-server");
    }

    #[test]
    fn the_rules_from_the_config_apply() {
        let (_t, d) = dirs_with(&[], &[]);
        let mut config = Config::default();
        assert_eq!((hooks(&config, &d, None).tidy)("ähm hallo".into()), "Hallo");
        config.rules.fillers = false;
        assert_eq!((hooks(&config, &d, None).tidy)("ähm hallo".into()), "ähm hallo");
    }

    struct Seen(Mutex<Vec<TranscribeOptions>>);
    impl Transcriber for Seen {
        fn transcribe(&self, _wav: &[u8], options: &TranscribeOptions) -> Result<String, SttError> {
            self.0.lock().unwrap().push(options.clone());
            Ok(String::new())
        }
    }

    #[test]
    fn the_configs_language_and_vocabulary_reach_the_engine() {
        let seen = std::sync::Arc::new(Seen(Mutex::new(Vec::new())));
        let config = Config { language: "de".into(), vocabulary: "Tauri, Anna".into(), ..Config::default() };
        let engine = WithDefaults { inner: Box::new(Shared(seen.clone())), language: config.language.clone(), vocabulary: config.vocabulary.clone() };
        engine.transcribe(b"", &TranscribeOptions::new("auto", "Groq")).unwrap();
        engine.transcribe(b"", &TranscribeOptions::new("en", "")).unwrap();
        let s = seen.0.lock().unwrap();
        assert_eq!(s[0].language.as_deref(), Some("de"), "auto from the phone: the server's language");
        assert_eq!(s[0].prompt.as_deref(), Some("Groq, Tauri, Anna"), "the server's terms last, where they survive the limit");
        assert_eq!(s[1].language.as_deref(), Some("en"), "the phone's own language wins");
        assert_eq!(s[1].prompt.as_deref(), Some("Tauri, Anna"));
    }

    struct Shared(std::sync::Arc<Seen>);
    impl Transcriber for Shared {
        fn transcribe(&self, wav: &[u8], options: &TranscribeOptions) -> Result<String, SttError> {
            self.0.transcribe(wav, options)
        }
    }
}

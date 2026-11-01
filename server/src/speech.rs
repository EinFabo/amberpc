//! Recognition in-process: whisper.cpp built into the binary (whisper-rs). One model in memory;
//! a request naming another downloaded one swaps it in.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};

use amber_core::engine::{ErrorKind, SttError, TranscribeOptions, Transcriber};
use amber_core::local::{self, Model};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

use crate::paths::model_ready;

pub struct Speech {
    dir: PathBuf,
    threads: u32,
    gpu: bool,
    /// The model in memory, by id. Held during a transcription, so a switch never pulls the
    /// model out from under one.
    loaded: Mutex<Option<Loaded>>,
}

/// A model with its one state, kept across requests: whisper.cpp detects the language with
/// the encoder context the state's previous run left - a fresh state means a full 30 s pass.
struct Loaded {
    id: String,
    /// Kept alive for the state.
    _ctx: WhisperContext,
    state: WhisperState,
}

impl Speech {
    pub fn new(dir: PathBuf, threads: u32, gpu: bool) -> Speech {
        // whisper.cpp reports every load and run on stderr; routed into `log`, which stays quiet.
        static QUIET: Once = Once::new();
        QUIET.call_once(whisper_rs::install_logging_hooks);
        Speech { dir, threads, gpu, loaded: Mutex::new(None) }
    }

    /// The engine for one request: [requested] if it is a downloaded model, else [default].
    pub fn engine(self: &Arc<Self>, requested: Option<&str>, default: &str) -> Result<Box<dyn Transcriber>, String> {
        let id = chosen(&self.dir, requested, default);
        let model = local::model(&id).ok_or_else(|| format!("unknown model {id}"))?;
        if !model_ready(&self.dir, model) {
            return Err(format!("the model {id} is not downloaded - amber-server models add {id}"));
        }
        Ok(Box::new(Engine { speech: self.clone(), model }))
    }
}

struct Engine {
    speech: Arc<Speech>,
    model: &'static Model,
}

impl Transcriber for Engine {
    fn transcribe(&self, wav: &[u8], options: &TranscribeOptions) -> Result<String, SttError> {
        let fail = |what: &str, e: String| SttError::new(ErrorKind::Server, what, e);
        let (samples, rate) = amber_core::audio::decode_wav(wav).map_err(|e| SttError::new(ErrorKind::BadResponse, "Not a WAV recording.", e))?;
        let samples = amber_core::audio::resample(&samples, rate, amber_core::audio::TARGET_RATE);
        let plan = params(options, samples.len(), self.model, self.speech.threads);

        let mut loaded = self.speech.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if loaded.as_ref().is_none_or(|l| l.id != self.model.id) {
            // One model in memory: the previous one goes first.
            *loaded = None;
            let mut cp = WhisperContextParameters::default();
            cp.use_gpu(self.speech.gpu);
            // whisper.cpp's own default, much faster on a GPU - but not with a shortened context
            // (local::flash_attention: later dictations came out as nonsense).
            cp.flash_attn(self.speech.gpu && amber_core::local::flash_attention(self.model));
            let ctx = WhisperContext::new_with_params(self.speech.dir.join(self.model.file), cp)
                .map_err(|e| fail("The speech model could not be loaded.", e.to_string()))?;
            let mut state = ctx.create_state().map_err(|e| fail("The speech model could not start.", e.to_string()))?;
            if self.model.short_context {
                prime(&mut state, plan.threads);
            }
            *loaded = Some(Loaded { id: self.model.id.to_string(), _ctx: ctx, state });
        }
        let state = &mut loaded.as_mut().expect("loaded above").state;

        let mut fp = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        // No retries at higher temperatures: on noise and silence they run each 30 s window
        // up to five times over (a 10-minute recording took 3.5 min instead of 1 on a CPU).
        fp.set_temperature(0.0);
        fp.set_temperature_inc(0.0);
        // Several languages to choose among: whisper.cpp's detection, then the likeliest of them -
        // short German speech is easily taken for Turkish or English otherwise.
        let picked = if plan.among.len() > 1 {
            let threads = plan.threads as usize;
            state.pcm_to_mel(&samples, threads).map_err(|e| fail("Recognition failed.", e.to_string()))?;
            let (_, probabilities) = state.lang_detect(0, threads).map_err(|e| fail("Recognition failed.", e.to_string()))?;
            let by_code: serde_json::Map<String, serde_json::Value> = probabilities
                .iter()
                .enumerate()
                .filter_map(|(id, p)| whisper_rs::get_lang_str(id as i32).map(|code| (code.to_string(), serde_json::json!(p))))
                .collect();
            Some(amber_core::engine::pick_language(&serde_json::Value::Object(by_code), &plan.among))
        } else {
            None
        };
        fp.set_language(picked.as_deref().or(plan.language.as_deref()));
        if let Some(prompt) = &plan.prompt {
            fp.set_initial_prompt(prompt);
        }
        if plan.audio_ctx > 0 {
            fp.set_audio_ctx(plan.audio_ctx as i32);
        }
        fp.set_n_threads(plan.threads);
        fp.set_no_timestamps(plan.no_timestamps);
        fp.set_print_progress(false);
        fp.set_print_realtime(false);
        fp.set_print_special(false);
        fp.set_print_timestamps(false);
        state.full(fp, &samples).map_err(|e| fail("Recognition failed.", e.to_string()))?;
        let text = state.as_iter().filter_map(|s| s.to_str_lossy().ok().map(|t| t.into_owned())).collect::<Vec<_>>().join("");
        if self.model.short_context && plan.audio_ctx == 0 {
            // A full window left the state at 30 s: the next short dictation's language
            // detection would pay for it.
            prime(state, plan.threads);
        }
        Ok(text)
    }
}

/// Leaves [state] at a short encoder context, as the desktop's `reset_context` does for its
/// whisper-server: 1.2 s of silence, the language given, a context of 256.
fn prime(state: &mut WhisperState, threads: i32) {
    let mut fp = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    fp.set_language(Some("en"));
    fp.set_audio_ctx(256);
    fp.set_n_threads(threads);
    fp.set_no_timestamps(true);
    fp.set_print_progress(false);
    fp.set_print_realtime(false);
    fp.set_print_special(false);
    fp.set_print_timestamps(false);
    let _ = state.full(fp, &vec![0.0; amber_core::audio::TARGET_RATE as usize * 6 / 5]);
}

/// A language code whisper knows ("de-DE" -> "de"), or None for detection: "auto", empty, or
/// unknown - an unknown id would put a start token where the language token belongs.
pub fn whisper_language(value: &str) -> Option<String> {
    let code: String = value.split(['-', '_']).next().unwrap_or("").chars().filter(|c| *c != '\0').collect::<String>().trim().to_lowercase();
    if code.is_empty() || code == "auto" {
        return None;
    }
    whisper_rs::get_lang_id(&code).map(|_| code)
}

/// The languages in [value] ("de,en", "de-DE, en_US") whisper knows, each once, in order.
pub fn whisper_languages(value: &str) -> Vec<String> {
    let mut codes = Vec::new();
    for code in value.split(',').filter_map(whisper_language) {
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

/// How one request is run.
#[derive(Debug, PartialEq)]
struct ParamsPlan {
    /// One language; None for detection - among [among] when that holds several.
    language: Option<String>,
    among: Vec<String>,
    prompt: Option<String>,
    /// Encoder positions; 0 for the model's full 30 s window.
    audio_ctx: u32,
    threads: i32,
    no_timestamps: bool,
}

/// How much of a prompt goes in: its end - whisper reads only the last ~223 tokens, and its
/// tokenizer slows down with the square of the length.
const PROMPT_CHARS: usize = 800;

fn params(options: &TranscribeOptions, samples: usize, m: &Model, threads: u32) -> ParamsPlan {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get() as u32);
    let threads = if threads == 0 { local::default_threads() } else { threads.min(cores) }.max(1);
    let prompt = options.prompt.as_deref().map(|p| {
        let clean: Vec<char> = p.chars().filter(|c| *c != '\0').collect();
        clean[clean.len().saturating_sub(PROMPT_CHARS)..].iter().collect::<String>()
    });
    let languages = options.language.as_deref().map(whisper_languages).unwrap_or_default();
    ParamsPlan {
        language: (languages.len() == 1).then(|| languages[0].clone()),
        among: if languages.len() > 1 { languages } else { Vec::new() },
        prompt: prompt.filter(|p| !p.trim().is_empty()),
        // Only tuned (ACFT) models may stop where the speech stops; a stock model loses words.
        audio_ctx: if m.short_context { local::audio_ctx_for(samples, m.context_slack) } else { 0 },
        threads: threads as i32,
        no_timestamps: true,
    }
}

/// [requested] if it is a downloaded model of the catalog, else [default].
fn chosen(dir: &Path, requested: Option<&str>, default: &str) -> String {
    requested
        .and_then(local::model)
        .filter(|m| model_ready(dir, m))
        .map_or_else(|| default.to_string(), |m| m.id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use amber_core::engine::TranscribeOptions;

    #[test]
    fn params_follow_the_model_and_the_request() {
        let tuned = amber_core::local::model("futo-multilingual-244").unwrap();
        let p = params(&TranscribeOptions::new("de", "Tauri"), 16_000 * 3, tuned, 4);
        assert_eq!(p.language.as_deref(), Some("de"));
        assert_eq!(p.prompt.as_deref(), Some("Tauri"));
        assert!(p.audio_ctx > 0 && p.audio_ctx < 1500, "a tuned model stops where the speech stops: {}", p.audio_ctx);
        assert!(p.no_timestamps);
        assert_eq!(p.threads, 4);
        let stock = amber_core::local::model("large-v3-turbo-q5_0").unwrap();
        let p = params(&TranscribeOptions::new("auto", ""), 16_000 * 3, stock, 0);
        assert_eq!(p.audio_ctx, 0, "a stock model keeps its 30 s window");
        assert_eq!(p.language, None);
        assert!((2..=8).contains(&p.threads), "0: half the logical cores, 2 to 8 - more slow ggml down: {}", p.threads);
    }

    #[test]
    fn a_language_list_is_kept_to_choose_among() {
        let m = local::model("futo-multilingual-74").unwrap();
        let p = params(&TranscribeOptions::new("de,en", ""), 16_000, m, 4);
        assert_eq!((p.language, p.among), (None, vec!["de".to_string(), "en".to_string()]));
        // A code whisper does not know falls away; one left is a fixed language.
        let p = params(&TranscribeOptions::new("de,xx", ""), 16_000, m, 4);
        assert_eq!((p.language.as_deref(), p.among.len()), (Some("de"), 0));
    }

    #[test]
    fn languages_prompts_and_threads_are_made_safe() {
        let m = amber_core::local::model("futo-multilingual-244").unwrap();
        let lang = |l: &str| params(&TranscribeOptions { language: Some(l.into()), prompt: None }, 16_000, m, 4).language;
        assert_eq!(lang("de-DE").as_deref(), Some("de"));
        assert_eq!(lang("Auto"), None);
        assert_eq!(lang("xx"), None, "not a language whisper knows: detected instead");
        assert_eq!(lang("de\0"), Some("de".into()));
        // The prompt: no NUL (whisper-rs would panic), and only its end - whisper reads ~223 tokens.
        let long = format!("{}Ende\0", "a".repeat(10_000));
        let p = params(&TranscribeOptions { language: None, prompt: Some(long) }, 16_000, m, 4).prompt.unwrap();
        assert!(p.chars().count() <= 800 && p.ends_with("Ende") && !p.contains('\0'));
        // Threads: never more than the machine has (an absurd count aborts whisper.cpp).
        let cores = std::thread::available_parallelism().unwrap().get() as i32;
        assert!(params(&TranscribeOptions::default(), 16_000, m, 100_000).threads <= cores);
        assert!(params(&TranscribeOptions::default(), 16_000, m, u32::MAX).threads >= 1);
    }

    #[test]
    fn an_unknown_or_missing_model_gets_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let default = "futo-multilingual-74";
        std::fs::write(dir.path().join(amber_core::local::model(default).unwrap().file), b"x").unwrap();
        assert_eq!(chosen(dir.path(), Some("large-v3-q5_0"), default), default, "not downloaded");
        assert_eq!(chosen(dir.path(), Some("whisper-1"), default), default, "not a model at all");
        assert_eq!(chosen(dir.path(), None, default), default);
        let other = amber_core::local::model("futo-multilingual-244").unwrap();
        std::fs::write(dir.path().join(other.file), b"x").unwrap();
        assert_eq!(chosen(dir.path(), Some(other.id), default), other.id, "downloaded: taken");
    }

    #[test]
    fn speech_is_shared_across_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Speech>();
    }

    #[test]
    #[ignore = "needs futo-multilingual-74 from ten_minutes_of_silence: cargo test -p amber-server --release -- --ignored"]
    fn a_short_dictation_after_a_long_one_stays_quick() {
        let cache = std::env::temp_dir().join("amber-server-test-models");
        let m = amber_core::local::model("futo-multilingual-74").unwrap();
        if !crate::paths::model_ready(&cache, m) {
            let cancel = std::sync::atomic::AtomicBool::new(false);
            amber_core::local::download_verified(&m.url(), m.sha256, &cache.join(m.file), &cancel, &mut |_, _| {}).unwrap();
        }
        let speech = std::sync::Arc::new(Speech::new(cache, 0, false));
        let engine = speech.engine(None, m.id).unwrap();
        let wav = |secs: usize| amber_core::audio::encode_wav(&vec![0.0; 16_000 * secs], 16_000);
        engine.transcribe(&wav(40), &TranscribeOptions::new("auto", "")).unwrap();
        let t = std::time::Instant::now();
        engine.transcribe(&wav(3), &TranscribeOptions::new("auto", "")).unwrap();
        let auto = t.elapsed();
        let t = std::time::Instant::now();
        engine.transcribe(&wav(3), &TranscribeOptions::new("de", "")).unwrap();
        let fixed = t.elapsed();
        eprintln!("3 s after 40 s: auto {auto:?}, de {fixed:?}");
        assert!(auto < fixed * 2 + std::time::Duration::from_millis(300), "detection ran a full window: {auto:?} vs {fixed:?}");
    }

    #[test]
    #[ignore = "downloads futo-multilingual-74 once: cargo test -p amber-server -- --ignored"]
    fn ten_minutes_of_silence() {
        let cache = std::env::temp_dir().join("amber-server-test-models");
        let m = amber_core::local::model("futo-multilingual-74").unwrap();
        if !crate::paths::model_ready(&cache, m) {
            let cancel = std::sync::atomic::AtomicBool::new(false);
            amber_core::local::download_verified(&m.url(), m.sha256, &cache.join(m.file), &cancel, &mut |_, _| {}).unwrap();
        }
        let speech = std::sync::Arc::new(Speech::new(cache, 0, false));
        let engine = speech.engine(None, m.id).unwrap();
        let wav = amber_core::audio::encode_wav(&vec![0.0; 16_000 * 600], 16_000);
        let started = std::time::Instant::now();
        let text = engine.transcribe(&wav, &TranscribeOptions::new("de", "")).unwrap();
        assert!(text.trim().len() < 200, "silence, not a novel: {text}");
        assert!(started.elapsed().as_secs() < 60, "took {:?}", started.elapsed());
    }
}


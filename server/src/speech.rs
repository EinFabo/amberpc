//! Recognition in-process: whisper.cpp built into the binary (whisper-rs). One model in memory;
//! a request naming another downloaded one swaps it in.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};

use amber_core::engine::{ErrorKind, SttError, TranscribeOptions, Transcriber};
use amber_core::local::{self, Model};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::paths::model_ready;

pub struct Speech {
    dir: PathBuf,
    threads: u32,
    gpu: bool,
    /// The model in memory, by id. Held during a transcription, so a switch never pulls the
    /// model out from under one.
    loaded: Mutex<Option<(String, WhisperContext)>>,
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
        if loaded.as_ref().is_none_or(|(id, _)| id != self.model.id) {
            // One model in memory: the previous one goes first.
            *loaded = None;
            let mut cp = WhisperContextParameters::default();
            cp.use_gpu(self.speech.gpu);
            let ctx = WhisperContext::new_with_params(self.speech.dir.join(self.model.file), cp)
                .map_err(|e| fail("The speech model could not be loaded.", e.to_string()))?;
            *loaded = Some((self.model.id.to_string(), ctx));
        }
        let (_, ctx) = loaded.as_ref().expect("loaded above");
        let mut state = ctx.create_state().map_err(|e| fail("The speech model could not start.", e.to_string()))?;

        let mut fp = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        // No retries at higher temperatures: on noise and silence they run each 30 s window
        // up to five times over (a 10-minute recording took 3.5 min instead of 1 on a CPU).
        fp.set_temperature(0.0);
        fp.set_temperature_inc(0.0);
        fp.set_language(plan.language.as_deref());
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
        Ok(state.as_iter().filter_map(|s| s.to_str_lossy().ok().map(|t| t.into_owned())).collect::<Vec<_>>().join(""))
    }
}

/// How one request is run.
#[derive(Debug, PartialEq)]
struct ParamsPlan {
    language: Option<String>,
    prompt: Option<String>,
    /// Encoder positions; 0 for the model's full 30 s window.
    audio_ctx: u32,
    threads: i32,
    no_timestamps: bool,
}

fn params(options: &TranscribeOptions, samples: usize, m: &Model, threads: u32) -> ParamsPlan {
    let threads = if threads == 0 { local::default_threads() } else { threads };
    ParamsPlan {
        language: options.language.clone(),
        prompt: options.prompt.clone(),
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


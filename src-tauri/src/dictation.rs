//! One dictation at a time: Idle -> Recording -> Transcribing -> Inserting -> Done -> Idle,
//! or -> Error -> Idle. Every step is announced to the bubble and the settings window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use amber_core::audio::{self, Rejected};
use amber_core::engine::{ErrorKind, OpenAiTranscriber, TranscribeOptions, Transcriber, WhisperServerTranscriber};
use amber_core::chunking::{self, ChunkPlanner, FrameVad};
use amber_core::{estimate, local};
use amber_core::providers;
use amber_core::settings::{EngineKind, Settings};
use amber_core::text;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::capture::{self, Recorder};
use crate::insert::{self, Outcome, Target};
use crate::state::Core;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Recording,
    Transcribing,
    Inserting,
    Done,
    Error,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StateEvent {
    pub state: Phase,
    pub message: Option<String>,
    /// While recognizing: how long it is expected to take, for the bubble's progress ring.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_ms: Option<u64>,
}

struct Inner {
    phase: Phase,
    /// Bumped by every new dictation, so a late timer from an old one changes nothing.
    session: u64,
    recorder: Option<Recorder>,
    target: Option<Target>,
    /// The target's program ("discord.exe"), for its profile.
    program: Option<String>,
    started: Option<Instant>,
    /// Stop arrived while the microphone was still opening (a very short hold).
    stop_requested: bool,
    /// Recognizes finished parts while the user is still speaking.
    chunks: Option<ChunkJob>,
}

pub struct Dictation {
    inner: Mutex<Inner>,
}

impl Default for Dictation {
    fn default() -> Self {
        Dictation { inner: Mutex::new(Inner { phase: Phase::Idle, session: 0, recorder: None, target: None, program: None, started: None, stop_requested: false, chunks: None }) }
    }
}

impl Dictation {
    pub fn is_recording(&self) -> bool {
        self.inner.lock().unwrap().phase == Phase::Recording
    }

    /// Recording or working on the result.
    pub fn is_active(&self) -> bool {
        matches!(self.inner.lock().unwrap().phase, Phase::Recording | Phase::Transcribing | Phase::Inserting)
    }

    pub fn phase(&self) -> Phase {
        self.inner.lock().unwrap().phase
    }
}

fn announce(app: &AppHandle, phase: Phase, message: Option<String>) {
    let _ = app.emit("dictation-state", StateEvent { state: phase, message, expected_ms: None });
    crate::bubble::sync_visibility(app, !matches!(phase, Phase::Idle));
}

/// Leaves Done/Error for Idle after a moment, unless a new dictation started meanwhile.
fn settle(app: &AppHandle, session: u64, after: Duration) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(after);
        let core = app.state::<Core>();
        let mut inner = core.dictation.inner.lock().unwrap();
        if inner.session == session && matches!(inner.phase, Phase::Done | Phase::Error) {
            inner.phase = Phase::Idle;
            drop(inner);
            announce(&app, Phase::Idle, None);
        }
    });
}

fn fail(app: &AppHandle, session: u64, message: String, notify: bool) {
    let core = app.state::<Core>();
    {
        let mut inner = core.dictation.inner.lock().unwrap();
        // A dictation cancelled or replaced meanwhile fails silently.
        if inner.session != session || !matches!(inner.phase, Phase::Recording | Phase::Transcribing | Phase::Inserting) {
            return;
        }
        inner.phase = Phase::Error;
    }
    crate::diag::warn(format!("dictation failed: {message}"));
    announce(app, Phase::Error, Some(message.clone()));
    if notify {
        crate::notify(app, "Amber", &message);
    }
    settle(app, session, Duration::from_millis(2500));
}

pub fn start(app: &AppHandle) {
    let core = app.state::<Core>();
    let settings = core.settings();
    let session = {
        let mut inner = core.dictation.inner.lock().unwrap();
        if matches!(inner.phase, Phase::Recording | Phase::Transcribing | Phase::Inserting) {
            return;
        }
        inner.session += 1;
        inner.phase = Phase::Recording;
        inner.target = insert::remember();
        inner.program = inner.target.and_then(|t| t.program());
        inner.started = Some(Instant::now());
        inner.stop_requested = false;
        inner.session
    };
    // The bubble answers at once; the microphone opens in the background.
    announce(app, Phase::Recording, None);

    let app = app.clone();
    std::thread::spawn(move || {
        let core = app.state::<Core>();
        // The microphone meter in the settings would hold the device open twice.
        drop(core.mic_test.lock().unwrap().take());

        let level_app = app.clone();
        let opened = Recorder::start(&settings.microphone, move |level| {
            let _ = level_app.emit_to(crate::bubble::BUBBLE, "level", serde_json::json!({ "level": level }));
        });
        let recorder = match opened {
            Ok(r) => r,
            Err(e) => return fail(&app, session, e, true),
        };
        if recorder.fell_back {
            crate::diag::warn(format!("microphone \"{}\" not found, using \"{}\"", settings.microphone, recorder.device));
        }
        let stop_now = {
            let mut inner = core.dictation.inner.lock().unwrap();
            if inner.session != session || inner.phase != Phase::Recording {
                // Cancelled while the microphone was opening.
                return;
            }
            if settings.engine == EngineKind::Local && settings.local.chunking {
                inner.chunks = Some(ChunkJob::start(&app, &settings, recorder.shared_samples(), recorder.rate));
            }
            inner.recorder = Some(recorder);
            std::mem::take(&mut inner.stop_requested)
        };
        if stop_now {
            stop(&app);
            return;
        }

        // The local model loads while the user speaks, not after - on its own thread, so the
        // time limit below counts from the start of the recording.
        if settings.engine == EngineKind::Local && !core.whisper.is_ready() {
            let app = app.clone();
            std::thread::spawn(move || {
                let _ = app.state::<Core>().whisper.ensure(&app);
            });
        }

        // Hard limit: a forgotten toggle does not record forever.
        std::thread::sleep(Duration::from_secs(capture::MAX_SECONDS as u64));
        let same = {
            let inner = core.dictation.inner.lock().unwrap();
            inner.session == session && inner.phase == Phase::Recording
        };
        if same {
            crate::diag::warn("dictation reached the time limit");
            stop(&app);
        }
    });
}

/// Ends the recording and sends it off.
pub fn stop(app: &AppHandle) {
    let core = app.state::<Core>();
    let (recorder, target, program, session, started, chunks) = {
        let mut inner = core.dictation.inner.lock().unwrap();
        if inner.phase != Phase::Recording {
            return;
        }
        let Some(recorder) = inner.recorder.take() else {
            inner.stop_requested = true;
            return;
        };
        inner.phase = Phase::Transcribing;
        (recorder, inner.target.take(), inner.program.take(), inner.session, inner.started.take(), inner.chunks.take())
    };
    announce(app, Phase::Transcribing, None);
    let app = app.clone();
    std::thread::spawn(move || {
        let recording = recorder.finish();
        // Waits for a part still being recognized; what is left after it is the tail.
        let parts = chunks.map(ChunkJob::finish);
        let held_ms = started.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
        run(&app, session, recording, parts, target, program, held_ms);
    });
}

/// Throws the recording away (the press turned into a drag).
pub fn cancel(app: &AppHandle) {
    let core = app.state::<Core>();
    let recorder = {
        let mut inner = core.dictation.inner.lock().unwrap();
        if inner.phase != Phase::Recording {
            return;
        }
        inner.phase = Phase::Idle;
        inner.target = None;
        if let Some(job) = inner.chunks.take() {
            job.abandon();
        }
        inner.recorder.take()
    };
    drop(recorder);
    announce(app, Phase::Idle, None);
}

/// Start if idle, stop if recording - for the tray menu.
pub fn toggle(app: &AppHandle) {
    if app.state::<Core>().dictation.is_recording() {
        stop(app);
    } else {
        start(app);
    }
}

fn run(app: &AppHandle, session: u64, recording: capture::Recording, parts: Option<Parts>, target: Option<Target>, program: Option<String>, held_ms: u64) {
    let core = app.state::<Core>();
    // What the user waits: from the stop to the text being there.
    let stopped = Instant::now();
    let settings = core.settings();
    if recording.interrupted && recording.samples.is_empty() {
        return fail(app, session, "The microphone stopped delivering sound.".into(), true);
    }
    let prepared = match audio::prepare(&recording.samples, recording.rate) {
        Ok(p) => p,
        Err(Rejected::Silence) => {
            crate::diag::info(format!("nothing heard in {held_ms} ms"));
            // Not worth a notification: the red bubble says enough.
            return fail(app, session, "Nothing heard - is the right microphone selected?".into(), false);
        }
    };

    // Recognized while speaking: only the tail after the last cut is left.
    let parts = parts.filter(|p| !p.failed && p.cut_at > 0 && !p.texts.is_empty() && p.cut_at <= recording.samples.len());
    let tail = parts.as_ref().map(|p| audio::prepare(&recording.samples[p.cut_at..], recording.rate));

    // The bubble's ring fills against what recognition is expected to take here.
    let (estimate_key, typical) = match settings.engine {
        EngineKind::Local => (estimate::key("local", &settings.local.runtime, &settings.local.model), 0.1),
        EngineKind::Server => (estimate::key(&settings.server.provider, "", &settings.server.model), 0.05),
    };
    let left_ms = match &tail {
        Some(Ok(t)) => t.spoken_ms,
        Some(Err(_)) => 0,
        None => prepared.spoken_ms,
    };
    let expected = core.estimates.expected_ms(&estimate_key, left_ms, typical, OVERHEAD_MS);
    let _ = app.emit("dictation-state", StateEvent { state: Phase::Transcribing, message: None, expected_ms: Some(expected) });

    let recognized = match (&parts, tail) {
        (Some(p), Some(Ok(t))) => {
            let options = TranscribeOptions::new(&settings.language, &chunking::prompt(&settings.vocabulary, &p.texts));
            recognize(app, &settings, &t.wav, &options, t.spoken_ms).map(|(raw, engine, ms)| {
                let mut all = p.texts.clone();
                all.push(raw);
                (chunking::join(&all), engine, ms)
            })
        }
        // The tail held no speech: the parts are the whole dictation.
        (Some(p), Some(Err(_))) => Ok((chunking::join(&p.texts), "local (parts)".to_string(), 0)),
        _ => {
            let options = TranscribeOptions::new(&settings.language, &settings.vocabulary);
            recognize(app, &settings, &prepared.wav, &options, prepared.spoken_ms)
        }
    };
    if let Some(p) = &parts {
        crate::diag::info(format!("recognized while speaking: {} parts, then the last {} ms", p.texts.len(), left_ms));
    }
    let (raw, engine, recognize_ms) = match recognized {
        Ok(r) => r,
        Err(message) => {
            // Kept in memory for a few minutes: one click recognizes it again.
            core.memory.keep_failed(prepared.wav, prepared.spoken_ms, program);
            return fail(app, session, format!("{message} The recording is kept - tray → \"Recognize again\"."), true);
        }
    };
    let profile = program.as_deref().and_then(|p| amber_core::profiles::find(&settings.profiles, p));
    let (cleaned, _skipped) = crate::llm::tidy(app, &settings, &text::clean(&raw), profile);
    if cleaned.is_empty() {
        crate::diag::info(format!("{} ms of audio, nothing recognized", prepared.spoken_ms));
        return fail(app, session, "Nothing recognized.".into(), false);
    }

    {
        let mut inner = core.dictation.inner.lock().unwrap();
        if inner.session != session {
            return;
        }
        inner.phase = Phase::Inserting;
    }
    announce(app, Phase::Inserting, None);
    let final_text = text::compose(&cleaned, settings.insert.trailing_space);
    let outcome = insert::insert(&final_text, &settings.insert, target);

    let what = if settings.log_text { format!("\"{cleaned}\"") } else { format!("{} words", text::words(&cleaned)) };
    let inserted = matches!(outcome, Ok(Outcome::Pasted | Outcome::Typed));
    core.memory.remember(
        &settings,
        &cleaned,
        program.as_deref().unwrap_or(""),
        &engine,
        inserted,
        prepared.spoken_ms,
        stopped.elapsed().as_millis() as u64,
    );
    match outcome {
        Ok(Outcome::OnClipboard(message)) => {
            crate::diag::warn(format!("{what} left on the clipboard: {message}"));
            fail(app, session, message, true);
        }
        Ok(how) => {
            crate::diag::info(format!(
                "{what} · {} ms audio · {engine} {recognize_ms} ms · {how:?} into {}",
                prepared.spoken_ms,
                program.as_deref().unwrap_or("?")
            ));
            {
                let mut inner = core.dictation.inner.lock().unwrap();
                if inner.session != session {
                    return;
                }
                inner.phase = Phase::Done;
            }
            announce(app, Phase::Done, Some(cleaned));
            settle(app, session, Duration::from_millis(900));
        }
        Err(e) => fail(app, session, format!("The text could not be inserted ({e}). It is in the history."), true),
    }
}

/// Recognizes the last failed recording once more (with the engine set now) and puts the
/// text on the clipboard - by now the window it was meant for may be anywhere.
pub fn recognize_again(app: &AppHandle) -> Result<String, String> {
    let core = app.state::<Core>();
    let failed = core.memory.take_failed().ok_or("No failed recording from the last ten minutes.")?;
    let settings = core.settings();
    let started = Instant::now();
    let options = TranscribeOptions::new(&settings.language, &settings.vocabulary);
    let (raw, engine, _) = match recognize(app, &settings, &failed.wav, &options, failed.spoken_ms) {
        Ok(r) => r,
        Err(e) => {
            // Still there for another try (another engine, the key fixed).
            core.memory.keep_failed(failed.wav, failed.spoken_ms, failed.program);
            return Err(e);
        }
    };
    let profile = failed.program.as_deref().and_then(|p| amber_core::profiles::find(&settings.profiles, p));
    let (text, _) = crate::llm::tidy(app, &settings, &text::clean(&raw), profile);
    if text.is_empty() {
        return Err("Nothing recognized.".into());
    }
    insert::to_clipboard(&text)?;
    core.memory.remember(&settings, &text, failed.program.as_deref().unwrap_or(""), &engine, false, failed.spoken_ms, started.elapsed().as_millis() as u64);
    crate::diag::info(format!("recognized again: {} words, on the clipboard", text::words(&text)));
    Ok(text)
}

/// What was recognized while the user was still speaking.
pub struct Parts {
    texts: Vec<String>,
    /// Where the tail begins, in samples of the recording.
    cut_at: usize,
    /// A part failed: the parts are worthless as a set, the whole recording goes instead.
    failed: bool,
}

/// Watches the growing recording, cuts it at real pauses (`chunking::ChunkPlanner`) and
/// recognizes each finished part in order, with the text so far as its prompt.
struct ChunkJob {
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<Parts>,
}

impl ChunkJob {
    fn start(app: &AppHandle, settings: &Settings, samples: Arc<Mutex<Vec<f32>>>, rate: u32) -> ChunkJob {
        // Short parts only pay off where a short part is also cheap: models tuned to a short
        // encoder context. A stock model works through 30 s for any part.
        let short = local::model(&settings.local.model).is_some_and(|m| m.short_context);
        let min_ms = if short { 4_000 } else { 20_000 };
        let stop = Arc::new(AtomicBool::new(false));
        let (app, settings, flag) = (app.clone(), settings.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            let mut planner = ChunkPlanner::new(rate, min_ms);
            let mut vad = FrameVad::default();
            let frame = planner.frame_samples();
            let mut parts = Parts { texts: Vec::new(), cut_at: 0, failed: false };
            let mut pos = 0;
            loop {
                let stopping = flag.load(Ordering::SeqCst);
                let fresh: Vec<f32> = {
                    let all = samples.lock().unwrap();
                    let whole_frames = (all.len().saturating_sub(pos)) / frame * frame;
                    all[pos..pos + whole_frames].to_vec()
                };
                for f in fresh.chunks_exact(frame) {
                    pos += frame;
                    let rms = audio::rms(f);
                    let Some(cut) = planner.on_frame(pos, rms, vad.is_speech(rms)) else { continue };
                    let slice: Vec<f32> = samples.lock().unwrap()[cut.start..cut.end].to_vec();
                    let Ok(part) = audio::prepare(&slice, rate) else {
                        // A cough, a breath: nothing to recognize, but the tail starts after it.
                        parts.cut_at = cut.end;
                        continue;
                    };
                    let options = TranscribeOptions::new(&settings.language, &chunking::prompt(&settings.vocabulary, &parts.texts));
                    match transcriber(&app, &settings).and_then(|p| p.transcriber.transcribe(&part.wav, &options).map_err(|e| e.message)) {
                        Ok(text) => {
                            parts.texts.push(text::clean(&text));
                            parts.cut_at = cut.end;
                        }
                        Err(e) => {
                            crate::diag::warn(format!("a part failed ({e}), the whole recording will be recognized"));
                            parts.failed = true;
                            return parts;
                        }
                    }
                }
                if stopping {
                    return parts;
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        });
        ChunkJob { stop, handle }
    }

    /// The parts recognized until now - waits for one still being recognized.
    fn finish(self) -> Parts {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.join().unwrap_or(Parts { texts: Vec::new(), cut_at: 0, failed: true })
    }

    /// The recording was thrown away: the job ends on its own, nobody waits for it.
    fn abandon(self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// An engine ready for one request, with the names it is logged and measured under.
pub struct Picked {
    pub transcriber: Box<dyn Transcriber>,
    label: String,
    estimate_key: String,
}

/// Fixed cost per request on top of the audio: a round trip, a context switch.
const OVERHEAD_MS: u64 = 200;

/// Recognizes [wav] with the engine the settings ask for. A local server that went away
/// mid-request gets one more try; a server that cannot be reached hands the recording to a
/// downloaded local model (if allowed) - never the other way round. Learns how long it took.
fn recognize(app: &AppHandle, settings: &Settings, wav: &[u8], options: &TranscribeOptions, spoken_ms: u64) -> Result<(String, String, u128), String> {
    let core = app.state::<Core>();
    let mut picked = transcriber(app, settings)?;
    let mut t0 = Instant::now();
    let mut result = picked.transcriber.transcribe(wav, options);

    if let Err(e) = &result {
        if settings.engine == EngineKind::Local && e.kind == ErrorKind::Unreachable {
            // Restarted for another model, or crashed.
            crate::diag::warn("local engine went away during the request, trying again");
            picked = transcriber(app, settings)?;
            t0 = Instant::now();
            result = picked.transcriber.transcribe(wav, options);
        } else if settings.engine == EngineKind::Server && e.kind.service_unavailable() && settings.server.local_fallback && local_ready(&core, settings) {
            crate::diag::warn(format!("server unavailable ({}), recognizing on this PC", e.detail));
            let server_error = e.message.clone();
            let local_result = local(app).and_then(|p| {
                let t = Instant::now();
                let r = p.transcriber.transcribe(wav, options).map_err(|e| e.message);
                Ok((p, t, r))
            });
            match local_result {
                Ok((p, t, Ok(raw))) => {
                    crate::notify(app, "Amber", "The server could not be reached - recognized on this PC instead.");
                    unload_after_fallback(app);
                    picked = p;
                    t0 = t;
                    result = Ok(raw);
                }
                // The server's failure is the one the user needs to hear about.
                Ok((_, _, Err(local))) | Err(local) => {
                    crate::diag::warn(format!("local fallback failed too: {local}"));
                    return Err(server_error);
                }
            }
        }
    }
    match result {
        Ok(raw) => {
            let took = t0.elapsed().as_millis();
            core.estimates.learn(&picked.estimate_key, spoken_ms, took as u64, OVERHEAD_MS);
            Ok((raw, picked.label, took))
        }
        Err(e) => {
            crate::diag::warn(format!("engine error: {}", e.detail));
            Err(e.message)
        }
    }
}

/// The model loaded for a fallback holds memory the server engine does not need: unloaded
/// after ten minutes without another fallback.
fn unload_after_fallback(app: &AppHandle) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let this = LAST.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(600));
        let core = app.state::<Core>();
        if LAST.load(Ordering::SeqCst) == this && core.settings().engine == EngineKind::Server && !core.dictation.is_active() {
            core.whisper.stop(&app);
        }
    });
}

/// Whether the local engine could take over: runtime and model downloaded.
fn local_ready(core: &Core, s: &Settings) -> bool {
    let layout = &core.paths.layout;
    local::runtime(&s.local.runtime).is_some_and(|rt| layout.runtime_installed(rt))
        && local::model(&s.local.model).is_some_and(|m| layout.model_installed(m))
}

/// The local whisper server - with a short encoder context when the model it runs (which may
/// still be the previous one while the chosen one downloads) was tuned for it.
fn local(app: &AppHandle) -> Result<Picked, String> {
    let served = app.state::<Core>().whisper.ensure(app)?;
    let slack = local::model(&served.model).filter(|m| m.short_context).map(|m| m.context_slack);
    Ok(Picked {
        transcriber: Box::new(WhisperServerTranscriber::new(&served.url, slack)),
        label: format!("local {}/{}", served.runtime, served.model),
        estimate_key: estimate::key("local", &served.runtime, &served.model),
    })
}

/// The engine the settings ask for, ready to use - or why not.
pub fn transcriber(app: &AppHandle, settings: &Settings) -> Result<Picked, String> {
    let core = app.state::<Core>();
    match settings.engine {
        EngineKind::Local => local(app),
        EngineKind::Server => {
            let provider = providers::by_id(&settings.server.provider).ok_or("Unknown server provider.")?;
            let base = providers::effective_base_url(provider, &settings.server.base_url);
            providers::check_base_url(&base)?;
            let key = core.key(provider.id).unwrap_or_default();
            if provider.needs_key && key.is_empty() {
                return Err(format!("No API key for {} yet - add it in the settings.", provider.name));
            }
            Ok(Picked {
                transcriber: Box::new(OpenAiTranscriber::new(&base, &key, &settings.server.model)),
                label: format!("{} {}", provider.id, settings.server.model),
                estimate_key: estimate::key(provider.id, "", &settings.server.model),
            })
        }
    }
}

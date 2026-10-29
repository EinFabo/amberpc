//! One dictation at a time: Idle -> Recording -> Transcribing -> Inserting -> Done -> Idle,
//! or -> Error -> Idle. Every step is announced to the bubble and the settings window.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use amber_core::audio::{self, Rejected};
use amber_core::engine::{OpenAiTranscriber, TranscribeOptions, Transcriber, WhisperServerTranscriber};
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
pub struct StateEvent {
    pub state: Phase,
    pub message: Option<String>,
}

struct Inner {
    phase: Phase,
    /// Bumped by every new dictation, so a late timer from an old one changes nothing.
    session: u64,
    recorder: Option<Recorder>,
    target: Option<Target>,
    started: Option<Instant>,
    /// Stop arrived while the microphone was still opening (a very short hold).
    stop_requested: bool,
}

pub struct Dictation {
    inner: Mutex<Inner>,
}

impl Default for Dictation {
    fn default() -> Self {
        Dictation { inner: Mutex::new(Inner { phase: Phase::Idle, session: 0, recorder: None, target: None, started: None, stop_requested: false }) }
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
    let _ = app.emit("dictation-state", StateEvent { state: phase, message });
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
    let (recorder, target, session, started) = {
        let mut inner = core.dictation.inner.lock().unwrap();
        if inner.phase != Phase::Recording {
            return;
        }
        let Some(recorder) = inner.recorder.take() else {
            inner.stop_requested = true;
            return;
        };
        inner.phase = Phase::Transcribing;
        (recorder, inner.target.take(), inner.session, inner.started.take())
    };
    announce(app, Phase::Transcribing, None);
    let app = app.clone();
    std::thread::spawn(move || {
        let recording = recorder.finish();
        let held_ms = started.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
        run(&app, session, recording, target, held_ms);
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

fn run(app: &AppHandle, session: u64, recording: capture::Recording, target: Option<Target>, held_ms: u64) {
    let core = app.state::<Core>();
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

    let t0 = Instant::now();
    let transcriber = match transcriber(app, &settings) {
        Ok(t) => t,
        Err(e) => return fail(app, session, e, true),
    };
    let options = TranscribeOptions::new(&settings.language, &settings.vocabulary);
    let raw = match transcriber.transcribe(&prepared.wav, &options) {
        Ok(raw) => raw,
        Err(e) => {
            crate::diag::warn(format!("engine error: {}", e.detail));
            return fail(app, session, e.message, true);
        }
    };
    let recognize_ms = t0.elapsed().as_millis();
    let cleaned = text::clean(&raw);
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
    let engine = engine_label(&settings);
    match outcome {
        Ok(Outcome::OnClipboard(message)) => {
            crate::diag::warn(format!("{what} left on the clipboard: target window is elevated"));
            fail(app, session, message, true);
        }
        Ok(how) => {
            crate::diag::info(format!(
                "{what} · {} ms audio · {engine} {recognize_ms} ms · {how:?}",
                prepared.spoken_ms
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
        Err(e) => fail(app, session, format!("The text could not be inserted: {e}"), true),
    }
}

fn engine_label(s: &Settings) -> String {
    match s.engine {
        EngineKind::Local => format!("local {}/{}", s.local.runtime, s.local.model),
        EngineKind::Server => format!("{} {}", s.server.provider, s.server.model),
    }
}

/// The engine the settings ask for, ready to use - or why not.
pub fn transcriber(app: &AppHandle, settings: &Settings) -> Result<Box<dyn Transcriber>, String> {
    let core = app.state::<Core>();
    match settings.engine {
        EngineKind::Local => {
            let url = core.whisper.ensure(app)?;
            Ok(Box::new(WhisperServerTranscriber::new(&url)))
        }
        EngineKind::Server => {
            let provider = providers::by_id(&settings.server.provider).ok_or("Unknown server provider.")?;
            let base = providers::effective_base_url(provider, &settings.server.base_url);
            providers::check_base_url(&base)?;
            let key = core.key(provider.id).unwrap_or_default();
            if provider.needs_key && key.is_empty() {
                return Err(format!("No API key for {} yet - add it in the settings.", provider.name));
            }
            Ok(Box::new(OpenAiTranscriber::new(&base, &key, &settings.server.model)))
        }
    }
}

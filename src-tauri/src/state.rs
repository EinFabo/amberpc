//! What the whole app shares: settings, paths, keys, and the parts that hold state.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use amber_core::gesture::{Gesture, Mode};
use amber_core::local::Layout;
use amber_core::settings::Settings;

use crate::capture::Recorder;
use crate::dictation::Dictation;
use crate::whisper::Whisper;

pub struct Paths {
    pub settings: PathBuf,
    pub secrets: PathBuf,
    pub logs: PathBuf,
    pub layout: Layout,
}

pub struct Core {
    pub paths: Paths,
    settings: RwLock<Settings>,
    keys: Mutex<HashMap<String, String>>,
    pub dictation: Dictation,
    pub whisper: Whisper,
    pub hotkey_gesture: Mutex<Gesture>,
    pub pointer_gesture: Mutex<Gesture>,
    /// Running downloads by id, with their cancel switch.
    pub downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// The settings window's microphone meter.
    pub mic_test: Mutex<Option<(Recorder, Instant)>>,
    pub phone: crate::phone::PhoneApi,
    /// The smart cleanup's local model.
    pub llm: crate::llm::Llm,
    epoch: Instant,
}

impl Core {
    pub fn new(paths: Paths) -> Self {
        let (settings, problem) = Settings::load(&paths.settings);
        if let Some(problem) = problem {
            crate::diag::warn(problem);
        }
        let keys = crate::secrets::load(&paths.secrets);
        let mode = settings.hotkey.mode;
        Core {
            paths,
            settings: RwLock::new(settings),
            keys: Mutex::new(keys),
            dictation: Dictation::default(),
            whisper: Whisper::default(),
            hotkey_gesture: Mutex::new(Gesture::new(mode)),
            // The bubble always offers both: a click toggles, a long press talks.
            pointer_gesture: Mutex::new(Gesture::for_pointer(Mode::Both)),
            downloads: Mutex::new(HashMap::new()),
            mic_test: Mutex::new(None),
            phone: crate::phone::PhoneApi::default(),
            llm: crate::llm::Llm::default(),
            epoch: Instant::now(),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    /// Changes, sanitizes and saves the settings; returns the new ones.
    pub fn update_settings(&self, change: impl FnOnce(&mut Settings)) -> Settings {
        let mut s = self.settings.write().unwrap();
        let mut next = s.clone();
        change(&mut next);
        let next = next.sanitized();
        if let Err(e) = next.save(&self.paths.settings) {
            crate::diag::error(format!("settings not saved: {e}"));
        }
        *s = next.clone();
        next
    }

    pub fn key(&self, provider: &str) -> Option<String> {
        self.keys.lock().unwrap().get(provider).cloned().filter(|k| !k.is_empty())
    }

    pub fn has_key(&self, provider: &str) -> bool {
        self.key(provider).is_some()
    }

    pub fn set_key(&self, provider: &str, key: Option<String>) -> Result<(), String> {
        let mut keys = self.keys.lock().unwrap();
        match key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
            Some(k) => keys.insert(provider.to_string(), k),
            None => keys.remove(provider),
        };
        crate::secrets::save(&self.paths.secrets, &keys)
    }

    /// Milliseconds since start, the gesture machine's clock.
    pub fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

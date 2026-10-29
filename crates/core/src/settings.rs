//! Everything the user can set, with defaults that work on first start. API keys are not in
//! here - they live encrypted next to it (see the app's `secrets`).

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

use crate::gesture::Mode;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub engine: EngineKind,
    pub local: LocalSettings,
    pub server: ServerSettings,
    /// "auto" or an ISO 639-1 code ("de"). A fixed language makes Whisper translate
    /// everything else into it, so "auto" is the default.
    pub language: String,
    /// Names and jargon, handed to the engine as a prompt.
    pub vocabulary: String,
    /// Device name; empty means the system's default microphone.
    pub microphone: String,
    pub hotkey: HotkeySettings,
    pub bubble: BubbleSettings,
    pub insert: InsertSettings,
    /// Short sounds when a dictation starts and ends.
    pub sounds: bool,
    pub autostart: bool,
    pub auto_update: bool,
    /// Keep the recognized text in the diagnostics log. Off: only lengths and timings.
    pub log_text: bool,
    /// Set once the first-run setup has been seen.
    pub onboarded: bool,
    /// Amber on the phone may use this PC as its server.
    pub phone: PhoneSettings,
    pub cleanup: CleanupSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            engine: EngineKind::Local,
            local: LocalSettings::default(),
            server: ServerSettings::default(),
            language: "auto".into(),
            vocabulary: String::new(),
            microphone: String::new(),
            hotkey: HotkeySettings::default(),
            bubble: BubbleSettings::default(),
            insert: InsertSettings::default(),
            sounds: true,
            autostart: true,
            auto_update: true,
            log_text: false,
            onboarded: false,
            phone: PhoneSettings::default(),
            cleanup: CleanupSettings::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EngineKind {
    Local,
    Server,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LocalSettings {
    /// "cpu" or "cuda", see `local::RUNTIMES`.
    pub runtime: String,
    /// A model id from `local::MODELS`.
    pub model: String,
    /// 0 = choose from the CPU.
    pub threads: u32,
    /// Start the local engine with the app, so the first dictation does not wait for loading.
    pub keep_loaded: bool,
}

impl Default for LocalSettings {
    fn default() -> Self {
        LocalSettings { runtime: "cpu".into(), model: "futo-multilingual-244".into(), threads: 0, keep_loaded: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerSettings {
    /// "groq", "openai" or "custom", see `providers`.
    pub provider: String,
    /// Only used for "custom"; known providers have a fixed address.
    pub base_url: String,
    pub model: String,
    /// A server that cannot be reached hands the recording to a downloaded local model -
    /// never the other way round, so the voice only ever stays more private.
    pub local_fallback: bool,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings { provider: "groq".into(), base_url: String::new(), model: "whisper-large-v3-turbo".into(), local_fallback: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct HotkeySettings {
    /// Tauri accelerator syntax, e.g. "Ctrl+Alt+Space". Empty = no hotkey.
    pub dictate: String,
    pub mode: Mode,
    /// Shows or hides the bubble. Empty = none.
    pub toggle_bubble: String,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        HotkeySettings { dictate: "Ctrl+Alt+Space".into(), mode: Mode::Both, toggle_bubble: String::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BubbleVisibility {
    Always,
    WhileDictating,
    Hidden,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    Left,
    Right,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct BubbleSettings {
    pub visibility: BubbleVisibility,
    pub edge: Edge,
    /// Vertical position as a fraction of the monitor's work area, 0 = top.
    pub y: f64,
    /// Monitor the bubble sits on, by name; empty = primary.
    pub monitor: String,
    /// Diameter in logical pixels.
    pub size: u32,
    /// 0.3..1
    pub opacity: f64,
}

impl Default for BubbleSettings {
    fn default() -> Self {
        BubbleSettings {
            visibility: BubbleVisibility::Always,
            edge: Edge::Right,
            y: 0.62,
            monitor: String::new(),
            size: 56,
            opacity: 1.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InsertMethod {
    /// Clipboard + Ctrl+V; works almost everywhere, fast for long text.
    Paste,
    /// Unicode keystrokes; leaves the clipboard alone, slower, some apps drop characters.
    Type,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct InsertSettings {
    pub method: InsertMethod,
    /// After pasting, put back the text that was on the clipboard before.
    pub restore_clipboard: bool,
    /// A space after the text, so the next dictation does not stick to it.
    pub trailing_space: bool,
}

impl Default for InsertSettings {
    fn default() -> Self {
        InsertSettings { method: InsertMethod::Paste, restore_clipboard: true, trailing_space: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PhoneSettings {
    /// Off until the user turns it on: it opens a port to the home network.
    pub enabled: bool,
    pub port: u16,
    /// The PC's cleanup (fillers, smart cleanup) also for what the phone dictates.
    pub cleanup: bool,
}

impl Default for PhoneSettings {
    fn default() -> Self {
        PhoneSettings { enabled: false, port: crate::api::DEFAULT_PORT, cleanup: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CleanupSettings {
    /// Drop "äh", "ähm" and the like - rules, local, instant.
    pub fillers: bool,
    /// The commas German needs before "dass", "weil", "um ... zu" (see `commas`).
    pub german_commas: bool,
    /// Let a chat model tidy up: self-corrections, false starts, punctuation.
    pub smart: bool,
    /// Where that model runs.
    pub engine: EngineKind,
    /// "cpu" or "cuda", see `local::LLM_RUNTIMES`.
    pub local_runtime: String,
    /// See `local::LLM_MODELS`.
    pub local_model: String,
    /// "groq", "openai" or "custom"; the key is the one the speech engine uses.
    pub provider: String,
    /// Only for "custom".
    pub base_url: String,
    pub model: String,
}

impl Default for CleanupSettings {
    fn default() -> Self {
        CleanupSettings {
            fillers: true,
            german_commas: true,
            smart: false,
            engine: EngineKind::Local,
            local_runtime: "cuda".into(),
            local_model: crate::local::LLM_MODELS[0].id.into(),
            provider: "groq".into(),
            base_url: String::new(),
            model: "openai/gpt-oss-20b".into(),
        }
    }
}

pub const BUBBLE_SIZES: std::ops::RangeInclusive<u32> = 40..=96;

impl Settings {
    /// Values out of range are pulled back in, so a hand-edited file cannot break the app.
    pub fn sanitized(mut self) -> Self {
        let d = Settings::default();
        if crate::local::runtime(&self.local.runtime).is_none() {
            self.local.runtime = d.local.runtime;
        }
        if crate::local::model(&self.local.model).is_none() {
            self.local.model = d.local.model;
        }
        self.local.threads = self.local.threads.min(64);
        if crate::providers::by_id(&self.server.provider).is_none() {
            self.server.provider = d.server.provider.clone();
        }
        if self.server.model.trim().is_empty() {
            if let Some(p) = crate::providers::by_id(&self.server.provider) {
                self.server.model = p.default_model().to_string();
            }
        }
        let lang = self.language.trim().to_lowercase();
        self.language = if lang.len() == 2 && lang.chars().all(|c| c.is_ascii_lowercase()) { lang } else { "auto".into() };
        self.bubble.size = self.bubble.size.clamp(*BUBBLE_SIZES.start(), *BUBBLE_SIZES.end());
        self.bubble.opacity = if self.bubble.opacity.is_finite() { self.bubble.opacity.clamp(0.3, 1.0) } else { 1.0 };
        self.bubble.y = if self.bubble.y.is_finite() { self.bubble.y.clamp(0.0, 1.0) } else { d.bubble.y };
        self.vocabulary = self.vocabulary.chars().take(800).collect();
        if self.phone.port < 1024 {
            self.phone.port = d.phone.port;
        }
        let c = &mut self.cleanup;
        if crate::local::llm_runtime(&c.local_runtime).is_none() {
            c.local_runtime = d.cleanup.local_runtime.clone();
        }
        if crate::local::llm_model(&c.local_model).is_none() {
            c.local_model = d.cleanup.local_model.clone();
        }
        match crate::providers::by_id(&c.provider) {
            None => {
                c.provider = d.cleanup.provider.clone();
                c.model = d.cleanup.model.clone();
            }
            Some(p) if c.model.trim().is_empty() => c.model = p.chat_models.first().map(|m| m.id.to_string()).unwrap_or_default(),
            Some(_) => {}
        }
        self
    }

    /// Reads the file; a missing file is a first start. A broken one is kept aside as
    /// `settings.broken.json` and the defaults take over, instead of the app not starting.
    pub fn load(path: &Path) -> (Settings, Option<String>) {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), None),
            Err(e) => return (Settings::default(), Some(format!("settings unreadable: {e}"))),
        };
        match serde_json::from_str::<Settings>(&raw) {
            Ok(s) => (s.sanitized(), None),
            Err(e) => {
                let _ = std::fs::rename(path, path.with_file_name("settings.broken.json"));
                (Settings::default(), Some(format!("settings broken, reset to defaults: {e}")))
            }
        }
    }

    /// Write to a temporary file and rename it over the old one: a crash mid-write leaves the
    /// previous settings, never half a file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(serde_json::to_string_pretty(self).expect("settings serialize").as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut s = Settings::default();
        s.engine = EngineKind::Server;
        s.vocabulary = "Tauri, Groq".into();
        s.bubble.edge = Edge::Left;
        s.save(&path).unwrap();
        let (back, err) = Settings::load(&path);
        assert!(err.is_none());
        assert_eq!(back, s);
    }

    #[test]
    fn missing_file_is_a_first_start() {
        let dir = tempfile::tempdir().unwrap();
        let (s, err) = Settings::load(&dir.path().join("settings.json"));
        assert_eq!(s, Settings::default());
        assert!(err.is_none());
        assert!(!s.onboarded);
    }

    #[test]
    fn broken_file_is_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        let (s, err) = Settings::load(&path);
        assert_eq!(s, Settings::default());
        assert!(err.is_some());
        assert!(dir.path().join("settings.broken.json").exists());
    }

    #[test]
    fn partial_and_unknown_fields_are_fine() {
        let s: Settings = serde_json::from_str(r#"{"sounds":false,"futureThing":1,"bubble":{"edge":"left"}}"#).unwrap();
        assert!(!s.sounds);
        assert_eq!(s.bubble.edge, Edge::Left);
        assert_eq!(s.bubble.size, BubbleSettings::default().size);
    }

    #[test]
    fn sanitize_pulls_values_back() {
        let mut s = Settings::default();
        s.bubble.size = 500;
        s.bubble.opacity = 0.0;
        s.bubble.y = f64::NAN;
        s.language = "Deutsch".into();
        s.local.model = "gone".into();
        s.server.provider = "nope".into();
        s.server.model = " ".into();
        s.phone.port = 80;
        s.cleanup.local_model = "gone".into();
        s.cleanup.provider = "openai".into();
        s.cleanup.model = "".into();
        let s = s.sanitized();
        assert_eq!(s.cleanup.local_model, CleanupSettings::default().local_model);
        assert_eq!(s.cleanup.model, "gpt-4.1-nano");
        assert!(s.cleanup.fillers && !s.cleanup.smart, "fillers on, the model only when asked for");
        assert_eq!(s.phone.port, 8765);
        assert!(!s.phone.enabled, "the phone API is off unless turned on");
        assert_eq!(s.bubble.size, 96);
        assert_eq!(s.bubble.opacity, 0.3);
        assert_eq!(s.bubble.y, BubbleSettings::default().y);
        assert_eq!(s.language, "auto");
        assert_eq!(s.local.model, LocalSettings::default().model);
        assert_eq!(s.server.provider, "groq");
        assert_eq!(s.server.model, "whisper-large-v3-turbo");

        let mut s = Settings::default();
        s.language = " DE ".into();
        assert_eq!(s.sanitized().language, "de");
    }
}

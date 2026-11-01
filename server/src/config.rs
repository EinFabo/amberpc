//! The settings, in `amber-server.toml` - written by `setup`, `models`, `cleanup` and
//! `config set`, and fine to edit by hand. It holds the access key: private to its owner.

use std::path::{Path, PathBuf};

use amber_core::api::AccessKey;
use serde::{Deserialize, Serialize};

/// What `config get` and `config set` know.
pub const KEYS: &[&str] =
    &["port", "language", "vocabulary", "threads", "gpu", "rules.fillers", "rules.spoken_punctuation", "rules.german_commas"];

const DEFAULT_SPEECH: &str = "futo-multilingual-244";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub port: u16,
    /// The ways in: each a name, the secret in its address, the models it may use and its
    /// quota (`amber-server key`).
    pub keys: Vec<AccessKey>,
    /// The one key of 0.4.x - read once and turned into the key "default".
    #[serde(default, rename = "key", skip_serializing)]
    pub legacy_key: String,
    /// The speech model a request gets that names none (an id of `local::MODELS`).
    pub speech: String,
    /// The cleanup model likewise (`local::LLM_MODELS`), none for no cleanup.
    pub cleanup: Option<String>,
    /// "auto" or an ISO 639-1 code.
    pub language: String,
    /// Names and terms the recognition should expect.
    pub vocabulary: String,
    /// 0: half the logical cores, 2 to 8 (more slows whisper.cpp down).
    pub threads: u32,
    pub gpu: Gpu,
    pub rules: RulesConfig,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Gpu {
    /// The GPU if this build and the machine have one.
    #[serde(alias = "on")]
    Auto,
    Off,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct RulesConfig {
    pub fillers: bool,
    pub spoken_punctuation: bool,
    pub german_commas: bool,
}

impl Default for RulesConfig {
    fn default() -> Self {
        RulesConfig { fillers: true, spoken_punctuation: true, german_commas: true }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: amber_core::api::DEFAULT_PORT,
            keys: Vec::new(),
            legacy_key: String::new(),
            speech: DEFAULT_SPEECH.into(),
            cleanup: None,
            language: "auto".into(),
            vocabulary: String::new(),
            threads: 0,
            gpu: Gpu::Auto,
            rules: RulesConfig::default(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("amber-server").join("amber-server.toml")
    }

    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let config: Config = toml::from_str(&text).map_err(|e| {
            format!("{}: {e}\nFix that line - or delete the file and run: amber-server setup", path.display())
        })?;
        Ok(config.sanitized())
    }

    /// Written to a temporary file next to it first, so a crash never leaves half a config.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().ok_or("no folder for the config")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let temp = path.with_extension("toml.new");
        let _ = std::fs::remove_file(&temp);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Private from the first byte: it holds the access key.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&temp).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&temp, path).map_err(|e| e.to_string())
    }

    /// A config with the one key "default" - what setup makes.
    #[cfg(test)]
    pub fn with_key(secret: &str) -> Config {
        Config { keys: vec![AccessKey::new("default", secret)], ..Config::default() }
    }

    /// Whether setup ran: only it makes the first key.
    pub fn set_up(&self) -> bool {
        !self.keys.is_empty()
    }

    /// A new key [name] for [models] (None: every model) at [per_day] requests a day (None:
    /// the usual); its secret back.
    pub fn add_key(&mut self, name: &str, models: Option<Vec<String>>, per_day: Option<u32>) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a key needs a name".into());
        }
        if self.find_key(name).is_some() {
            return Err(format!("there is a key named {name} already"));
        }
        if let Some(ids) = &models {
            if let Some(bad) = ids.iter().find(|id| amber_core::local::model(id).is_none() && amber_core::local::llm_model(id).is_none()) {
                return Err(format!("{bad} is no model - amber-server models, amber-server cleanup"));
            }
        }
        if per_day == Some(0) {
            return Err("per day: at least 1".into());
        }
        let secret = amber_core::api::new_key();
        self.keys.push(AccessKey { name: name.to_string(), secret: secret.clone(), models, per_day });
        Ok(secret)
    }

    /// A new secret for the key [name]: devices with the old address are locked out.
    pub fn renew_key(&mut self, name: &str) -> Result<String, String> {
        let secret = amber_core::api::new_key();
        let i = self.find_key(name).ok_or_else(|| format!("no key named {name} - amber-server key"))?;
        self.keys[i].secret = secret.clone();
        Ok(secret)
    }

    pub fn remove_key(&mut self, name: &str) -> Result<(), String> {
        let i = self.find_key(name).ok_or_else(|| format!("no key named {name} - amber-server key"))?;
        if self.keys.len() == 1 {
            return Err("that is the last key - nobody could reach the server; add another first, or renew it".into());
        }
        self.keys.remove(i);
        Ok(())
    }

    pub fn find_key(&self, name: &str) -> Option<usize> {
        self.keys.iter().position(|k| k.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Unknown models and impossible values fall back to the defaults.
    pub fn sanitized(mut self) -> Config {
        let d = Config::default();
        if self.keys.is_empty() && !self.legacy_key.trim().is_empty() {
            self.keys.push(AccessKey::new("default", self.legacy_key.trim()));
        }
        self.legacy_key.clear();
        // A key without a name or secret is no way in; a name only once.
        let mut names = std::collections::HashSet::new();
        self.keys.retain(|k| !k.name.trim().is_empty() && !k.secret.trim().is_empty() && names.insert(k.name.trim().to_lowercase()));
        if self.port < 1024 {
            self.port = d.port;
        }
        if amber_core::local::model(&self.speech).is_none() {
            self.speech = d.speech;
        }
        if self.cleanup.as_deref().is_some_and(|id| amber_core::local::llm_model(id).is_none()) {
            self.cleanup = None;
        }
        self.language = language(&self.language).unwrap_or(d.language);
        self.threads = self.threads.min(MAX_THREADS);
        self.vocabulary = self.vocabulary.trim().to_string();
        self
    }

    pub fn get(&self, key: &str) -> Result<String, String> {
        Ok(match key {
            "port" => self.port.to_string(),
            "language" => self.language.clone(),
            "vocabulary" => self.vocabulary.clone(),
            "threads" => self.threads.to_string(),
            "gpu" => if self.gpu == Gpu::Auto { "auto" } else { "off" }.into(),
            "rules.fillers" => self.rules.fillers.to_string(),
            "rules.spoken_punctuation" => self.rules.spoken_punctuation.to_string(),
            "rules.german_commas" => self.rules.german_commas.to_string(),
            _ => return Err(unknown(key)),
        })
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let v = value.trim();
        match key {
            "port" => self.port = v.parse::<u16>().ok().filter(|p| *p >= 1024).ok_or("port: a number from 1024 to 65535")?,
            "language" => self.language = language(v).ok_or("language: auto, a code whisper knows such as de, or several to choose among such as de,en")?,
            "vocabulary" => self.vocabulary = v.to_string(),
            "threads" => {
                self.threads =
                    v.parse().ok().filter(|t| *t <= MAX_THREADS).ok_or("threads: a number up to 256, 0 for half the cores (2 to 8)")?
            }
            "gpu" => {
                self.gpu = match v.to_ascii_lowercase().as_str() {
                    "auto" | "on" => Gpu::Auto,
                    "off" => Gpu::Off,
                    _ => return Err("gpu: auto or off".into()),
                }
            }
            "rules.fillers" => self.rules.fillers = switch(key, v)?,
            "rules.spoken_punctuation" => self.rules.spoken_punctuation = switch(key, v)?,
            "rules.german_commas" => self.rules.german_commas = switch(key, v)?,
            _ => return Err(unknown(key)),
        }
        Ok(())
    }
}

fn unknown(key: &str) -> String {
    format!("unknown setting \"{key}\" - these exist: {}", KEYS.join(", "))
}

fn switch(key: &str, value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(format!("{key}: on or off")),
    }
}

/// "auto", or a code whisper knows, in lower case.
/// "auto", or one or more codes whisper knows ("de", "de,en"); None when any is unknown.
fn language(value: &str) -> Option<String> {
    if value.trim().eq_ignore_ascii_case("auto") {
        return Some("auto".into());
    }
    // Every part must be a code whisper knows; one given twice counts once.
    let mut parts: Vec<String> = value.split(',').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect();
    parts.dedup();
    let codes = crate::speech::whisper_languages(value);
    (!codes.is_empty() && parts.iter().all(|p| crate::speech::whisper_language(p).is_some())).then(|| codes.join(","))
}

const MAX_THREADS: u32 = 256;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_single_key_becomes_the_default_key() {
        // Phones set up with 0.4.x keep working: same secret, now a key named "default".
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("amber-server.toml");
        std::fs::write(&path, "port = 9000\nkey = \"oldsecret1234567890\"\n").unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.keys, [AccessKey::new("default", "oldsecret1234567890")]);
        c.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.lines().any(|l| l.starts_with("key =")), "{text}");
        assert_eq!(Config::load(&path).unwrap(), c);
    }

    #[test]
    fn keys_are_added_renewed_and_removed() {
        let mut c = Config::with_key("defaultsecret123456");
        let secret = c.add_key("Laptop", None, None).unwrap();
        assert_eq!(secret.len(), 20);
        assert!(c.add_key("laptop", None, None).is_err(), "names are unique, whatever the case");
        assert!(c.add_key("  ", None, None).is_err());
        c.add_key("Kind", Some(vec!["futo-multilingual-74".into()]), Some(200)).unwrap();
        assert!(c.add_key("X", Some(vec!["no-such-model".into()]), None).unwrap_err().contains("no-such-model"));
        assert!(c.add_key("Y", None, Some(0)).is_err());
        let renewed = c.renew_key("Laptop").unwrap();
        assert_ne!(renewed, secret);
        c.remove_key("laptop").unwrap();
        c.remove_key("Kind").unwrap();
        assert!(c.remove_key("default").unwrap_err().contains("last"), "the server would be shut");
        assert!(c.remove_key("nobody").is_err());
        assert_eq!(c.keys.len(), 1);
    }

    #[test]
    fn unusable_keys_are_dropped_on_load() {
        let c = Config {
            keys: vec![AccessKey::new("a", "secret1"), AccessKey::new("", "secret2"), AccessKey::new("b", ""), AccessKey::new("A", "secret3")],
            ..Config::default()
        }
        .sanitized();
        assert_eq!(c.keys.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn round_trip_and_sanitize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("amber-server.toml");
        let c = Config::with_key(&"k".repeat(20));
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
        let bad = Config { port: 80, speech: "gone".into(), cleanup: Some("gone".into()), ..Config::default() }.sanitized();
        assert_eq!(bad.port, 8765);
        assert_eq!(bad.speech, "futo-multilingual-244");
        assert_eq!(bad.cleanup, None);
    }

    #[test]
    fn a_hand_edited_config_with_gpu_on_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("amber-server.toml");
        std::fs::write(&path, "gpu = \"on\"\nport = 9000\n").unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!((c.gpu, c.port), (Gpu::Auto, 9000));
        std::fs::write(&path, "port = \"nine\"\n").unwrap();
        let err = Config::load(&path).unwrap_err();
        assert!(err.contains("amber-server.toml") && err.contains("amber-server setup"), "says where and what to do: {err}");
    }

    #[cfg(unix)]
    #[test]
    fn the_config_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("amber-server.toml");
        Config::default().save(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn several_languages_can_be_set() {
        let mut c = Config::default();
        c.set("language", "de, en").unwrap();
        assert_eq!(c.get("language").unwrap(), "de,en");
        assert!(c.set("language", "de,klingon").is_err());
        c.set("language", "de,de,en").unwrap();
        assert_eq!(c.language, "de,en", "a code twice is no unknown one");
        c.set("language", "auto").unwrap();
        assert_eq!(c.language, "auto");
    }

    #[test]
    fn every_key_can_be_read_and_set() {
        let mut c = Config::default();
        c.set("rules.fillers", "false").unwrap();
        assert_eq!(c.get("rules.fillers").unwrap(), "false");
        c.set("gpu", "off").unwrap();
        assert_eq!(c.gpu, Gpu::Off);
        c.set("language", "de").unwrap();
        assert_eq!(c.get("language").unwrap(), "de");
        assert!(c.set("port", "70000").is_err());
        assert!(c.set("threads", "100000").is_err(), "more than any machine has");
        assert!(c.set("language", "xx").is_err(), "not a language whisper knows");
        c.set("language", "DE").unwrap();
        assert_eq!(c.language, "de");
        let unknown = c.set("nope", "1").unwrap_err();
        assert!(unknown.contains("rules.german_commas") && unknown.contains("port"), "{unknown}");
    }
}

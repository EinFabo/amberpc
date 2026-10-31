//! The settings, in `amber-server.toml` - written by `setup`, `models`, `cleanup` and
//! `config set`, and fine to edit by hand. It holds the access key: private to its owner.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What `config get` and `config set` know.
pub const KEYS: &[&str] =
    &["port", "language", "vocabulary", "threads", "gpu", "rules.fillers", "rules.spoken_punctuation", "rules.german_commas"];

const DEFAULT_SPEECH: &str = "futo-multilingual-244";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub port: u16,
    /// The access key in the phone's address.
    pub key: String,
    /// The speech model a request gets that names none (an id of `local::MODELS`).
    pub speech: String,
    /// The cleanup model likewise (`local::LLM_MODELS`), none for no cleanup.
    pub cleanup: Option<String>,
    /// "auto" or an ISO 639-1 code.
    pub language: String,
    /// Names and terms the recognition should expect.
    pub vocabulary: String,
    /// 0: all cores.
    pub threads: u32,
    pub gpu: Gpu,
    pub rules: RulesConfig,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Gpu {
    /// The GPU if this build and the machine have one.
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
            key: String::new(),
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
        let config: Config = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(config.sanitized())
    }

    /// Written to a temporary file next to it first, so a crash never leaves half a config.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().ok_or("no folder for the config")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let temp = path.with_extension("toml.new");
        std::fs::write(&temp, text).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&temp, path).map_err(|e| e.to_string())
    }

    /// Unknown models and impossible values fall back to the defaults.
    pub fn sanitized(mut self) -> Config {
        let d = Config::default();
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
            "language" => self.language = language(v).ok_or("language: auto, or a two-letter code such as de or en")?,
            "vocabulary" => self.vocabulary = v.to_string(),
            "threads" => self.threads = v.parse().map_err(|_| "threads: a number, 0 for all cores")?,
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

/// "auto", or a two-letter code in lower case.
fn language(value: &str) -> Option<String> {
    let v = value.trim().to_ascii_lowercase();
    (v == "auto" || (v.len() == 2 && v.chars().all(|c| c.is_ascii_lowercase()))).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_sanitize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("amber-server.toml");
        let c = Config { key: "k".repeat(20), ..Config::default() };
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
        let bad = Config { port: 80, speech: "gone".into(), cleanup: Some("gone".into()), ..Config::default() }.sanitized();
        assert_eq!(bad.port, 8765);
        assert_eq!(bad.speech, "futo-multilingual-244");
        assert_eq!(bad.cleanup, None);
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
    fn every_key_can_be_read_and_set() {
        let mut c = Config::default();
        c.set("rules.fillers", "false").unwrap();
        assert_eq!(c.get("rules.fillers").unwrap(), "false");
        c.set("gpu", "off").unwrap();
        assert_eq!(c.gpu, Gpu::Off);
        c.set("language", "de").unwrap();
        assert_eq!(c.get("language").unwrap(), "de");
        assert!(c.set("port", "70000").is_err());
        let unknown = c.set("nope", "1").unwrap_err();
        assert!(unknown.contains("rules.german_commas") && unknown.contains("port"), "{unknown}");
    }
}

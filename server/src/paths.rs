//! Where Amber Server keeps its models and the cleanup engine.

use std::path::{Path, PathBuf};

use amber_core::local::{LlmModel, Model};

/// `~/.local/share/amber-server` / `%LOCALAPPDATA%\amber-server`.
pub fn data_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("amber-server")
}

pub fn speech_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn cleanup_dir() -> PathBuf {
    data_dir().join("llm")
}

/// A speech model counts once its file is there under its own name: downloads land as `.part`
/// and are renamed only after their checksum matched.
pub fn model_ready(dir: &Path, m: &Model) -> bool {
    dir.join(m.file).is_file()
}

/// The same for a cleanup model.
pub fn llm_ready(dir: &Path, m: &LlmModel) -> bool {
    dir.join(m.file).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_half_download_is_no_model() {
        let dir = tempfile::tempdir().unwrap();
        let m = amber_core::local::model("futo-multilingual-74").unwrap();
        std::fs::write(dir.path().join(m.file).with_extension("part"), b"x").unwrap();
        assert!(!model_ready(dir.path(), m));
        std::fs::write(dir.path().join(m.file), b"x").unwrap();
        assert!(model_ready(dir.path(), m));
        let llm = amber_core::local::llm_model("qwen3-4b-instruct-2507-q4km").unwrap();
        std::fs::write(dir.path().join(llm.file).with_extension("part"), b"x").unwrap();
        assert!(!llm_ready(dir.path(), llm));
    }
}

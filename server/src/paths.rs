//! Where Amber Server keeps its models and the cleanup engine.

use std::path::Path;

use amber_core::local::Model;

/// A speech model counts once its file is there under its own name: downloads land as `.part`
/// and are renamed only after their checksum matched.
pub fn model_ready(dir: &Path, m: &Model) -> bool {
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
    }
}

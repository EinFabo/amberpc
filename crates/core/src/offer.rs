//! What Amber PC offers the phone to choose from: every downloaded model, and the cloud model
//! set in the app - the PC's own choice marked as the default.

use crate::api::{ModelInfo, ModelKind};
use crate::settings::{EngineKind, Settings};

/// What this PC has ready: downloaded models (with their engine installed), and whether the
/// cloud models set in the app can run (a provider and its key).
pub struct Installed {
    pub speech: Vec<String>,
    /// Where the local speech models run: "NVIDIA GPU (CUDA)".
    pub speech_runtime: String,
    pub llms: Vec<String>,
    pub llm_runtime: String,
    pub cloud_speech: bool,
    pub cloud_cleanup: bool,
}

/// The models the phone may choose among, local ones first, the PC's own as the default.
pub fn pc_models(s: &Settings, installed: &Installed) -> Vec<ModelInfo> {
    let provider = |id: &str| crate::providers::by_id(id).map_or(id.to_string(), |p| p.name.to_string());
    let mut speech: Vec<ModelInfo> = installed
        .speech
        .iter()
        .filter_map(|id| crate::local::model(id))
        .map(|m| ModelInfo {
            id: m.id.into(),
            kind: ModelKind::Speech,
            label: m.label.into(),
            note: format!("on this PC · {}", installed.speech_runtime),
            default: s.engine == EngineKind::Local && s.local.model == m.id,
        })
        .collect();
    if installed.cloud_speech && !s.server.model.trim().is_empty() {
        speech.push(ModelInfo {
            id: s.server.model.clone(),
            kind: ModelKind::Speech,
            label: s.server.model.clone(),
            note: format!("through this PC · {}", provider(&s.server.provider)),
            default: s.engine == EngineKind::Server,
        });
    }
    let mut cleanup: Vec<ModelInfo> = installed
        .llms
        .iter()
        .filter_map(|id| crate::local::llm_model(id))
        .map(|m| ModelInfo {
            id: m.id.into(),
            kind: ModelKind::Cleanup,
            label: m.label.into(),
            note: format!("on this PC · {}", installed.llm_runtime),
            default: s.cleanup.engine == EngineKind::Local && s.cleanup.local_model == m.id,
        })
        .collect();
    if installed.cloud_cleanup && !s.cleanup.model.trim().is_empty() {
        let label = crate::providers::by_id(&s.cleanup.provider).and_then(|p| p.chat_model(&s.cleanup.model)).map_or(s.cleanup.model.clone(), |m| m.label.to_string());
        cleanup.push(ModelInfo {
            id: s.cleanup.model.clone(),
            kind: ModelKind::Cleanup,
            label,
            note: format!("through this PC · {}", provider(&s.cleanup.provider)),
            default: s.cleanup.engine == EngineKind::Server,
        });
    }
    // The PC's own may not be ready (a model still downloading): then the first is.
    for list in [&mut speech, &mut cleanup] {
        if !list.iter().any(|m| m.default) {
            if let Some(first) = list.first_mut() {
                first.default = true;
            }
        }
    }
    speech.into_iter().chain(cleanup).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ModelKind;
    use crate::settings::{EngineKind, Settings};

    fn installed() -> Installed {
        Installed {
            speech: vec!["acft-large-v3-turbo-q8_0".into(), "futo-multilingual-74".into()],
            speech_runtime: "NVIDIA GPU (CUDA)".into(),
            llms: vec!["qwen3-4b-instruct-2507-q4km".into()],
            llm_runtime: "NVIDIA GPU (CUDA)".into(),
            cloud_speech: true,
            cloud_cleanup: true,
        }
    }

    #[test]
    fn every_downloaded_model_and_the_cloud_one() {
        let mut s = Settings::default();
        s.engine = EngineKind::Local;
        s.local.model = "acft-large-v3-turbo-q8_0".into();
        s.server.provider = "groq".into();
        s.server.model = "whisper-large-v3-turbo".into();
        s.cleanup.engine = EngineKind::Local;
        s.cleanup.local_model = "qwen3-4b-instruct-2507-q4km".into();
        s.cleanup.provider = "groq".into();
        s.cleanup.model = "openai/gpt-oss-20b".into();
        let list = pc_models(&s, &installed());
        let ids = |k: ModelKind| list.iter().filter(|m| m.kind == k).map(|m| (m.id.as_str(), m.default)).collect::<Vec<_>>();
        assert_eq!(ids(ModelKind::Speech), [("acft-large-v3-turbo-q8_0", true), ("futo-multilingual-74", false), ("whisper-large-v3-turbo", false)]);
        assert_eq!(ids(ModelKind::Cleanup), [("qwen3-4b-instruct-2507-q4km", true), ("openai/gpt-oss-20b", false)]);
        assert!(list[0].note.contains("on this PC") && list[2].note.contains("Groq"), "{:?}", list);
    }

    #[test]
    fn the_pcs_own_engine_is_the_default() {
        let mut s = Settings::default();
        s.engine = EngineKind::Server;
        s.server.provider = "groq".into();
        s.server.model = "whisper-large-v3-turbo".into();
        s.cleanup.engine = EngineKind::Server;
        s.cleanup.provider = "groq".into();
        s.cleanup.model = "openai/gpt-oss-20b".into();
        let list = pc_models(&s, &installed());
        let default = |k: ModelKind| list.iter().find(|m| m.kind == k && m.default).map(|m| m.id.clone());
        assert_eq!(default(ModelKind::Speech).as_deref(), Some("whisper-large-v3-turbo"));
        assert_eq!(default(ModelKind::Cleanup).as_deref(), Some("openai/gpt-oss-20b"));
        // No key for the cloud, nothing downloaded: nothing to offer.
        let none = Installed { speech: vec![], llms: vec![], cloud_speech: false, cloud_cleanup: false, ..installed() };
        assert!(pc_models(&s, &none).is_empty());
    }
}

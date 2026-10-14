//! STUB — à remplacer par l'agent audio/moteurs.
use super::TranscribeOptions;
#[derive(Clone, Debug)]
pub struct RemoteEngine { pub endpoint: String, pub api_key: Option<String>, pub model: String }
impl RemoteEngine {
    pub async fn transcribe(&self, _samples: &[f32], _opts: &TranscribeOptions) -> anyhow::Result<String> { todo!() }
    pub async fn test(&self) -> anyhow::Result<String> { todo!() }
}

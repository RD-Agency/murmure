//! STUB — à remplacer par l'agent audio/moteurs.
use super::TranscribeOptions;
use std::path::Path;
pub struct LocalEngine { _p: () }
impl LocalEngine {
    pub fn load(_model_path: &Path) -> anyhow::Result<LocalEngine> { todo!() }
    pub fn model_path(&self) -> &Path { todo!() }
    pub fn transcribe(&self, _samples: &[f32], _opts: &TranscribeOptions) -> anyhow::Result<String> { todo!() }
}

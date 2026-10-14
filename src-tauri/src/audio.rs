//! STUB — à remplacer par l'agent audio/moteurs.
use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct AudioDevice { pub name: String, pub is_default: bool }
pub fn list_input_devices() -> Vec<AudioDevice> { todo!() }
pub struct Recorder { _p: () }
impl Recorder {
    pub fn start(_device: Option<&str>, _on_level: impl Fn(f32) + Send + 'static) -> anyhow::Result<Recorder> { todo!() }
    pub fn stop(self) -> anyhow::Result<Vec<f32>> { todo!() }
}

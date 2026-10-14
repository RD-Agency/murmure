//! STUB — à remplacer par l'agent audio/moteurs.
pub mod local;
pub mod remote;
pub use local::LocalEngine;
pub use remote::RemoteEngine;
#[derive(Clone, Debug, Default)]
pub struct TranscribeOptions { pub language: Option<String>, pub initial_prompt: Option<String> }
pub fn encode_wav(_samples: &[f32]) -> Vec<u8> { todo!() }

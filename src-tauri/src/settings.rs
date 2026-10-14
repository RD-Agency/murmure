//! STUB — à remplacer par l'agent audio/moteurs. Signatures contractuelles (ARCHITECTURE.md).
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind { Local, Remote }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode { Hybrid, Hold, Toggle }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PasteMode { Paste, Clipboard }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub engine: EngineKind,
    pub local_model: String,
    pub language: String,
    pub initial_prompt: String,
    pub remote_endpoint: String,
    pub remote_model: String,
    pub hotkey: String,
    pub hotkey_mode: HotkeyMode,
    pub input_device: Option<String>,
    pub paste_mode: PasteMode,
    pub restore_clipboard: bool,
    pub sounds: bool,
    pub launch_at_login: bool,
    pub history_enabled: bool,
    pub onboarding_done: bool,
}
impl Default for Settings { fn default() -> Self { todo!() } }
pub fn load(_config_dir: &Path) -> Settings { todo!() }
pub fn save(_config_dir: &Path, _s: &Settings) -> anyhow::Result<()> { todo!() }
pub fn get_api_key() -> Option<String> { todo!() }
pub fn set_api_key(_key: &str) -> anyhow::Result<()> { todo!() }

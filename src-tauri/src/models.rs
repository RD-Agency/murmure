//! STUB — à remplacer par l'agent audio/moteurs.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};
#[derive(Clone, Debug, Serialize)]
pub struct ModelInfo { pub id: String, pub name: String, pub description: String, pub size_mb: u64, pub downloaded: bool, pub recommended: bool, pub path: Option<String> }
pub fn models_dir(_data_dir: &Path) -> PathBuf { todo!() }
pub fn list(_data_dir: &Path) -> Vec<ModelInfo> { todo!() }
pub fn model_path(_data_dir: &Path, _id: &str) -> Option<PathBuf> { todo!() }
pub async fn download(_data_dir: &Path, _id: &str, _cancel: Arc<AtomicBool>, _on_progress: impl Fn(u64, u64) + Send + 'static) -> anyhow::Result<PathBuf> { todo!() }
pub fn delete(_data_dir: &Path, _id: &str) -> anyhow::Result<()> { todo!() }

//! Historique local des dictées : `history.json` dans le dossier de données de l'app.
//!
//! Plus récent d'abord, plafonné à [`MAX_ENTRIES`]. Les écritures sont atomiques
//! (fichier temporaire puis renommage) et sérialisées par un verrou global.

use crate::settings::EngineKind;
use anyhow::Context;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Nombre maximal d'entrées conservées.
pub const MAX_ENTRIES: usize = 500;

const FILE_NAME: &str = "history.json";

/// Sérialise les accès lecture-modification-écriture au fichier.
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub text: String,
    /// RFC 3339
    pub created_at: String,
    pub duration_ms: u64,
    pub latency_ms: u64,
    pub engine: EngineKind,
    pub model: String,
}

impl HistoryEntry {
    pub fn new(text: String, duration_ms: u64, latency_ms: u64, engine: EngineKind, model: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            text,
            created_at: chrono::Local::now().to_rfc3339(),
            duration_ms,
            latency_ms,
            engine,
            model,
        }
    }
}

fn file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// Insère en tête et tronque au plafond (logique pure, testée).
pub fn push_capped(entries: &mut Vec<HistoryEntry>, entry: HistoryEntry, cap: usize) {
    entries.insert(0, entry);
    entries.truncate(cap);
}

fn read(data_dir: &Path) -> Vec<HistoryEntry> {
    let path = file_path(data_dir);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            log::warn!("historique illisible ({}), ignoré : {e}", path.display());
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

fn write(data_dir: &Path, entries: &[HistoryEntry]) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("création de {}", data_dir.display()))?;
    let path = file_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(entries)?;
    std::fs::write(&tmp, json).with_context(|| format!("écriture de {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renommage vers {}", path.display()))?;
    Ok(())
}

/// Toutes les entrées, plus récente d'abord.
pub fn load(data_dir: &Path) -> Vec<HistoryEntry> {
    let _g = LOCK.lock();
    read(data_dir)
}

pub fn add(data_dir: &Path, entry: HistoryEntry) -> anyhow::Result<()> {
    let _g = LOCK.lock();
    let mut entries = read(data_dir);
    push_capped(&mut entries, entry, MAX_ENTRIES);
    write(data_dir, &entries)
}

pub fn delete(data_dir: &Path, id: &str) -> anyhow::Result<()> {
    let _g = LOCK.lock();
    let mut entries = read(data_dir);
    entries.retain(|e| e.id != id);
    write(data_dir, &entries)
}

pub fn clear(data_dir: &Path) -> anyhow::Result<()> {
    let _g = LOCK.lock();
    write(data_dir, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> HistoryEntry {
        HistoryEntry::new(text.to_string(), 1000, 200, EngineKind::Local, "m".into())
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmure-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn push_capped_keeps_newest_first_and_caps() {
        let mut v = Vec::new();
        for i in 0..(MAX_ENTRIES + 20) {
            push_capped(&mut v, entry(&i.to_string()), MAX_ENTRIES);
        }
        assert_eq!(v.len(), MAX_ENTRIES);
        assert_eq!(v[0].text, (MAX_ENTRIES + 19).to_string());
        assert_eq!(v[MAX_ENTRIES - 1].text, "20");
    }

    #[test]
    fn add_delete_clear_roundtrip() {
        let dir = temp_dir();
        assert!(load(&dir).is_empty());
        add(&dir, entry("a")).unwrap();
        add(&dir, entry("b")).unwrap();
        let all = load(&dir);
        assert_eq!(all.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        delete(&dir, &all[0].id).unwrap();
        assert_eq!(load(&dir)[0].text, "a");
        clear(&dir).unwrap();
        assert!(load(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_on_disk_is_capped() {
        let dir = temp_dir();
        let many: Vec<_> = (0..MAX_ENTRIES).map(|i| entry(&i.to_string())).collect();
        write(&dir, &many).unwrap();
        add(&dir, entry("new")).unwrap();
        let all = load(&dir);
        assert_eq!(all.len(), MAX_ENTRIES);
        assert_eq!(all[0].text, "new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_file_yields_empty() {
        let dir = temp_dir();
        std::fs::write(dir.join(FILE_NAME), b"{pas du json").unwrap();
        assert!(load(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! État partagé de l'application (géré par Tauri via `app.manage`).
//!
//! Tous les verrous sont des `parking_lot::Mutex` : on ne les garde jamais
//! pendant un appel lent (chargement de modèle, réseau, collage).

use crate::audio::Recorder;
use crate::engine::LocalEngine;
use crate::hotkey::HotkeyMachine;
use crate::pipeline::Command;
use crate::settings::Settings;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

/// Phase de la machine d'états de la dictée.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Recording,
    Transcribing,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Recording => "recording",
            Phase::Transcribing => "transcribing",
        }
    }
}

/// Session de dictée en cours.
pub struct Dictation {
    pub phase: Phase,
    pub recorder: Option<Recorder>,
    pub started_at: Option<Instant>,
    /// Mains libres : l'enregistrement continue sans maintenir la touche.
    pub hands_free: bool,
    /// Incrémenté à chaque démarrage / annulation / erreur : permet d'ignorer
    /// les résultats ou retours à « idle » devenus obsolètes.
    pub generation: u64,
}

impl Default for Dictation {
    fn default() -> Self {
        Self { phase: Phase::Idle, recorder: None, started_at: None, hands_free: false, generation: 0 }
    }
}

/// Moteur local chargé en mémoire et l'id du modèle correspondant.
pub struct LoadedEngine {
    pub model_id: String,
    pub engine: Arc<LocalEngine>,
}

/// Téléchargement en cours : annulation + progression lisible depuis n'importe quel thread.
#[derive(Clone)]
pub struct DownloadHandle {
    pub cancel: Arc<AtomicBool>,
    downloaded: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl DownloadHandle {
    fn new(total_hint: u64) -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            downloaded: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(total_hint)),
        }
    }

    pub fn set_progress(&self, downloaded: u64, total: u64) {
        self.downloaded.store(downloaded, Ordering::Relaxed);
        if total > 0 {
            self.total.store(total, Ordering::Relaxed);
        }
    }

    pub fn progress(&self) -> (u64, u64) {
        (self.downloaded.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }

    /// Pourcentage entier, plafonné à 99 tant que le téléchargement n'est pas fini.
    pub fn percent(&self) -> u8 {
        let (d, t) = self.progress();
        percent(d, t)
    }
}

/// Pourcentage entier 0..=99 (100 n'est affiché qu'une fois le fichier vérifié).
pub fn percent(downloaded: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    ((downloaded.min(total) as u128 * 100 / total as u128) as u8).min(99)
}

/// Résultat de [`Downloads::begin`].
pub enum Begin {
    /// Nouveau téléchargement : l'appelant doit le lancer puis appeler [`Downloads::finish`].
    Started(DownloadHandle),
    /// Un téléchargement du même modèle tourne déjà : on le rejoint (rien à lancer).
    Joined(DownloadHandle),
}

/// Ligne renvoyée par la commande `get_downloads`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DownloadStatus {
    pub id: String,
    pub downloaded: u64,
    pub total: u64,
}

/// Registre des téléchargements en cours : garantit qu'un modèle n'est téléchargé qu'une fois.
#[derive(Default)]
pub struct Downloads(Mutex<HashMap<String, DownloadHandle>>);

impl Downloads {
    /// Réserve le téléchargement de `id`, ou rejoint celui déjà en cours.
    pub fn begin(&self, id: &str, total_hint: u64) -> Begin {
        let mut map = self.0.lock();
        if let Some(h) = map.get(id) {
            return Begin::Joined(h.clone());
        }
        let h = DownloadHandle::new(total_hint);
        map.insert(id.to_string(), h.clone());
        Begin::Started(h)
    }

    pub fn finish(&self, id: &str) {
        self.0.lock().remove(id);
    }

    pub fn get(&self, id: &str) -> Option<DownloadHandle> {
        self.0.lock().get(id).cloned()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.lock().contains_key(id)
    }

    /// Demande l'annulation ; renvoie `false` si rien n'était en cours.
    pub fn cancel(&self, id: &str) -> bool {
        match self.0.lock().get(id) {
            Some(h) => {
                h.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Instantané trié par id (ordre stable pour le front).
    pub fn snapshot(&self) -> Vec<DownloadStatus> {
        let mut v: Vec<DownloadStatus> = self
            .0
            .lock()
            .iter()
            .map(|(id, h)| {
                let (downloaded, total) = h.progress();
                DownloadStatus { id: id.clone(), downloaded, total }
            })
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }
}

pub struct AppState {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub dictation: Mutex<Dictation>,
    pub engine: Mutex<Option<LoadedEngine>>,
    /// Sérialise les chargements de modèle (évite deux chargements concurrents).
    pub engine_load: Mutex<()>,
    /// Téléchargements de modèles en cours (un seul par id).
    pub downloads: Downloads,
    /// Cache de la clé API (évite d'interroger le trousseau à chaque dictée).
    /// `None` = pas encore lu.
    pub api_key_cache: Mutex<Option<Option<String>>>,
    pub hotkey: Mutex<HotkeyMachine>,
    /// File de commandes vers le thread de dictée (ordre garanti).
    pub commands: Sender<Command>,
}

impl AppState {
    pub fn new(config_dir: PathBuf, data_dir: PathBuf, settings: Settings, commands: Sender<Command>) -> Self {
        Self {
            config_dir,
            data_dir,
            settings: Mutex::new(settings),
            dictation: Mutex::new(Dictation::default()),
            engine: Mutex::new(None),
            engine_load: Mutex::new(()),
            downloads: Downloads::default(),
            api_key_cache: Mutex::new(None),
            hotkey: Mutex::new(HotkeyMachine::default()),
            commands,
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().clone()
    }

    pub fn phase(&self) -> Phase {
        self.dictation.lock().phase
    }

    pub fn is_recording(&self) -> bool {
        self.phase() == Phase::Recording
    }

    /// Clé API (trousseau), mise en cache.
    pub fn api_key(&self) -> Option<String> {
        let mut cache = self.api_key_cache.lock();
        if let Some(k) = cache.as_ref() {
            return k.clone();
        }
        let k = crate::settings::get_api_key().filter(|k| !k.trim().is_empty());
        *cache = Some(k.clone());
        k
    }

    pub fn invalidate_api_key(&self) {
        *self.api_key_cache.lock() = None;
    }

    /// Décharge le moteur local s'il ne correspond plus à `model_id`.
    pub fn unload_engine_unless(&self, model_id: &str) {
        let mut eng = self.engine.lock();
        if eng.as_ref().is_some_and(|l| l.model_id != model_id) {
            log::info!("déchargement du moteur local (modèle changé)");
            *eng = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedoublonnage_des_telechargements() {
        let d = Downloads::default();
        let Begin::Started(h) = d.begin("small-q5_1", 100) else { panic!("devait démarrer") };
        h.set_progress(42, 100);
        // Second appel (clic dans l'UI pendant l'auto-installation) : on rejoint, même handle.
        let Begin::Joined(j) = d.begin("small-q5_1", 100) else { panic!("devait rejoindre") };
        assert_eq!(j.progress(), (42, 100));
        assert!(Arc::ptr_eq(&h.cancel, &j.cancel));
        assert_eq!(d.snapshot(), vec![DownloadStatus { id: "small-q5_1".into(), downloaded: 42, total: 100 }]);
        // Un autre modèle est indépendant.
        assert!(matches!(d.begin("large-v3-turbo", 10), Begin::Started(_)));
        assert_eq!(d.snapshot().len(), 2);
        // Annulation partagée, puis libération.
        assert!(d.cancel("small-q5_1"));
        assert!(h.cancel.load(Ordering::Relaxed));
        d.finish("small-q5_1");
        assert!(!d.contains("small-q5_1"));
        assert!(!d.cancel("small-q5_1"));
        assert!(matches!(d.begin("small-q5_1", 100), Begin::Started(_)));
    }

    #[test]
    fn pourcentage() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(0, 547), 0);
        assert_eq!(percent(230, 547), 42);
        assert_eq!(percent(547, 547), 99);
        assert_eq!(percent(600, 547), 99);
        assert_eq!(percent(u64::MAX / 2, u64::MAX), 49);
        let h = DownloadHandle::new(1000);
        h.set_progress(500, 0); // total inconnu : on garde l'estimation
        assert_eq!(h.percent(), 50);
    }
}

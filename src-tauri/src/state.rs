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
use std::sync::atomic::AtomicBool;
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

pub struct AppState {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub dictation: Mutex<Dictation>,
    pub engine: Mutex<Option<LoadedEngine>>,
    /// Sérialise les chargements de modèle (évite deux chargements concurrents).
    pub engine_load: Mutex<()>,
    /// Téléchargements en cours : id du modèle → drapeau d'annulation.
    pub downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
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
            downloads: Mutex::new(HashMap::new()),
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

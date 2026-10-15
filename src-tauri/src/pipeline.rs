//! Flux de dictée : start / stop / cancel / toggle.
//!
//! Toutes les commandes passent par une file (`std::sync::mpsc`) consommée par un
//! thread dédié : l'ordre appui → relâchement est donc garanti, et l'ouverture du
//! micro (quelques dizaines de ms) ne bloque ni le thread principal ni le handler
//! du raccourci. La transcription est lancée en tâche async (locale dans
//! `spawn_blocking`, distante en async) pour ne pas bloquer la file.

use crate::audio::Recorder;
use crate::engine::{LocalEngine, RemoteEngine, TranscribeOptions};
use crate::history::{self, HistoryEntry};
use crate::inject::{self, Outcome};
use crate::settings::{EngineKind, Settings};
use crate::state::{AppState, LoadedEngine, Phase};
use crate::{hotkey, install, models, overlay, tray};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Enregistrements plus courts : abandon silencieux.
const MIN_RECORDING: Duration = Duration::from_millis(250);
/// Durée d'affichage d'une erreur avant retour à « idle ».
const ERROR_DISPLAY: Duration = Duration::from_millis(2500);
/// Intervalle minimal entre deux événements `audio-level` (~30 Hz).
const LEVEL_INTERVAL: Duration = Duration::from_millis(33);
/// En dessous de ce RMS (et de ce pic), l'enregistrement est considéré comme du silence.
const SILENCE_RMS: f32 = 0.002;
const SILENCE_PEAK: f32 = 0.01;
const SAMPLE_RATE: usize = 16_000;

pub const NO_MODEL_MSG: &str = "Aucun modèle local téléchargé — ouvrez Murmure pour en installer un";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Start { hands_free: bool },
    Stop,
    Cancel,
    Toggle,
    HandsFree,
}

#[derive(Clone, Serialize)]
struct StatePayload<'a> {
    state: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hands_free: Option<bool>,
}

/// Envoie une commande au thread de dictée.
pub fn send(app: &AppHandle, cmd: Command) {
    if let Some(state) = app.try_state::<AppState>()
        && state.commands.send(cmd).is_err()
    {
        log::error!("thread de dictée arrêté, commande {cmd:?} perdue");
    }
}

/// Démarre le thread consommateur de la file.
pub fn spawn_worker(app: AppHandle, rx: Receiver<Command>) {
    let res = std::thread::Builder::new()
        .name("murmure-dictation".into())
        .spawn(move || {
            for cmd in rx {
                log::debug!("commande dictée : {cmd:?}");
                match cmd {
                    Command::Start { hands_free } => start(&app, hands_free),
                    Command::Stop => stop(&app),
                    Command::Cancel => cancel(&app),
                    Command::Toggle => {
                        match app.state::<AppState>().phase() {
                            Phase::Recording => stop(&app),
                            Phase::Idle => start(&app, true),
                            Phase::Transcribing => {}
                        }
                    }
                    Command::HandsFree => set_hands_free(&app),
                }
            }
        });
    if let Err(e) = res {
        log::error!("impossible de démarrer le thread de dictée : {e}");
    }
}

fn emit_state(app: &AppHandle, state: &str, message: Option<&str>, hands_free: Option<bool>) {
    if let Err(e) = app.emit("dictation-state", StatePayload { state, message, hands_free }) {
        log::warn!("émission dictation-state : {e}");
    }
}

/// Retour à l'état de repos (si la génération n'a pas changé entre-temps).
fn finish_idle(app: &AppHandle, generation: u64, message: Option<&str>) {
    let state = app.state::<AppState>();
    {
        let mut d = state.dictation.lock();
        if d.generation != generation {
            return;
        }
        d.phase = Phase::Idle;
        d.recorder = None;
        d.started_at = None;
        d.hands_free = false;
    }
    emit_state(app, "idle", message, None);
    overlay::hide(app);
    tray::update(app, Phase::Idle);
}

/// Affiche une erreur puis revient à « idle » après [`ERROR_DISPLAY`].
fn fail(app: &AppHandle, message: &str) {
    log::warn!("dictée : {message}");
    let state = app.state::<AppState>();
    let generation = {
        let mut d = state.dictation.lock();
        d.phase = Phase::Idle;
        d.recorder = None;
        d.started_at = None;
        d.hands_free = false;
        d.generation += 1;
        d.generation
    };
    hotkey::unregister_escape(app);
    emit_state(app, "error", Some(message), None);
    overlay::show(app);
    tray::update(app, Phase::Idle);
    if message == NO_MODEL_MSG {
        tray::show_main(app, Some("home"));
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(ERROR_DISPLAY).await;
        let state = app.state::<AppState>();
        let still_current = {
            let d = state.dictation.lock();
            d.generation == generation && d.phase == Phase::Idle
        };
        if still_current {
            emit_state(&app, "idle", None, None);
            overlay::hide(&app);
        }
    });
}

/// Vérifie que le moteur choisi est utilisable avant d'ouvrir le micro.
/// Sans modèle local, lance son installation et renvoie le message de progression
/// (la fenêtre principale n'est pas ouverte : elle ne l'est qu'en cas d'échec du téléchargement).
fn preflight(app: &AppHandle, state: &AppState, settings: &Settings) -> Result<(), String> {
    match settings.engine {
        EngineKind::Local => {
            if models::model_path(&state.data_dir, &settings.local_model).is_none() {
                // Pas de modèle : installation automatique (ou progression de celle en cours).
                return Err(install::on_dictation_without_model(app, settings).unwrap_or_else(|e| e));
            }
        }
        EngineKind::Remote => {
            if settings.remote_endpoint.trim().is_empty() {
                return Err("Aucun serveur de transcription configuré — renseignez l'adresse dans les réglages".into());
            }
            if state.api_key().is_none() && !is_local_endpoint(&settings.remote_endpoint) {
                return Err("Clé API manquante — ajoutez-la dans les réglages de Murmure".into());
            }
        }
    }
    Ok(())
}

/// Un serveur sur la machine locale peut fonctionner sans clé.
fn is_local_endpoint(endpoint: &str) -> bool {
    let rest = endpoint.trim().split("://").nth(1).unwrap_or(endpoint);
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn start(app: &AppHandle, hands_free: bool) {
    let state = app.state::<AppState>();
    if state.phase() != Phase::Idle {
        log::debug!("démarrage ignoré : dictée déjà en cours");
        return;
    }
    let settings = state.settings();
    if let Err(msg) = preflight(app, &state, &settings) {
        fail(app, &msg);
        return;
    }

    let level_app = app.clone();
    let last_emit = AtomicU64::new(0);
    let origin = Instant::now();
    let on_level = move |level: f32| {
        // Limitation à ~30 Hz quel que soit le rythme du Recorder.
        let now_ms = origin.elapsed().as_millis() as u64;
        let last = last_emit.load(Ordering::Relaxed);
        if last != 0 && now_ms.saturating_sub(last) < LEVEL_INTERVAL.as_millis() as u64 {
            return;
        }
        last_emit.store(now_ms.max(1), Ordering::Relaxed);
        let _ = level_app.emit("audio-level", level.clamp(0.0, 1.0));
    };

    let recorder = match Recorder::start(settings.input_device.as_deref(), on_level) {
        Ok(r) => r,
        Err(e) => {
            fail(app, &format!("Micro indisponible : {e}"));
            return;
        }
    };
    {
        let mut d = state.dictation.lock();
        d.phase = Phase::Recording;
        d.recorder = Some(recorder);
        d.started_at = Some(Instant::now());
        d.hands_free = hands_free;
        d.generation += 1;
    }
    emit_state(app, "recording", None, Some(hands_free));
    overlay::show(app);
    tray::update(app, Phase::Recording);
    hotkey::register_escape(app);
    if settings.sounds {
        play_sound(SoundKind::Start);
    }

    // Préchauffe le moteur local pendant que l'utilisateur parle.
    if settings.engine == EngineKind::Local {
        let app2 = app.clone();
        let id = settings.local_model.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = ensure_local_engine(&app2, &id) {
                log::warn!("préchargement du moteur : {e}");
            }
        });
    }
}

fn set_hands_free(app: &AppHandle) {
    let state = app.state::<AppState>();
    {
        let mut d = state.dictation.lock();
        if d.phase != Phase::Recording {
            return;
        }
        d.hands_free = true;
    }
    emit_state(app, "recording", None, Some(true));
}

fn stop(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (recorder, started_at, generation) = {
        let mut d = state.dictation.lock();
        if d.phase != Phase::Recording {
            return;
        }
        d.phase = Phase::Transcribing;
        (d.recorder.take(), d.started_at.take(), d.generation)
    };
    hotkey::unregister_escape(app);
    let settings = state.settings();
    if settings.sounds {
        play_sound(SoundKind::Stop);
    }
    let duration = started_at.map(|t| t.elapsed()).unwrap_or_default();

    let Some(recorder) = recorder else {
        finish_idle(app, generation, None);
        return;
    };
    let samples = match recorder.stop() {
        Ok(s) => s,
        Err(e) => {
            fail(app, &format!("Erreur d'enregistrement : {e}"));
            return;
        }
    };
    if duration < MIN_RECORDING
        || samples.len() < SAMPLE_RATE * MIN_RECORDING.as_millis() as usize / 1000
        || is_silent(&samples)
    {
        log::debug!("enregistrement trop court ou silencieux : abandon");
        finish_idle(app, generation, None);
        return;
    }

    emit_state(app, "transcribing", None, None);
    tray::update(app, Phase::Transcribing);
    let duration_ms = duration.as_millis() as u64;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        transcribe_and_deliver(app, samples, settings, generation, duration_ms).await;
    });
}

fn cancel(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (recorder, phase, generation) = {
        let mut d = state.dictation.lock();
        let phase = d.phase;
        if phase == Phase::Idle {
            return;
        }
        // Invalide la session : un résultat en vol sera ignoré.
        d.generation += 1;
        (d.recorder.take(), phase, d.generation)
    };
    hotkey::unregister_escape(app);
    log::info!("dictée annulée ({})", phase.as_str());
    if let Some(rec) = recorder {
        // On jette l'audio, hors du thread de la file.
        std::thread::spawn(move || {
            let _ = rec.stop();
        });
    }
    finish_idle(app, generation, None);
}

async fn transcribe_and_deliver(
    app: AppHandle,
    samples: Vec<f32>,
    settings: Settings,
    generation: u64,
    duration_ms: u64,
) {
    let state = app.state::<AppState>();
    let spoken_punctuation = settings.spoken_punctuation && matches!(settings.language.trim(), "fr" | "auto" | "");
    let opts = TranscribeOptions {
        language: match settings.language.trim() {
            "" | "auto" => None,
            l => Some(l.to_string()),
        },
        // Vocabulaire des commandes seulement en français fixé : en détection auto, il biaiserait la langue.
        initial_prompt: initial_prompt(&settings.initial_prompt, spoken_punctuation && settings.language.trim() == "fr"),
    };
    let t0 = Instant::now();
    let (result, model) = match settings.engine {
        EngineKind::Local => {
            let app2 = app.clone();
            let id = settings.local_model.clone();
            let res = tauri::async_runtime::spawn_blocking(move || {
                let engine = ensure_local_engine(&app2, &id)?;
                engine.transcribe(&samples, &opts).map_err(|e| format!("Échec de la transcription : {e}"))
            })
            .await
            .map_err(|e| format!("Échec de la transcription : {e}"))
            .and_then(|r| r);
            (res, settings.local_model.clone())
        }
        EngineKind::Remote => {
            let engine = RemoteEngine {
                endpoint: settings.remote_endpoint.trim().to_string(),
                api_key: state.api_key(),
                model: settings.remote_model.clone(),
            };
            let res = engine
                .transcribe(&samples, &opts)
                .await
                .map_err(|e| format!("Échec de la transcription distante : {e}"));
            (res, settings.remote_model.clone())
        }
    };
    let latency_ms = t0.elapsed().as_millis() as u64;

    if state.dictation.lock().generation != generation {
        log::debug!("résultat ignoré (dictée annulée)");
        return;
    }
    let text = match result {
        Ok(t) if spoken_punctuation => crate::punctuation::apply_fr(&clean_text(&t)),
        Ok(t) => clean_text(&t),
        Err(msg) => {
            fail(&app, &msg);
            return;
        }
    };
    if text.is_empty() {
        finish_idle(&app, generation, None);
        return;
    }

    let app2 = app.clone();
    let to_paste = text.clone();
    let (mode, restore) = (settings.paste_mode, settings.restore_clipboard);
    let outcome = tauri::async_runtime::spawn_blocking(move || inject::deliver(&app2, &to_paste, mode, restore))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);

    let entry = HistoryEntry::new(text, duration_ms, latency_ms, settings.engine, model);
    if settings.history_enabled {
        let data_dir = state.data_dir.clone();
        let e2 = entry.clone();
        let saved = tauri::async_runtime::spawn_blocking(move || history::add(&data_dir, e2)).await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(e)) => log::warn!("historique : {e}"),
            Err(e) => log::warn!("historique : {e}"),
        }
    }
    if let Err(e) = app.emit("dictation-result", &entry) {
        log::warn!("émission dictation-result : {e}");
    }

    match outcome {
        Ok(Outcome::Pasted) | Ok(Outcome::Copied) => finish_idle(&app, generation, None),
        Ok(Outcome::CopiedOnly(msg)) => fail(&app, &msg),
        Err(msg) => fail(&app, &msg),
    }
}

/// Renvoie le moteur local pour `model_id`, en le chargeant (ou rechargeant) si besoin.
/// Bloquant : à appeler hors du thread principal.
pub fn ensure_local_engine(app: &AppHandle, model_id: &str) -> Result<Arc<LocalEngine>, String> {
    let state = app.state::<AppState>();
    let cached = |state: &AppState| {
        state
            .engine
            .lock()
            .as_ref()
            .filter(|l| l.model_id == model_id)
            .map(|l| l.engine.clone())
    };
    if let Some(e) = cached(&state) {
        return Ok(e);
    }
    let _loading = state.engine_load.lock();
    if let Some(e) = cached(&state) {
        return Ok(e);
    }
    let path = models::model_path(&state.data_dir, model_id).ok_or_else(|| NO_MODEL_MSG.to_string())?;
    log::info!("chargement du modèle local {model_id} ({})", path.display());
    let t0 = Instant::now();
    let engine = Arc::new(
        LocalEngine::load(&path).map_err(|e| format!("Impossible de charger le modèle « {model_id} » : {e}"))?,
    );
    log::info!("modèle chargé en {:?}", t0.elapsed());
    *state.engine.lock() = Some(LoadedEngine { model_id: model_id.to_string(), engine: engine.clone() });
    Ok(engine)
}

/// Normalise le texte transcrit : espaces superflus, pas d'espace en tête ni en fin.
/// Prompt initial de Whisper : vocabulaire de l'utilisateur, plus les commandes de ponctuation dictée.
fn initial_prompt(vocabulary: &str, spoken_punctuation: bool) -> Option<String> {
    let vocabulary = vocabulary.trim();
    let prompt = match (vocabulary.is_empty(), spoken_punctuation) {
        (true, false) => return None,
        (true, true) => crate::punctuation::PROMPT_FR.to_string(),
        (false, false) => vocabulary.to_string(),
        (false, true) => format!("{vocabulary} {}", crate::punctuation::PROMPT_FR),
    };
    Some(prompt)
}

pub fn clean_text(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_silent(samples: &[f32]) -> bool {
    if samples.is_empty() {
        return true;
    }
    let mut peak = 0f32;
    let mut sum = 0f64;
    for &s in samples {
        peak = peak.max(s.abs());
        sum += f64::from(s) * f64::from(s);
    }
    let rms = (sum / samples.len() as f64).sqrt() as f32;
    rms < SILENCE_RMS && peak < SILENCE_PEAK
}

#[derive(Clone, Copy)]
enum SoundKind {
    Start,
    Stop,
}

/// Petit son système, sans bloquer (macOS : `afplay`, Linux : `canberra-gtk-play` si présent).
fn play_sound(kind: SoundKind) {
    #[cfg(target_os = "macos")]
    let cmd: Option<(&str, Vec<&str>)> = Some((
        "afplay",
        vec![
            "-v",
            "0.35",
            match kind {
                SoundKind::Start => "/System/Library/Sounds/Tink.aiff",
                SoundKind::Stop => "/System/Library/Sounds/Pop.aiff",
            },
        ],
    ));
    #[cfg(target_os = "linux")]
    let cmd: Option<(&str, Vec<&str>)> = Some((
        "canberra-gtk-play",
        vec![
            "-i",
            match kind {
                SoundKind::Start => "message-new-instant",
                SoundKind::Stop => "message-sent-instant",
            },
        ],
    ));
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let cmd: Option<(&str, Vec<&str>)> = {
        let _ = kind;
        None
    };

    if let Some((bin, args)) = cmd {
        let child = std::process::Command::new(bin)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            // Récupère le processus pour éviter les zombies.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_normalizes_whitespace() {
        assert_eq!(clean_text("  Bonjour   tout\n le monde.  "), "Bonjour tout le monde.");
        assert_eq!(clean_text("\t \n"), "");
        assert_eq!(clean_text("Déjà propre"), "Déjà propre");
    }

    #[test]
    fn silence_detection() {
        assert!(is_silent(&[]));
        assert!(is_silent(&vec![0.0005; 16_000]));
        let voice: Vec<f32> = (0..16_000).map(|i| (i as f32 * 0.05).sin() * 0.2).collect();
        assert!(!is_silent(&voice));
    }

    #[test]
    fn local_endpoint_detection() {
        assert!(is_local_endpoint("http://localhost:8000/v1/audio/transcriptions"));
        assert!(is_local_endpoint("http://127.0.0.1/v1/audio/transcriptions"));
        assert!(is_local_endpoint("http://[::1]:9000/v1"));
        assert!(!is_local_endpoint("https://api.openai.com/v1/audio/transcriptions"));
        assert!(!is_local_endpoint("https://localhost.evil.fr/v1"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Transcription d'un fichier audio (dictaphone), depuis l'écran d'accueil.
//!
//! Décodage et découpage dans `audio_file.rs`, puis chaque segment passe par le moteur choisi
//! (local ou serveur), avec la langue et le vocabulaire des réglages. Le texte assemblé reçoit la
//! ponctuation dictée ; il n'est ni collé ni ajouté à l'historique : l'interface l'affiche et le
//! copie à la demande.
//!
//! Un seul fichier à la fois. Progression par l'événement `file-transcription-progress` ;
//! `cancel_file_transcription` interrompt le décodage ou s'arrête après le segment en cours.

use crate::audio_file::{self, CANCELLED};
use crate::engine::{RemoteEngine, TranscribeOptions};
use crate::postprocess;
use crate::settings::{EngineKind, Settings};
use crate::state::AppState;
use crate::{models, pipeline};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// Part de la barre de progression consacrée au décodage.
const DECODE_SHARE: f32 = 0.1;

/// Tâche en cours (gérée par Tauri).
#[derive(Default)]
pub struct FileJob {
    busy: AtomicBool,
    cancel: Arc<AtomicBool>,
}

/// Libère la tâche quoi qu'il arrive.
struct Busy<'a>(&'a FileJob);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::SeqCst);
    }
}

/// Événement `file-transcription-progress`.
#[derive(Clone, Serialize)]
pub struct Progress {
    /// « decoding » puis « transcribing ».
    pub stage: &'static str,
    /// Avancement global de 0 à 1.
    pub fraction: f32,
    /// Segments transcrits et nombre total (0 pendant le décodage).
    pub done: usize,
    pub total: usize,
}

/// Résultat renvoyé au front.
#[derive(Clone, Serialize)]
pub struct FileTranscript {
    pub text: String,
    pub file_name: String,
    /// Durée de l'audio.
    pub duration_ms: u64,
    /// Temps de traitement.
    pub latency_ms: u64,
}

fn emit(app: &AppHandle, p: Progress) {
    if let Err(e) = app.emit("file-transcription-progress", p) {
        log::debug!("émission file-transcription-progress : {e}");
    }
}

/// Boîte de dialogue de choix du fichier ; `None` si l'utilisateur annule.
#[tauri::command]
pub async fn pick_audio_file(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Transcrire un fichier audio")
            .add_filter("Fichiers audio", audio_file::EXTENSIONS)
            .blocking_pick_file()
            .and_then(|p| p.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_file_transcription(job: State<'_, FileJob>) {
    job.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub async fn transcribe_file(app: AppHandle, job: State<'_, FileJob>, path: String) -> Result<FileTranscript, String> {
    if job.busy.swap(true, Ordering::SeqCst) {
        return Err("Une transcription de fichier est déjà en cours.".into());
    }
    let _busy = Busy(&job);
    job.cancel.store(false, Ordering::SeqCst);
    let cancel = job.cancel.clone();
    let path = PathBuf::from(path);
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let state = app.state::<AppState>();
    let settings = state.settings();
    check_engine(&state, &settings)?;
    let started = Instant::now();

    // 1. Décodage, hors du runtime async.
    emit(&app, Progress { stage: "decoding", fraction: 0.0, done: 0, total: 0 });
    let samples = {
        let (app, cancel) = (app.clone(), cancel.clone());
        tauri::async_runtime::spawn_blocking(move || {
            audio_file::decode_file(&path, &cancel, |p| {
                emit(&app, Progress { stage: "decoding", fraction: p * DECODE_SHARE, done: 0, total: 0 });
            })
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| {
            let msg = format!("{e:#}");
            if msg == CANCELLED { msg } else { format!("Fichier illisible : {msg}") }
        })?
    };
    let duration_ms = samples.len() as u64 * 1000 / u64::from(audio_file::RATE);
    log::info!("fichier « {file_name} » décodé : {} s", duration_ms / 1000);

    // 2. Segments d'environ 20 s coupés sur les silences.
    let segments = audio_file::split_on_silence(&samples, audio_file::SEGMENT_TARGET, audio_file::SEGMENT_WINDOW);
    let total = segments.len();
    let samples = Arc::new(samples);
    let spoken_punctuation = settings.spoken_punctuation && matches!(settings.language.trim(), "fr" | "auto" | "");
    let opts = TranscribeOptions {
        language: match settings.language.trim() {
            "" | "auto" => None,
            l => Some(l.to_string()),
        },
        initial_prompt: pipeline::initial_prompt(&settings.initial_prompt, spoken_punctuation && settings.language.trim() == "fr"),
    };
    let mut parts: Vec<String> = Vec::with_capacity(total);
    emit(&app, Progress { stage: "transcribing", fraction: DECODE_SHARE, done: 0, total });
    for (i, range) in segments.into_iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.into());
        }
        if !audio_file::is_silent(&samples[range.clone()]) {
            let text = transcribe_segment(&app, &settings, samples.clone(), range, &opts).await?;
            let text = pipeline::clean_text(&text);
            if !text.is_empty() {
                parts.push(text);
            }
        }
        let fraction = DECODE_SHARE + (1.0 - DECODE_SHARE) * (i + 1) as f32 / total as f32;
        emit(&app, Progress { stage: "transcribing", fraction, done: i + 1, total });
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }

    let opts = postprocess::Options {
        spoken_punctuation,
        short_text_plain: false,
        submit_words: &[],
        vocabulary: &settings.initial_prompt,
    };
    let text = postprocess::finalize(&parts.join(" "), &opts).text;
    if text.is_empty() {
        return Err("Aucune parole reconnue dans ce fichier.".into());
    }
    Ok(FileTranscript { text, file_name, duration_ms, latency_ms: started.elapsed().as_millis() as u64 })
}

/// Le moteur choisi est-il utilisable ? (Pas d'installation automatique ici : l'accueil l'affiche déjà.)
fn check_engine(state: &AppState, s: &Settings) -> Result<(), String> {
    match s.engine {
        EngineKind::Local if models::model_path(&state.data_dir, &s.local_model).is_none() => {
            Err(pipeline::NO_MODEL_MSG.into())
        }
        EngineKind::Remote if s.remote_endpoint.trim().is_empty() => {
            Err("Aucun serveur de transcription configuré : renseignez l'adresse dans les réglages.".into())
        }
        _ => Ok(()),
    }
}

async fn transcribe_segment(
    app: &AppHandle,
    settings: &Settings,
    samples: Arc<Vec<f32>>,
    range: std::ops::Range<usize>,
    opts: &TranscribeOptions,
) -> Result<String, String> {
    match settings.engine {
        EngineKind::Local => {
            let (app, id, opts) = (app.clone(), settings.local_model.clone(), opts.clone());
            tauri::async_runtime::spawn_blocking(move || {
                let engine = pipeline::ensure_local_engine(&app, &id)?;
                engine.transcribe(&samples[range], &opts).map_err(|e| format!("Échec de la transcription : {e}"))
            })
            .await
            .map_err(|e| format!("Échec de la transcription : {e}"))?
        }
        EngineKind::Remote => {
            let state = app.state::<AppState>();
            let engine = RemoteEngine {
                endpoint: settings.remote_endpoint.trim().to_string(),
                api_key: state.api_key(),
                model: settings.remote_model.clone(),
            };
            engine
                .transcribe(&samples[range], opts)
                .await
                .map_err(|e| format!("Échec de la transcription distante : {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::LocalEngine;

    /// Chaîne complète sur un vrai fichier : décodage, découpage, Whisper local, ponctuation.
    /// `MURMURE_TEST_AUDIO=/chemin/dictee.m4a cargo test --release --lib file_transcription -- --ignored --nocapture`
    #[test]
    #[ignore = "nécessite ~/.whisper/models/ggml-large-v3-turbo.bin et un fichier audio"]
    fn integration_fichier_complet() {
        let audio = std::env::var("MURMURE_TEST_AUDIO").expect("MURMURE_TEST_AUDIO");
        let model = dirs::home_dir().unwrap().join(".whisper/models/ggml-large-v3-turbo.bin");
        let t = Instant::now();
        let samples = audio_file::decode_file(std::path::Path::new(&audio), &AtomicBool::new(false), |_| {}).unwrap();
        println!("décodage : {} ms, {} s d'audio", t.elapsed().as_millis(), samples.len() / 16_000);
        let segments = audio_file::split_on_silence(&samples, audio_file::SEGMENT_TARGET, audio_file::SEGMENT_WINDOW);
        println!("segments (s) : {:?}", segments.iter().map(|r| (r.start / 16_000, r.end / 16_000)).collect::<Vec<_>>());
        let engine = LocalEngine::load(&model).unwrap();
        let opts = TranscribeOptions { language: Some("fr".into()), initial_prompt: pipeline::initial_prompt("", true) };
        let t = Instant::now();
        let parts: Vec<String> =
            segments.into_iter().map(|r| pipeline::clean_text(&engine.transcribe(&samples[r], &opts).unwrap())).collect();
        let post = postprocess::Options { spoken_punctuation: true, ..Default::default() };
        let text = postprocess::finalize(&parts.join(" "), &post).text;
        println!("transcription : {} ms\n{text}", t.elapsed().as_millis());
        assert!(postprocess::count_words(&text) > 20);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fenêtre dictaphone : dicter en plusieurs fois, relire, corriger, puis insérer.
//!
//! Tant que la fenêtre est visible, chaque dictée (raccourci ou bouton Enregistrer / Pause) arrive
//! dans la fenêtre au lieu d'être collée : elle s'insère à l'endroit du curseur du texte, ce qui
//! permet de revenir en arrière, d'ajouter une phrase au milieu ou de re-dicter un passage.
//! L'arrêt automatique sur silence est coupé (on peut réfléchir sans que l'enregistrement s'arrête),
//! et la pastille ne s'affiche pas : la fenêtre montre déjà l'état.
//!
//! « Insérer » masque la fenêtre (le focus revient à l'application d'origine) puis colle le texte
//! complet par le chemin habituel (`inject::deliver`).

use crate::pipeline::{self, Command};
use crate::state::AppState;
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "dictaphone";
/// Le temps que le gestionnaire de fenêtres rende le focus à l'application d'origine.
const FOCUS_RETURN_DELAY: Duration = Duration::from_millis(350);

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// La fenêtre dictaphone est-elle ouverte (visible) ? Les dictées lui sont alors destinées.
pub fn is_active(app: &AppHandle) -> bool {
    window(app).is_some_and(|w| w.is_visible().unwrap_or(false))
}

/// Ouvre (ou ramène au premier plan) la fenêtre dictaphone.
pub fn open(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = window(app) {
        w.unminimize().ok();
        w.show()?;
        w.set_focus()?;
        return Ok(());
    }
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("dictaphone.html".into()))
        .title("Murmure — Dictaphone")
        .inner_size(620.0, 520.0)
        .min_inner_size(440.0, 340.0)
        .center()
        .focused(true)
        .build()?;
    Ok(())
}

/// Texte d'une dictée, envoyé à la fenêtre (inséré au curseur).
#[derive(Clone, Serialize)]
pub struct DictaphoneText<'a> {
    pub text: &'a str,
}

/// Livre le texte final d'une dictée à la fenêtre.
pub fn push_text(app: &AppHandle, text: &str) {
    if let Err(e) = app.emit_to(LABEL, "dictaphone-text", DictaphoneText { text }) {
        log::warn!("dictaphone : émission du texte : {e}");
    }
}

/// Texte provisoire pendant l'enregistrement (segments déjà transcrits).
pub fn push_partial(app: &AppHandle, text: &str) {
    let _ = app.emit_to(LABEL, "dictaphone-partial", DictaphoneText { text });
}

#[tauri::command]
pub fn dictaphone_open(app: AppHandle) -> Result<(), String> {
    open(&app).map_err(|e| e.to_string())
}

/// Enregistrer / Pause : démarre une dictée ou termine celle en cours (le texte arrive ensuite).
#[tauri::command]
pub fn dictaphone_toggle(app: AppHandle) {
    pipeline::send(&app, Command::Toggle);
}

/// Abandonne l'enregistrement en cours sans rien insérer.
#[tauri::command]
pub fn dictaphone_cancel(app: AppHandle) {
    pipeline::send(&app, Command::Cancel);
}

/// Masque la fenêtre, laisse le focus revenir à l'application d'origine, puis y colle `text`.
#[tauri::command]
pub async fn dictaphone_insert(app: AppHandle, text: String) -> Result<(), String> {
    let text = text.trim_end().to_string();
    if text.is_empty() {
        return Ok(());
    }
    if let Some(w) = window(&app) {
        w.hide().map_err(|e| e.to_string())?;
    }
    // macOS : masquer l'application rend la main à la précédente (une fenêtre masquée ne suffit pas).
    #[cfg(target_os = "macos")]
    if !app.get_webview_window(crate::MAIN_WINDOW).is_some_and(|w| w.is_visible().unwrap_or(false)) {
        let _ = app.hide();
    }
    tokio::time::sleep(FOCUS_RETURN_DELAY).await;
    let settings = app.state::<AppState>().settings();
    let app2 = app.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        crate::inject::deliver(&app2, &text, settings.paste_mode, settings.restore_clipboard, false)
    })
    .await
    .map_err(|e| e.to_string())??;
    match outcome {
        crate::inject::Outcome::CopiedOnly(msg) => Err(msg),
        _ => Ok(()),
    }
}

/// Copie le texte dans le presse-papiers (sans fermer la fenêtre).
#[tauri::command]
pub async fn dictaphone_copy(text: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || crate::inject::copy_to_clipboard(&text)).await.map_err(|e| e.to_string())?
}

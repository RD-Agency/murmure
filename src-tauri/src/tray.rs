//! Icône de barre de menus / zone de notification.

use crate::pipeline::{self, Command};
use crate::state::Phase;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, Wry};

const TRAY_ID: &str = "main";

const ICON_IDLE: &[u8] = include_bytes!("../icons/tray@2x.png");
const ICON_RECORDING: &[u8] = include_bytes!("../icons/tray-recording@2x.png");

/// Entrée de menu dont le libellé change avec l'état.
pub struct TrayHandles {
    toggle: MenuItem<Wry>,
}

fn icon(bytes: &'static [u8]) -> Option<Image<'static>> {
    Image::from_bytes(bytes)
        .map_err(|e| log::warn!("icône du tray illisible : {e}"))
        .ok()
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Démarrer la dictée", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Ouvrir Murmure", true, None::<&str>)?;
    let history = MenuItem::with_id(app, "history", "Historique", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Réglages", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter Murmure", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &open, &history, &settings, &sep, &quit])?;

    let mut builder = tauri::tray::TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Murmure")
        .icon_as_template(true)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => pipeline::send(app, Command::Toggle),
            "open" => show_main(app, None),
            "history" => show_main(app, Some("history")),
            "settings" => show_main(app, Some("settings")),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(img) = icon(ICON_IDLE) {
        builder = builder.icon(img);
    } else if let Some(img) = app.default_window_icon() {
        builder = builder.icon(img.clone());
    }
    builder.build(app)?;
    app.manage(TrayHandles { toggle });
    Ok(())
}

/// Met à jour icône et libellés selon la phase de la dictée.
pub fn update(app: &AppHandle, phase: Phase) {
    let recording = phase == Phase::Recording;
    if let Some(h) = app.try_state::<TrayHandles>() {
        let label = match phase {
            Phase::Recording => "Arrêter la dictée",
            Phase::Transcribing => "Transcription…",
            Phase::Idle => "Démarrer la dictée",
        };
        let _ = h.toggle.set_text(label);
        let _ = h.toggle.set_enabled(phase != Phase::Transcribing);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let bytes = if recording { ICON_RECORDING } else { ICON_IDLE };
        if let Some(img) = icon(bytes) {
            let _ = tray.set_icon_with_as_template(Some(img), true);
        }
        let tip = match phase {
            Phase::Recording => "Murmure — enregistrement…",
            Phase::Transcribing => "Murmure — transcription…",
            Phase::Idle => "Murmure",
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}

/// Affiche la fenêtre principale et, si demandé, navigue vers une vue.
pub fn show_main(app: &AppHandle, route: Option<&str>) {
    crate::show_main_window(app);
    if let Some(r) = route
        && let Err(e) = app.emit("navigate", r)
    {
        log::warn!("émission navigate : {e}");
    }
}

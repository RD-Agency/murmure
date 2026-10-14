// Modules de l'agent audio/moteurs
pub mod audio;
pub mod engine;
pub mod models;
pub mod settings;

// Modules système
pub mod history;
pub mod hotkey;
pub mod inject;
pub mod overlay;
pub mod permissions;
pub mod pipeline;
pub mod state;
pub mod tray;

use crate::engine::RemoteEngine;
use crate::history::HistoryEntry;
use crate::models::ModelInfo;
use crate::pipeline::Command;
use crate::settings::{EngineKind, Settings};
use crate::state::AppState;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt as _;

const MAIN_WINDOW: &str = "main";
/// Argument passé par le lancement automatique : démarrer sans fenêtre.
const HIDDEN_ARG: &str = "--hidden";
/// Argument de ligne de commande : bascule la dictée dans l'instance déjà lancée.
const TOGGLE_ARG: &str = "--toggle";

type CmdResult<T> = Result<T, String>;

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ───────────────────────────── Fenêtre principale ─────────────────────────────

/// Affiche, restaure et met au premier plan la fenêtre principale.
pub fn show_main_window(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Masque la fenêtre principale (l'app reste dans la barre de menus).
fn hide_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.hide();
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

// ───────────────────────────────── Réglages ──────────────────────────────────

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
async fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> CmdResult<()> {
    let old = state.settings();

    // 1. Raccourci d'abord : en cas d'échec, rien n'est enregistré.
    hotkey::validate(&settings.hotkey)?;
    hotkey::reapply(&app, &old.hotkey, &settings.hotkey)?;

    // 2. Persistance.
    let config_dir = state.config_dir.clone();
    let to_save = settings.clone();
    let saved = tauri::async_runtime::spawn_blocking(move || settings::save(&config_dir, &to_save))
        .await
        .map_err(err_str)?;
    if let Err(e) = saved {
        // Rétablit l'ancien raccourci pour rester cohérent avec le disque.
        let _ = hotkey::reapply(&app, &settings.hotkey, &old.hotkey);
        return Err(format!("Impossible d'enregistrer les réglages : {e}"));
    }
    *state.settings.lock() = settings.clone();

    // 3. Lancement à l'ouverture de session.
    if old.launch_at_login != settings.launch_at_login
        && let Err(e) = apply_autostart(&app, settings.launch_at_login)
    {
        log::warn!("autostart : {e}");
    }

    // 4. Moteur local : décharge si le modèle a changé ou si on passe en distant.
    match settings.engine {
        EngineKind::Local => {
            state.unload_engine_unless(&settings.local_model);
            preload_local_engine(&app);
        }
        EngineKind::Remote => {
            if state.engine.lock().take().is_some() {
                log::info!("moteur local déchargé (moteur distant sélectionné)");
            }
        }
    }

    if let Err(e) = app.emit("settings-changed", &settings) {
        log::warn!("émission settings-changed : {e}");
    }
    Ok(())
}

fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let al = app.autolaunch();
    let current = al.is_enabled().unwrap_or(!enabled);
    match (enabled, current) {
        (true, false) => al.enable().map_err(err_str),
        (false, true) => al.disable().map_err(err_str),
        _ => Ok(()),
    }
}

#[tauri::command]
async fn set_api_key(state: State<'_, AppState>, key: String) -> CmdResult<()> {
    let res = tauri::async_runtime::spawn_blocking(move || settings::set_api_key(key.trim()))
        .await
        .map_err(err_str)?;
    state.invalidate_api_key();
    res.map_err(|e| format!("Impossible d'enregistrer la clé API dans le trousseau : {e}"))
}

#[tauri::command]
async fn has_api_key(app: AppHandle) -> CmdResult<bool> {
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().api_key().is_some())
        .await
        .map_err(err_str)
}

#[tauri::command]
async fn test_remote(
    state: State<'_, AppState>,
    endpoint: String,
    api_key: Option<String>,
    model: String,
) -> CmdResult<String> {
    let endpoint = endpoint.trim().to_string();
    if endpoint.is_empty() {
        return Err("Renseignez l'adresse du serveur.".into());
    }
    let api_key = match api_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        Some(k) => Some(k),
        None => state.api_key(),
    };
    let engine = RemoteEngine { endpoint, api_key, model };
    engine.test().await.map_err(|e| format!("Échec du test : {e}"))
}

// ───────────────────────────── Audio et modèles ──────────────────────────────

#[tauri::command]
async fn list_input_devices() -> CmdResult<Vec<audio::AudioDevice>> {
    tauri::async_runtime::spawn_blocking(audio::list_input_devices)
        .await
        .map_err(err_str)
}

#[tauri::command]
async fn list_models(state: State<'_, AppState>) -> CmdResult<Vec<ModelInfo>> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || models::list(&data_dir))
        .await
        .map_err(err_str)
}

#[derive(Clone, Serialize)]
struct DownloadProgress {
    id: String,
    downloaded: u64,
    total: u64,
    done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[tauri::command]
fn download_model(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut downloads = state.downloads.lock();
        if downloads.contains_key(&id) {
            return Err("Ce modèle est déjà en cours de téléchargement.".into());
        }
        downloads.insert(id.clone(), cancel.clone());
    }
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn(async move {
        let progress_app = app.clone();
        let progress_id = id.clone();
        let last = parking_lot::Mutex::new(None::<Instant>);
        let on_progress = move |downloaded: u64, total: u64| {
            // ~10 événements/s maximum, plus le dernier.
            let mut last = last.lock();
            let due = last.is_none_or(|t| t.elapsed() >= Duration::from_millis(100));
            if due || (total > 0 && downloaded >= total) {
                *last = Some(Instant::now());
                let _ = progress_app.emit(
                    "model-download-progress",
                    DownloadProgress { id: progress_id.clone(), downloaded, total, done: false, error: None },
                );
            }
        };
        let result = models::download(&data_dir, &id, cancel.clone(), on_progress).await;
        let state = app.state::<AppState>();
        state.downloads.lock().remove(&id);
        let (downloaded, total, error) = match &result {
            Ok(path) => {
                let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                (size, size, None)
            }
            Err(_) if cancel.load(Ordering::Relaxed) => (0, 0, Some("Téléchargement annulé".to_string())),
            Err(e) => (0, 0, Some(format!("Échec du téléchargement : {e}"))),
        };
        if let Some(e) = &error {
            log::warn!("modèle {id} : {e}");
        } else {
            log::info!("modèle {id} téléchargé");
        }
        let _ = app.emit(
            "model-download-progress",
            DownloadProgress { id: id.clone(), downloaded, total, done: true, error },
        );
        if result.is_ok() {
            preload_local_engine(&app);
        }
    });
    Ok(())
}

#[tauri::command]
fn cancel_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    if let Some(flag) = state.downloads.lock().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
async fn delete_model(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    if state.downloads.lock().contains_key(&id) {
        return Err("Annulez d'abord le téléchargement en cours.".into());
    }
    {
        let mut eng = state.engine.lock();
        if eng.as_ref().is_some_and(|l| l.model_id == id) {
            *eng = None;
        }
    }
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || models::delete(&data_dir, &id))
        .await
        .map_err(err_str)?
        .map_err(|e| format!("Impossible de supprimer le modèle : {e}"))
}

/// Charge en arrière-plan le moteur local si le moteur local est choisi et le modèle présent.
fn preload_local_engine(app: &AppHandle) {
    let state = app.state::<AppState>();
    let s = state.settings();
    if s.engine != EngineKind::Local {
        return;
    }
    let data_dir = state.data_dir.clone();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if models::model_path(&data_dir, &s.local_model).is_some()
            && let Err(e) = pipeline::ensure_local_engine(&app, &s.local_model)
        {
            log::warn!("préchargement du moteur local : {e}");
        }
    });
}

// ───────────────────────────────── Dictée ────────────────────────────────────

#[tauri::command]
fn start_dictation(app: AppHandle) {
    pipeline::send(&app, Command::Start { hands_free: true });
}

#[tauri::command]
fn stop_dictation(app: AppHandle) {
    pipeline::send(&app, Command::Stop);
}

#[tauri::command]
fn cancel_dictation(app: AppHandle) {
    pipeline::send(&app, Command::Cancel);
}

#[tauri::command]
fn toggle_dictation(app: AppHandle) {
    pipeline::send(&app, Command::Toggle);
}

// ──────────────────────────────── Historique ─────────────────────────────────

#[tauri::command]
async fn get_history(state: State<'_, AppState>) -> CmdResult<Vec<HistoryEntry>> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || history::load(&data_dir))
        .await
        .map_err(err_str)
}

#[tauri::command]
async fn delete_history_entry(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || history::delete(&data_dir, &id))
        .await
        .map_err(err_str)?
        .map_err(err_str)
}

#[tauri::command]
async fn clear_history(state: State<'_, AppState>) -> CmdResult<()> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || history::clear(&data_dir))
        .await
        .map_err(err_str)?
        .map_err(err_str)
}

#[tauri::command]
async fn copy_text(text: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || inject::copy_to_clipboard(&text))
        .await
        .map_err(err_str)?
}

// ─────────────────────────────── Permissions ─────────────────────────────────

#[tauri::command]
fn check_permissions() -> permissions::Permissions {
    permissions::check()
}

#[tauri::command]
async fn request_microphone() -> CmdResult<permissions::Permissions> {
    Ok(permissions::request_microphone().await)
}

#[tauri::command]
fn open_permission_settings(app: AppHandle, kind: String) -> CmdResult<()> {
    let url = permissions::settings_url(&kind)?;
    app.opener().open_url(url, None::<&str>).map_err(err_str)
}

#[tauri::command]
fn get_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

// ──────────────────────────────── Démarrage ──────────────────────────────────

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let config_dir = app.path().app_config_dir()?;
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    std::fs::create_dir_all(&data_dir)?;
    log::info!("config : {} — données : {}", config_dir.display(), data_dir.display());

    let settings = settings::load(&config_dir);
    let (tx, rx) = std::sync::mpsc::channel::<Command>();
    app.manage(AppState::new(config_dir, data_dir, settings.clone(), tx));
    app.manage(hotkey::Registered::default());
    pipeline::spawn_worker(handle.clone(), rx);

    tray::create(&handle)?;
    overlay::init(&handle);

    if let Err(e) = hotkey::apply(&handle, &settings.hotkey) {
        log::error!("{e}");
    }
    if let Err(e) = apply_autostart(&handle, settings.launch_at_login) {
        log::warn!("autostart : {e}");
    }
    preload_local_engine(&handle);

    if std::env::args().any(|a| a == HIDDEN_ARG) && settings.onboarding_done {
        hide_main_window(&handle);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,murmure_lib=debug"))
        .format_timestamp_millis()
        .init();

    let app = tauri::Builder::default()
        // single-instance doit être le premier plugin enregistré.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // `murmure --toggle` : bascule la dictée (utile pour lier un raccourci système,
            // p. ex. sous KDE Wayland où les raccourcis globaux X11 ne sont pas reçus).
            if args.iter().any(|a| a == TOGGLE_ARG) {
                pipeline::send(app, Command::Toggle);
            } else {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args([HIDDEN_ARG])
                .app_name("Murmure")
                .build(),
        )
        .setup(setup)
        .on_window_event(|window, event| {
            if window.label() == MAIN_WINDOW
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                // Fermer masque : l'app reste dans la barre de menus. Quitter via le tray.
                api.prevent_close();
                hide_main_window(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            set_api_key,
            has_api_key,
            test_remote,
            list_input_devices,
            list_models,
            download_model,
            cancel_download,
            delete_model,
            start_dictation,
            stop_dictation,
            cancel_dictation,
            toggle_dictation,
            get_history,
            delete_history_entry,
            clear_history,
            copy_text,
            check_permissions,
            request_microphone,
            open_permission_settings,
            get_platform,
        ])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(a) => a,
        Err(e) => {
            log::error!("échec du démarrage de Murmure : {e}");
            std::process::exit(1);
        }
    };

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            // Clic sur l'icône du Dock / relance depuis le Finder.
            show_main_window(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}

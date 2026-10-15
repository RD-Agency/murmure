// SPDX-License-Identifier: AGPL-3.0-or-later
// Modules de l'agent audio/moteurs
pub mod audio;
pub mod audio_file;
pub mod config_transfer;
pub mod engine;
pub mod file_transcription;
pub mod models;
pub mod settings;

// Modules système
pub mod correction;
pub mod history;
pub mod ducking;
pub mod hotkey;
pub mod inject;
pub mod install;
pub mod overlay;
pub mod permissions;
pub mod pipeline;
pub mod postprocess;
pub mod silence;
pub mod state;
pub mod tray;
pub mod punctuation;
pub mod updater;
pub mod wayland;
#[cfg(target_os = "linux")]
pub mod portal;

use crate::engine::RemoteEngine;
use crate::history::{
    AudioUsage, CorrectionResult, ExportFormat, HistoryEntry, HistoryPage, HistoryQuery, HistoryState,
    VocabularySuggestion,
};
use crate::models::ModelInfo;
use crate::pipeline::Command;
use crate::settings::{EngineKind, Settings};
use crate::state::AppState;
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
    apply_settings(&app, &state, settings).await
}

/// Valide, enregistre et applique de nouveaux réglages (interface et import).
async fn apply_settings(app: &AppHandle, state: &AppState, mut settings: Settings) -> CmdResult<()> {
    let app = app.clone();
    let old = state.settings();
    // Champ géré par Rust (portail Wayland) : la copie du front peut être périmée.
    settings.wayland_restore_token = old.wayland_restore_token.clone();

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

    // Plafond de l'audio abaissé : rotation immédiate.
    if settings.audio_cap_bytes() < old.audio_cap_bytes()
        && let Ok(db) = app.state::<HistoryState>().db()
    {
        let cap = settings.audio_cap_bytes();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = db.enforce_audio_cap(cap) {
                log::warn!("plafond de l'audio : {e:#}");
            }
        });
    }

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
            // Passage au local ou nouveau modèle absent du disque : installation automatique.
            if install::settings_trigger(&old, &settings) {
                install::ensure_installed(&app);
            }
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

/// Exporte les réglages : boîte d'enregistrement, puis écriture. `None` si l'utilisateur annule.
#[tauri::command]
async fn export_config(app: AppHandle, state: State<'_, AppState>, include_secrets: bool) -> CmdResult<Option<String>> {
    let settings = state.settings();
    let api_key = if include_secrets { state.api_key() } else { None };
    tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        let Some(path) = app
            .dialog()
            .file()
            .set_title("Exporter les réglages")
            .set_file_name(config_transfer::DEFAULT_FILE_NAME)
            .add_filter("Réglages Murmure", &["json"])
            .blocking_save_file()
            .and_then(|p| p.into_path().ok())
        else {
            return Ok(None);
        };
        let file = config_transfer::export(&settings, api_key.as_deref());
        config_transfer::write(&path, &file)?;
        log::info!("réglages exportés dans {}", path.display());
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(err_str)?
}

/// Importe un fichier de réglages et l'applique. `None` si l'utilisateur annule.
#[tauri::command]
async fn import_config(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<Settings>> {
    let app2 = app.clone();
    let current = state.settings();
    let imported = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        let Some(path) = app2
            .dialog()
            .file()
            .set_title("Importer des réglages")
            .add_filter("Réglages Murmure", &["json"])
            .blocking_pick_file()
            .and_then(|p| p.into_path().ok())
        else {
            return Ok(None);
        };
        config_transfer::read(&path).and_then(|f| config_transfer::merge(&current, &f)).map(Some)
    })
    .await
    .map_err(err_str)??;
    let Some(imported) = imported else { return Ok(None) };
    if let Some(key) = imported.api_key {
        let res = tauri::async_runtime::spawn_blocking(move || settings::set_api_key(&key)).await.map_err(err_str)?;
        state.invalidate_api_key();
        res.map_err(|e| format!("Réglages non importés : clé API impossible à enregistrer dans le trousseau ({e})"))?;
    }
    apply_settings(&app, &state, imported.settings).await?;
    Ok(Some(state.settings()))
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

/// Lance le téléchargement, ou rejoint celui déjà en cours (auto-installation comprise).
#[tauri::command]
fn download_model(app: AppHandle, id: String) -> CmdResult<()> {
    install::start(&app, &id, install::Origin::User).map(|_| ())
}

#[tauri::command]
fn cancel_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.downloads.cancel(&id);
    Ok(())
}

/// Téléchargements en cours (y compris ceux lancés par Rust au démarrage).
#[tauri::command]
fn get_downloads(state: State<'_, AppState>) -> Vec<state::DownloadStatus> {
    state.downloads.snapshot()
}

#[tauri::command]
async fn delete_model(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    if state.downloads.contains(&id) {
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
pub(crate) fn preload_local_engine(app: &AppHandle) {
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

/// Exécute `f` sur la base de l'historique, hors du thread de l'interface.
async fn with_history<T: Send + 'static>(
    history: &HistoryState,
    f: impl FnOnce(&history::HistoryDb) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let db = history.db()?;
    tauri::async_runtime::spawn_blocking(move || f(&db))
        .await
        .map_err(err_str)?
        .map_err(|e| format!("{e:#}"))
}

/// Les dictées les plus récentes (accueil).
#[tauri::command]
async fn get_history(history: State<'_, HistoryState>) -> CmdResult<Vec<HistoryEntry>> {
    with_history(&history, |db| db.recent(history::RECENT_LIMIT)).await
}

#[tauri::command]
async fn search_history(history: State<'_, HistoryState>, query: HistoryQuery) -> CmdResult<HistoryPage> {
    with_history(&history, move |db| db.search(&query)).await
}

#[tauri::command]
async fn delete_history_entry(history: State<'_, HistoryState>, id: String) -> CmdResult<()> {
    with_history(&history, move |db| db.delete(&id)).await
}

#[tauri::command]
async fn clear_history(history: State<'_, HistoryState>) -> CmdResult<()> {
    with_history(&history, |db| db.clear()).await
}

/// Correction manuelle : nouveau texte, paire avant/après et termes corrigés.
#[tauri::command]
async fn correct_history_entry(history: State<'_, HistoryState>, id: String, text: String) -> CmdResult<CorrectionResult> {
    with_history(&history, move |db| db.correct(&id, &text)).await
}

#[tauri::command]
async fn get_vocabulary_suggestions(
    state: State<'_, AppState>,
    history: State<'_, HistoryState>,
) -> CmdResult<Vec<VocabularySuggestion>> {
    let s = state.settings();
    with_history(&history, move |db| db.suggestions(&s.initial_prompt, &s.replacements)).await
}

#[tauri::command]
async fn dismiss_vocabulary_suggestion(history: State<'_, HistoryState>, term: String) -> CmdResult<()> {
    with_history(&history, move |db| db.dismiss_suggestion(&term)).await
}

#[tauri::command]
async fn get_audio_usage(state: State<'_, AppState>, history: State<'_, HistoryState>) -> CmdResult<AudioUsage> {
    let cap = state.settings().audio_cap_bytes();
    with_history(&history, move |db| db.audio_usage(cap)).await
}

#[tauri::command]
async fn delete_all_audio(history: State<'_, HistoryState>) -> CmdResult<()> {
    with_history(&history, |db| db.delete_all_audio()).await
}

/// Audio d'une dictée en WAV, transmis en binaire (ArrayBuffer côté JavaScript).
#[tauri::command]
async fn get_dictation_audio(history: State<'_, HistoryState>, id: String) -> CmdResult<tauri::ipc::Response> {
    with_history(&history, move |db| db.audio_wav(&id)).await.map(tauri::ipc::Response::new)
}

/// Exporte l'historique filtré dans Documents/Murmure ; renvoie le chemin créé.
#[tauri::command]
async fn export_history(
    state: State<'_, AppState>,
    history: State<'_, HistoryState>,
    query: HistoryQuery,
    format: ExportFormat,
) -> CmdResult<String> {
    let base = dirs::document_dir().unwrap_or_else(|| state.data_dir.clone()).join("Murmure");
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S");
    let dest = match format {
        ExportFormat::Csv => base.join(format!("historique-{stamp}.csv")),
        ExportFormat::Json => base.join(format!("historique-{stamp}.json")),
        ExportFormat::Corpus => base.join(format!("corpus-{stamp}")),
    };
    let path = dest.clone();
    let n = with_history(&history, move |db| {
        std::fs::create_dir_all(&base)?;
        db.export(&query, format, &dest)
    })
    .await
    .map_err(|e| format!("Export impossible : {e}"))?;
    if n == 0 {
        let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        return Err(match format {
            ExportFormat::Corpus => "Aucune dictée avec audio à exporter.".into(),
            _ => "Aucune dictée à exporter.".into(),
        });
    }
    Ok(path.to_string_lossy().into_owned())
}

/// Affiche un fichier ou un dossier dans le gestionnaire de fichiers.
#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> CmdResult<()> {
    app.opener().reveal_item_in_dir(path).map_err(err_str)
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

/// Accélération du moteur local : « Metal · Apple M2 », « Vulkan · NVIDIA GeForce RTX 5050 »,
/// « CUDA · … » ou « Processeur » (aucun GPU compilé ou présent).
#[tauri::command]
async fn get_acceleration() -> CmdResult<String> {
    // Premier appel : initialise les backends ggml (instance Vulkan, contexte CUDA), hors thread UI.
    tauri::async_runtime::spawn_blocking(engine::local::acceleration_label)
        .await
        .map_err(err_str)
}

// ─────────────────────────────── Wayland ──────────────────────────────────────

#[tauri::command]
fn get_wayland_status(app: AppHandle) -> wayland::WaylandStatus {
    wayland::status(&app)
}

/// Déclenche les demandes de portail (raccourci global + contrôle du clavier) et renvoie l'état.
#[tauri::command]
async fn wayland_authorize(app: AppHandle) -> CmdResult<wayland::WaylandStatus> {
    #[cfg(target_os = "linux")]
    {
        if !wayland::is_wayland_session() {
            return Ok(wayland::status(&app));
        }
        Ok(portal::authorize(&app).await)
    }
    #[cfg(not(target_os = "linux"))]
    Ok(wayland::status(&app))
}

/// Ouvre le réglage du raccourci du bureau (portail GlobalShortcuts v2).
#[tauri::command]
async fn wayland_configure_shortcut(app: AppHandle) -> CmdResult<()> {
    #[cfg(target_os = "linux")]
    {
        portal::configure_shortcut(&app).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Err("Sans objet sur cette plateforme.".into())
    }
}

/// Relit le raccourci attribué par le bureau (après une modification dans KDE).
#[tauri::command]
async fn wayland_refresh(app: AppHandle) -> wayland::WaylandStatus {
    #[cfg(target_os = "linux")]
    {
        portal::refresh_trigger(&app).await;
        wayland::update(&app, |s| s.paste_fallback = inject::wayland_fallback_tool().map(str::to_string));
    }
    wayland::status(&app)
}

// ───────────────────────────── Mises à jour ──────────────────────────────────

/// Vérifie maintenant ; `None` si l'app est à jour (ou manifeste pas encore publié).
#[tauri::command]
async fn check_update(app: AppHandle) -> CmdResult<Option<updater::UpdateInfo>> {
    updater::check(&app).await
}

/// Télécharge (événements `update-progress`), installe puis relance l'app.
#[tauri::command]
async fn install_update(app: AppHandle) -> CmdResult<()> {
    updater::install(&app).await
}

// ──────────────────────────────── Démarrage ──────────────────────────────────

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let config_dir = app.path().app_config_dir()?;
    // MURMURE_DATA_DIR : dossier de données imposé (tests, CI, déploiement en parc).
    let data_dir = match install::data_dir_override() {
        Some(d) => d,
        None => app.path().app_data_dir()?,
    };
    std::fs::create_dir_all(&config_dir)?;
    std::fs::create_dir_all(&data_dir)?;
    log::info!("config : {} — données : {}", config_dir.display(), data_dir.display());

    let relaunched_hidden = updater::take_relaunch_hidden(&config_dir);
    // Volume resté baissé si Murmure s'est arrêté brutalement pendant une dictée.
    ducking::recover(&config_dir);
    let settings = settings::load(&config_dir);
    app.manage(HistoryState::open(&data_dir));
    let (tx, rx) = std::sync::mpsc::channel::<Command>();
    app.manage(AppState::new(config_dir, data_dir, settings.clone(), tx));
    app.manage(hotkey::Registered::default());
    app.manage(updater::UpdaterState::default());
    app.manage(wayland::WaylandShared::default());
    app.manage(file_transcription::FileJob::default());
    #[cfg(target_os = "linux")]
    app.manage(portal::Portals::default());
    pipeline::spawn_worker(handle.clone(), rx);

    tray::create(&handle)?;
    overlay::init(&handle);

    if let Err(e) = hotkey::apply(&handle, &settings.hotkey) {
        log::error!("{e}");
    }
    // Wayland : raccourci global et collage via les portails XDG.
    #[cfg(target_os = "linux")]
    if wayland::is_wayland_session() {
        log::info!("session Wayland : raccourci et collage via les portails XDG");
        portal::init(&handle, &settings);
    }
    if let Err(e) = apply_autostart(&handle, settings.launch_at_login) {
        log::warn!("autostart : {e}");
    }
    preload_local_engine(&handle);
    // Modèle local absent : installation en arrière-plan (préchargement à la fin).
    install::ensure_installed(&handle);
    // Mises à jour silencieuses : vérification, téléchargement, redémarrage quand l'app est inactive.
    updater::spawn_background(&handle);

    if (relaunched_hidden || std::env::args().any(|a| a == HIDDEN_ARG)) && settings.onboarding_done {
        hide_main_window(&handle);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        // Le plugin updater journalise en `error` un manifeste absent ou un réseau coupé :
        // on le coupe, `updater.rs` trace lui-même ces cas en debug.
        "info,murmure_lib=debug,tauri_plugin_updater=off",
    ))
        .format_timestamp_millis()
        .init();

    // `murmure --install-model [id]` : mode console, AVANT Tauri et le single-instance
    // (aucune fenêtre, fonctionne même si une instance graphique tourne déjà).
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = install::cli(&args) {
        std::process::exit(code);
    }
    // `murmure --export-config fichier.json` / `--import-config fichier.json` : même principe.
    if let Some(code) = config_transfer::cli(&args) {
        std::process::exit(code);
    }

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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
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
            get_downloads,
            delete_model,
            start_dictation,
            stop_dictation,
            cancel_dictation,
            toggle_dictation,
            get_history,
            search_history,
            delete_history_entry,
            clear_history,
            correct_history_entry,
            get_vocabulary_suggestions,
            dismiss_vocabulary_suggestion,
            get_audio_usage,
            delete_all_audio,
            get_dictation_audio,
            export_history,
            reveal_path,
            copy_text,
            check_permissions,
            request_microphone,
            open_permission_settings,
            get_platform,
            get_acceleration,
            check_update,
            install_update,
            get_wayland_status,
            wayland_authorize,
            wayland_configure_shortcut,
            wayland_refresh,
            export_config,
            import_config,
            file_transcription::pick_audio_file,
            file_transcription::transcribe_file,
            file_transcription::cancel_file_transcription,
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
        // Windows : une mise à jour prête s'installe quand on quitte (sans relancer l'app).
        #[cfg(windows)]
        if let tauri::RunEvent::ExitRequested { .. } = event {
            updater::apply_staged(app, false, false);
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = (app, event);
    });
}

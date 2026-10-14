//! Installation automatique du modèle local.
//!
//! - Au démarrage, quand on passe au moteur local ou qu'on change de modèle dans les réglages,
//!   et quand on dicte sans modèle : le modèle choisi (ou le recommandé si l'id est inconnu)
//!   est téléchargé en arrière-plan, puis le moteur est préchargé.
//! - Un seul téléchargement par modèle (registre [`Downloads`]) : un clic dans l'UI pendant
//!   l'auto-installation rejoint le téléchargement en cours au lieu d'échouer.
//! - Mode console `murmure --install-model [id]` : téléchargement avec progression sur stdout,
//!   sans fenêtre, avant l'initialisation de Tauri (et donc du single-instance).

use crate::settings::{self, EngineKind, Settings};
use crate::state::{AppState, Begin, DownloadHandle};
use crate::{models, tray};
use parking_lot::Mutex;
use serde::Serialize;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Argument de ligne de commande : installe un modèle en mode console puis quitte.
pub const INSTALL_ARG: &str = "--install-model";
/// Variable d'environnement : dossier de données (modèles, historique) à utiliser à la place
/// du dossier standard. Pratique pour les tests, la CI et les déploiements en parc.
pub const DATA_DIR_ENV: &str = "MURMURE_DATA_DIR";
/// Identifiant de l'app (doit rester égal à `identifier` dans `tauri.conf.json`).
const IDENTIFIER: &str = "fr.rdagency.murmure";

/// Message affiché quand une dictée déclenche l'installation.
pub const INSTALL_STARTED_MSG: &str = "Installation du modèle…";

/// Payload de l'événement `model-download-progress`.
#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub id: String,
    pub downloaded: u64,
    pub total: u64,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Qui a demandé le téléchargement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Bouton « Télécharger » de l'UI.
    User,
    /// Auto-installation (démarrage, réglages, dictée).
    Auto,
}

// ───────────────────────────── Logique pure ─────────────────────────────

/// Modèle à télécharger automatiquement, ou `None` s'il n'y a rien à faire
/// (moteur distant, ou modèle déjà sur disque). Un id inconnu vise le modèle recommandé.
pub fn auto_install_target(
    engine: EngineKind,
    local_model: &str,
    is_present: impl Fn(&str) -> bool,
) -> Option<&'static str> {
    if engine != EngineKind::Local {
        return None;
    }
    let id = models::resolve_id(local_model);
    (!is_present(id)).then_some(id)
}

/// Vrai si un enregistrement de réglages doit déclencher l'auto-installation :
/// passage au moteur local, ou changement de modèle local.
pub fn settings_trigger(old: &Settings, new: &Settings) -> bool {
    new.engine == EngineKind::Local && (old.engine != EngineKind::Local || old.local_model != new.local_model)
}

/// Message de la pastille quand on dicte sans modèle prêt.
/// `in_progress` : pourcentage du téléchargement en cours, s'il y en a un.
pub fn dictation_message(in_progress: Option<u8>) -> String {
    match in_progress {
        Some(pct) => format!("Installation du modèle en cours ({pct} %) — réessayez dans un instant"),
        None => INSTALL_STARTED_MSG.to_string(),
    }
}

/// Extrait la valeur de `--install-model` : `None` = option absente,
/// `Some(None)` = option sans id, `Some(Some(id))` = id explicite.
pub fn parse_install_arg(args: &[String]) -> Option<Option<String>> {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == INSTALL_ARG {
            return Some(it.next().filter(|v| !v.starts_with('-')).cloned());
        }
        if let Some(v) = a.strip_prefix(INSTALL_ARG).and_then(|r| r.strip_prefix('=')) {
            return Some(Some(v.to_string()).filter(|v| !v.is_empty()));
        }
    }
    None
}

/// Dossier de données imposé par [`DATA_DIR_ENV`], s'il est défini et non vide.
pub fn data_dir_override() -> Option<PathBuf> {
    std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

// ─────────────────────────── Dans l'application ───────────────────────────

/// Lance le téléchargement de `id`, ou rejoint celui déjà en cours.
/// Renvoie `true` si un téléchargement était déjà en cours.
pub fn start(app: &AppHandle, id: &str, origin: Origin) -> Result<bool, String> {
    if !models::is_known(id) {
        return Err(format!("Modèle inconnu : {id}"));
    }
    let state = app.state::<AppState>();
    let handle = match state.downloads.begin(id, models::expected_size(id).unwrap_or(0)) {
        Begin::Joined(_) => {
            log::info!("modèle {id} : téléchargement déjà en cours, on le rejoint ({origin:?})");
            return Ok(true);
        }
        Begin::Started(h) => h,
    };
    log::info!("modèle {id} : téléchargement lancé ({origin:?})");
    let app = app.clone();
    let id = id.to_string();
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn(async move { run_download(app, id, handle, data_dir).await });
    Ok(false)
}

async fn run_download(app: AppHandle, id: String, handle: DownloadHandle, data_dir: PathBuf) {
    let progress_app = app.clone();
    let progress_id = id.clone();
    let progress_handle = handle.clone();
    let last = Mutex::new(None::<Instant>);
    let on_progress = move |downloaded: u64, total: u64| {
        progress_handle.set_progress(downloaded, total);
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
    let cancel = handle.cancel.clone();
    let result = models::download(&data_dir, &id, cancel.clone(), on_progress).await;
    let state = app.state::<AppState>();
    state.downloads.finish(&id);

    let cancelled = cancel.load(Ordering::Relaxed)
        || result
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<models::DownloadError>())
            .is_some_and(|e| matches!(e, models::DownloadError::Cancelled));
    let (downloaded, total, error) = match &result {
        Ok(path) => {
            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            (size, size, None)
        }
        Err(_) if cancelled => (0, 0, Some("Téléchargement annulé".to_string())),
        Err(e) => (0, 0, Some(format!("Échec du téléchargement : {e}"))),
    };
    match &error {
        Some(e) => log::warn!("modèle {id} : {e}"),
        None => log::info!("modèle {id} installé"),
    }
    let _ = app.emit(
        "model-download-progress",
        DownloadProgress { id: id.clone(), downloaded, total, done: true, error: error.clone() },
    );
    if result.is_ok() {
        adopt_if_missing(&app, &id);
        crate::preload_local_engine(&app);
    } else if !cancelled {
        // Seul cas où l'on ouvre la fenêtre de force : l'installation a échoué.
        tray::show_main(&app, Some("home"));
    }
}

/// Après une installation réussie : si le modèle choisi n'est pas sur disque
/// (id inconnu, ou modèle supprimé), on sélectionne celui qu'on vient d'installer.
fn adopt_if_missing(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let updated = {
        let mut s = state.settings.lock();
        if s.local_model == id || models::model_path(&state.data_dir, &s.local_model).is_some() {
            return;
        }
        s.local_model = id.to_string();
        s.clone()
    };
    log::info!("modèle local sélectionné : {id}");
    if let Err(e) = settings::save(&state.config_dir, &updated) {
        log::warn!("enregistrement des réglages : {e}");
    }
    let _ = app.emit("settings-changed", &updated);
}

/// Démarre l'installation si le moteur local est choisi et que son modèle manque.
/// Renvoie l'id visé si un téléchargement a été lancé ou rejoint.
pub fn ensure_installed(app: &AppHandle) -> Option<&'static str> {
    let state = app.state::<AppState>();
    let s = state.settings();
    let id = auto_install_target(s.engine, &s.local_model, |id| models::model_path(&state.data_dir, id).is_some())?;
    if !models::is_known(&s.local_model) {
        log::warn!("modèle « {} » inconnu : installation du modèle recommandé {id}", s.local_model);
    }
    match start(app, id, Origin::Auto) {
        Ok(_) => Some(id),
        Err(e) => {
            log::warn!("auto-installation impossible : {e}");
            None
        }
    }
}

/// Dictée demandée sans modèle prêt : message à afficher (et téléchargement lancé si besoin).
/// `Err` si l'installation ne peut même pas démarrer.
pub fn on_dictation_without_model(app: &AppHandle, settings: &Settings) -> Result<String, String> {
    let state = app.state::<AppState>();
    let id = models::resolve_id(&settings.local_model);
    if let Some(h) = state.downloads.get(id) {
        return Ok(dictation_message(Some(h.percent())));
    }
    match start(app, id, Origin::Auto)? {
        // Course : lancé entre-temps par un autre chemin.
        true => Ok(dictation_message(state.downloads.get(id).map(|h| h.percent()))),
        false => Ok(dictation_message(None)),
    }
}

// ───────────────────────────── Mode console ─────────────────────────────

/// Si `--install-model` figure dans `args`, installe le modèle en console et renvoie
/// le code de sortie ; sinon `None` (démarrage normal de l'app).
pub fn cli(args: &[String]) -> Option<i32> {
    let requested = parse_install_arg(args)?;
    Some(run_cli(requested))
}

fn run_cli(requested: Option<String>) -> i32 {
    let Some(data_dir) = data_dir_override().or_else(|| dirs::data_dir().map(|d| d.join(IDENTIFIER))) else {
        eprintln!("Dossier de données introuvable : définissez {DATA_DIR_ENV}.");
        return 1;
    };
    let id = match requested {
        Some(id) if models::is_known(&id) => id,
        Some(id) => {
            eprintln!("Modèle inconnu : {id}");
            eprintln!("Modèles disponibles : {}", models::ids().collect::<Vec<_>>().join(", "));
            return 2;
        }
        None => {
            // Sans id : le modèle des réglages (ou le recommandé).
            let configured = dirs::config_dir()
                .map(|d| settings::load(&d.join(IDENTIFIER)).local_model)
                .unwrap_or_default();
            models::resolve_id(&configured).to_string()
        }
    };

    println!("Murmure — installation du modèle {id}");
    println!("Dossier : {}", models::models_dir(&data_dir).display());
    if let Some(p) = models::model_path(&data_dir, &id) {
        println!("Déjà installé : {}", p.display());
        return 0;
    }
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Impossible de démarrer le runtime : {e}");
            return 1;
        }
    };
    let tty = std::io::stdout().is_terminal();
    let printer = ConsoleProgress::new(tty);
    let started = Instant::now();
    let result = rt.block_on(models::download(
        &data_dir,
        &id,
        Arc::new(AtomicBool::new(false)),
        move |d, t| printer.update(d, t),
    ));
    if tty {
        println!();
    }
    match result {
        Ok(path) => {
            println!(
                "Modèle installé et vérifié (taille + SHA256) en {} s : {}",
                started.elapsed().as_secs(),
                path.display()
            );
            0
        }
        Err(e) => {
            eprintln!("Échec de l'installation : {e}");
            1
        }
    }
}

/// Affichage de la progression : ligne réécrite sur un terminal, une ligne tous les 5 % sinon (logs).
struct ConsoleProgress {
    tty: bool,
    last_step: Mutex<Option<u64>>,
}

impl ConsoleProgress {
    fn new(tty: bool) -> Self {
        Self { tty, last_step: Mutex::new(None) }
    }

    fn update(&self, downloaded: u64, total: u64) {
        let mb = |b: u64| b / 1_048_576;
        let pct = if total > 0 { downloaded.min(total) * 100 / total } else { 0 };
        let line = format!("{pct:>3} % · {} / {} Mo", mb(downloaded), mb(total));
        let mut out = std::io::stdout().lock();
        if self.tty {
            let _ = write!(out, "\r{line}   ");
            let _ = out.flush();
        } else {
            let step = pct / 5;
            let mut last = self.last_step.lock();
            if *last != Some(step) {
                *last = Some(step);
                let _ = writeln!(out, "{line}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn decision_de_telechargement() {
        let none = |_: &str| false;
        let all = |_: &str| true;
        assert_eq!(auto_install_target(EngineKind::Remote, "small-q5_1", none), None);
        assert_eq!(auto_install_target(EngineKind::Local, "small-q5_1", all), None);
        assert_eq!(auto_install_target(EngineKind::Local, "small-q5_1", none), Some("small-q5_1"));
        // Id inconnu : le recommandé, sauf s'il est déjà là.
        assert_eq!(auto_install_target(EngineKind::Local, "n-importe", none), Some(models::RECOMMENDED_ID));
        assert_eq!(
            auto_install_target(EngineKind::Local, "n-importe", |id| id == models::RECOMMENDED_ID),
            None
        );
    }

    #[test]
    fn declenchement_par_les_reglages() {
        let base = Settings { engine: EngineKind::Remote, ..Settings::default() };
        let local = Settings { engine: EngineKind::Local, ..base.clone() };
        assert!(settings_trigger(&base, &local), "passage au moteur local");
        assert!(!settings_trigger(&local, &base), "passage au distant");
        assert!(!settings_trigger(&local, &Settings { sounds: !local.sounds, ..local.clone() }), "autre réglage");
        let other_model = Settings { local_model: "small-q5_1".into(), ..local.clone() };
        assert!(settings_trigger(&local, &other_model), "changement de modèle");
    }

    #[test]
    fn messages_de_dictee() {
        assert_eq!(dictation_message(None), "Installation du modèle…");
        assert_eq!(
            dictation_message(Some(42)),
            "Installation du modèle en cours (42 %) — réessayez dans un instant"
        );
    }

    #[test]
    fn option_install_model() {
        assert_eq!(parse_install_arg(&args(&["murmure"])), None);
        assert_eq!(parse_install_arg(&args(&["murmure", "--toggle"])), None);
        assert_eq!(parse_install_arg(&args(&["murmure", "--install-model"])), Some(None));
        assert_eq!(
            parse_install_arg(&args(&["murmure", "--install-model", "small-q5_1"])),
            Some(Some("small-q5_1".into()))
        );
        assert_eq!(parse_install_arg(&args(&["murmure", "--install-model", "--hidden"])), Some(None));
        assert_eq!(
            parse_install_arg(&args(&["murmure", "--install-model=small-q5_1"])),
            Some(Some("small-q5_1".into()))
        );
        assert_eq!(parse_install_arg(&args(&["murmure", "--install-model="])), Some(None));
        // argv[0] n'est jamais interprété.
        assert_eq!(parse_install_arg(&args(&["--install-model"])), None);
    }

    #[test]
    fn identifiant_synchronise_avec_tauri_conf() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], IDENTIFIER);
    }
}

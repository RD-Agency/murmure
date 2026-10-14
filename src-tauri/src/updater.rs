//! Mises à jour automatiques (`tauri-plugin-updater`).
//!
//! - Vérification silencieuse 10 s après le démarrage puis toutes les 6 h, jamais pendant une
//!   dictée (on attend le retour à l'état « idle »). Aucune vérification en mode dev.
//! - Une mise à jour trouvée est seulement annoncée (événement `update-available` + entrée du
//!   tray) : rien n'est téléchargé sans action de l'utilisateur.
//! - Manifeste absent (404), réseau coupé, plateforme absente du manifeste : log debug uniquement.

use crate::state::{AppState, Phase};
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(10);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// Pendant une dictée, on réessaie à cet intervalle.
const BUSY_RETRY: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Payload de `update-available` et retour de `check_update`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

/// Payload de `update-progress` (octets cumulés ; `total` = 0 si inconnu).
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: u64,
}

/// État géré par Tauri : dernière mise à jour trouvée + verrou d'installation.
#[derive(Default)]
pub struct UpdaterState {
    pending: Mutex<Option<Update>>,
    installing: AtomicBool,
}

/// Issue d'une vérification, avant traduction pour l'appelant.
enum Outcome {
    Available(Box<Update>),
    UpToDate,
    /// Erreur réseau ou inattendue (message lisible).
    Failed(String),
}

fn info_of(u: &Update) -> UpdateInfo {
    UpdateInfo { version: u.version.clone(), notes: u.body.clone().filter(|n| !n.trim().is_empty()) }
}

/// Erreurs « normales » : manifeste pas encore publié, plateforme absente → pas de mise à jour.
fn is_benign(e: &UpdaterError) -> bool {
    matches!(
        e,
        UpdaterError::ReleaseNotFound | UpdaterError::TargetNotFound(_) | UpdaterError::TargetsNotFound(_)
    )
}

fn is_network(e: &UpdaterError) -> bool {
    matches!(e, UpdaterError::Reqwest(_) | UpdaterError::Network(_) | UpdaterError::Io(_))
}

async fn run_check(app: &AppHandle) -> Outcome {
    let updater = match app.updater_builder().timeout(REQUEST_TIMEOUT).build() {
        Ok(u) => u,
        Err(e) => return Outcome::Failed(format!("Mise à jour non configurée : {e}")),
    };
    match updater.check().await {
        Ok(Some(update)) => Outcome::Available(Box::new(update)),
        Ok(None) => Outcome::UpToDate,
        Err(e) if is_benign(&e) => {
            log::debug!("mise à jour : aucun manifeste exploitable ({e})");
            Outcome::UpToDate
        }
        Err(e) if is_network(&e) => {
            log::debug!("mise à jour : serveur injoignable ({e})");
            Outcome::Failed("Impossible de joindre le serveur de mises à jour. Vérifiez votre connexion.".into())
        }
        Err(e) => {
            log::debug!("mise à jour : vérification impossible ({e})");
            Outcome::Failed(format!("Vérification impossible : {e}"))
        }
    }
}

/// Mémorise la mise à jour trouvée ; l'annonce (événement + tray) si elle est nouvelle.
fn remember(app: &AppHandle, update: Update) -> UpdateInfo {
    let info = info_of(&update);
    let state = app.state::<UpdaterState>();
    let is_new = {
        let mut pending = state.pending.lock();
        let is_new = pending.as_ref().is_none_or(|p| p.version != update.version);
        *pending = Some(update);
        is_new
    };
    if is_new {
        log::info!("mise à jour disponible : {}", info.version);
        crate::tray::show_update(app, &info.version);
    }
    if let Err(e) = app.emit("update-available", &info) {
        log::warn!("émission update-available : {e}");
    }
    info
}

/// Vérification demandée par l'utilisateur (`check_update`).
pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    if cfg!(debug_assertions) {
        log::debug!("mise à jour : vérification ignorée en mode dev");
        return Ok(None);
    }
    match run_check(app).await {
        Outcome::Available(u) => Ok(Some(remember(app, *u))),
        Outcome::UpToDate => Ok(None),
        Outcome::Failed(msg) => Err(msg),
    }
}

/// Boucle de fond : 10 s après le démarrage, puis toutes les 6 h. Silencieuse.
pub fn spawn_background(app: &AppHandle) {
    if cfg!(debug_assertions) {
        log::debug!("mise à jour : vérification automatique désactivée en mode dev");
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            // Jamais pendant une dictée.
            while app.state::<AppState>().phase() != Phase::Idle {
                tokio::time::sleep(BUSY_RETRY).await;
            }
            if !app.state::<UpdaterState>().installing.load(Ordering::Relaxed) {
                match run_check(&app).await {
                    Outcome::Available(u) => {
                        remember(&app, *u);
                    }
                    Outcome::UpToDate => log::debug!("mise à jour : Murmure est à jour"),
                    Outcome::Failed(msg) => log::debug!("mise à jour : {msg}"),
                }
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

/// Libère le verrou d'installation même en cas d'erreur.
struct InstallGuard<'a>(&'a AtomicBool);
impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Télécharge (événements `update-progress`), installe puis relance l'app (`install_update`).
pub async fn install(app: &AppHandle) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("Les mises à jour ne s'installent pas en mode développement.".into());
    }
    if app.state::<AppState>().phase() != Phase::Idle {
        return Err("Terminez la dictée en cours avant d'installer la mise à jour.".into());
    }
    let state = app.state::<UpdaterState>();
    if state.installing.swap(true, Ordering::Relaxed) {
        return Err("Installation déjà en cours.".into());
    }
    let _guard = InstallGuard(&state.installing);

    // Mise à jour connue, sinon nouvelle vérification.
    let cached = state.pending.lock().clone();
    let update = match cached {
        Some(u) => u,
        None => match run_check(app).await {
            Outcome::Available(u) => {
                remember(app, (*u).clone());
                *u
            }
            Outcome::UpToDate => return Err("Aucune mise à jour disponible.".into()),
            Outcome::Failed(msg) => return Err(msg),
        },
    };

    crate::tray::set_update_installing(app, true);
    let progress_app = app.clone();
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now() - Duration::from_secs(1);
    let result = update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let total = total.unwrap_or(0);
                // ~10 événements/s suffisent pour la jauge.
                if last_emit.elapsed() >= Duration::from_millis(100) || (total > 0 && downloaded >= total) {
                    last_emit = std::time::Instant::now();
                    let _ = progress_app.emit("update-progress", UpdateProgress { downloaded, total });
                }
            },
            || log::info!("mise à jour téléchargée, installation…"),
        )
        .await;

    if let Err(e) = result {
        log::warn!("échec de l'installation de la mise à jour : {e}");
        crate::tray::set_update_installing(app, false);
        crate::tray::show_update(app, &update.version);
        return Err(if is_network(&e) {
            "Téléchargement interrompu. Vérifiez votre connexion et réessayez.".to_string()
        } else {
            format!("Échec de l'installation : {e}")
        });
    }

    log::info!("mise à jour {} installée, redémarrage", update.version);
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifeste_absent_est_benin() {
        assert!(is_benign(&UpdaterError::ReleaseNotFound));
        assert!(is_benign(&UpdaterError::TargetsNotFound(vec!["darwin-aarch64".into()])));
        assert!(!is_benign(&UpdaterError::EmptyEndpoints));
    }

    #[test]
    fn erreurs_reseau() {
        assert!(is_network(&UpdaterError::Network("timeout".into())));
        assert!(!is_network(&UpdaterError::ReleaseNotFound));
    }

    #[test]
    fn payloads_serialises() {
        let i = UpdateInfo { version: "0.2.0".into(), notes: None };
        assert_eq!(serde_json::to_string(&i).unwrap(), r#"{"version":"0.2.0","notes":null}"#);
        let p = UpdateProgress { downloaded: 5, total: 10 };
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"downloaded":5,"total":10}"#);
    }
}

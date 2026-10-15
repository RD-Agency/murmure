// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mises à jour automatiques et silencieuses (`tauri-plugin-updater`).
//!
//! - Vérification 10 s après le démarrage puis toutes les heures, jamais pendant une dictée.
//!   Aucune vérification en mode dev.
//! - Mise à jour trouvée : téléchargée en arrière-plan sans rien afficher (signature vérifiée
//!   par le plugin). macOS et AppImage : installée tout de suite sur disque, active au prochain
//!   lancement. Windows : gardée en mémoire, l'installeur NSIS ferme puis relance l'app.
//! - Redémarrage automatique dès que l'app est inactive (aucune dictée, aucun téléchargement de
//!   modèle, fenêtre principale masquée) depuis [`IDLE_BEFORE_RESTART`] : relance masquée via un
//!   marqueur `relaunch-hidden`, l'utilisateur ne voit rien. Windows installe aussi en quittant.
//! - Linux .deb / .rpm : l'installation exige root, la mise à jour est seulement annoncée.
//! - Manifeste absent (404), réseau coupé, plateforme absente du manifeste : log debug uniquement.

use crate::state::{AppState, Phase};
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(10);
const CHECK_INTERVAL: Duration = Duration::from_secs(3600);
/// Pas de la boucle de fond (vérification due, inactivité, redémarrage).
const TICK: Duration = Duration::from_secs(30);
/// Inactivité continue exigée avant un redémarrage automatique.
pub const IDLE_BEFORE_RESTART: Duration = Duration::from_secs(120);
/// Marqueur posé avant un redémarrage automatique : relancer sans fenêtre.
const RELAUNCH_HIDDEN_MARKER: &str = "relaunch-hidden";
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

/// Mise à jour téléchargée, prête à être appliquée par un redémarrage.
struct Staged {
    update: Update,
    /// Octets de l'installeur (Windows) ; `None` : déjà installée sur disque (macOS, AppImage).
    bytes: Option<Vec<u8>>,
}

/// État géré par Tauri : dernière mise à jour trouvée, mise à jour prête, verrou d'installation.
#[derive(Default)]
pub struct UpdaterState {
    pending: Mutex<Option<Update>>,
    staged: Mutex<Option<Staged>>,
    installing: AtomicBool,
}

/// Installation sans intervention possible sur cette plateforme / ce format de paquet.
fn silent_install_supported() -> bool {
    if cfg!(any(target_os = "macos", windows)) {
        return true;
    }
    cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some()
}

/// Peut-on redémarrer sans déranger : app inactive depuis assez longtemps et fenêtre masquée.
pub fn restart_allowed(idle_for: Option<Duration>, window_visible: bool, downloading_model: bool) -> bool {
    !window_visible && !downloading_model && idle_for.is_some_and(|d| d >= IDLE_BEFORE_RESTART)
}

/// Lancement après un redémarrage automatique : consomme le marqueur, `true` s'il existait.
pub fn take_relaunch_hidden(config_dir: &std::path::Path) -> bool {
    std::fs::remove_file(config_dir.join(RELAUNCH_HIDDEN_MARKER)).is_ok()
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
    let staged = app.state::<UpdaterState>().staged.lock().as_ref().map(|st| info_of(&st.update));
    if let Some(info) = staged {
        let _ = app.emit("update-ready", &info);
        return Ok(Some(info));
    }
    match run_check(app).await {
        Outcome::Available(u) => Ok(Some(remember(app, *u))),
        Outcome::UpToDate => Ok(None),
        Outcome::Failed(msg) => Err(msg),
    }
}

/// Boucle de fond : vérifie (10 s puis toutes les heures), télécharge et met en place en
/// silence, puis redémarre dès que l'app est inactive.
pub fn spawn_background(app: &AppHandle) {
    if cfg!(debug_assertions) {
        log::debug!("mise à jour : vérification automatique désactivée en mode dev");
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        let mut next_check = Instant::now();
        let mut idle = IdleTracker::default();
        loop {
            let (phase, generation) = activity(&app);
            idle.observe(phase, generation);
            let staged = app.state::<UpdaterState>().staged.lock().is_some();
            let installing = app.state::<UpdaterState>().installing.load(Ordering::Relaxed);

            if !staged && !installing && phase == Phase::Idle && Instant::now() >= next_check {
                next_check = Instant::now() + CHECK_INTERVAL;
                match run_check(&app).await {
                    Outcome::Available(u) if silent_install_supported() => stage(&app, *u).await,
                    Outcome::Available(u) => {
                        remember(&app, *u);
                    }
                    Outcome::UpToDate => log::debug!("mise à jour : Murmure est à jour"),
                    Outcome::Failed(msg) => log::debug!("mise à jour : {msg}"),
                }
            } else if staged
                && restart_allowed(idle.idle_for(), main_window_visible(&app), downloading_model(&app))
            {
                log::info!("mise à jour prête et app inactive : redémarrage silencieux");
                apply_staged(&app, true, true);
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// Suit depuis quand l'app est inactive. Une dictée (même brève, entre deux passages de la
/// boucle) incrémente `generation` et remet le compteur à zéro.
#[derive(Default)]
struct IdleTracker {
    since: Option<Instant>,
    generation: u64,
}

impl IdleTracker {
    fn observe(&mut self, phase: Phase, generation: u64) {
        if phase != Phase::Idle {
            self.since = None;
        } else if self.since.is_none() || generation != self.generation {
            self.since = Some(Instant::now());
        }
        self.generation = generation;
    }

    fn idle_for(&self) -> Option<Duration> {
        self.since.map(|t| t.elapsed())
    }
}

fn activity(app: &AppHandle) -> (Phase, u64) {
    let state = app.state::<AppState>();
    let d = state.dictation.lock();
    (d.phase, d.generation)
}

fn main_window_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false))
}

fn downloading_model(app: &AppHandle) -> bool {
    !app.state::<AppState>().downloads.snapshot().is_empty()
}

/// Télécharge en silence puis installe sur disque (macOS, AppImage) ou garde l'installeur
/// en mémoire (Windows). En cas d'échec, la mise à jour est annoncée comme avant.
async fn stage(app: &AppHandle, update: Update) {
    let state = app.state::<UpdaterState>();
    if state.installing.swap(true, Ordering::Relaxed) {
        return;
    }
    let _guard = InstallGuard(&state.installing);
    log::info!("mise à jour {} trouvée, téléchargement silencieux", update.version);
    let bytes = match update.download(|_, _| {}, || {}).await {
        Ok(b) => b,
        Err(e) => {
            log::warn!("mise à jour : téléchargement impossible ({e})");
            remember(app, update);
            return;
        }
    };
    let bytes = if cfg!(windows) {
        Some(bytes)
    } else {
        if let Err(e) = update.install(&bytes) {
            log::warn!("mise à jour : installation impossible ({e})");
            remember(app, update);
            return;
        }
        None
    };
    let info = info_of(&update);
    log::info!("mise à jour {} prête, appliquée au prochain redémarrage", info.version);
    *state.pending.lock() = None;
    *state.staged.lock() = Some(Staged { update, bytes });
    crate::tray::show_update_ready(app, &info.version);
    if let Err(e) = app.emit("update-ready", &info) {
        log::warn!("émission update-ready : {e}");
    }
}

/// Applique la mise à jour prête : redémarre sur la nouvelle version (macOS, AppImage) ou lance
/// l'installeur (Windows, qui quitte l'app). `hidden` : relance sans fenêtre. Sans effet si
/// aucune mise à jour n'est prête.
pub fn apply_staged(app: &AppHandle, hidden: bool, restart: bool) {
    let Some(staged) = app.state::<UpdaterState>().staged.lock().take() else { return };
    if hidden {
        let marker = app.state::<AppState>().config_dir.join(RELAUNCH_HIDDEN_MARKER);
        if let Err(e) = std::fs::write(&marker, b"") {
            log::warn!("mise à jour : marqueur de relance masquée : {e}");
        }
    }
    match staged.bytes {
        Some(bytes) => {
            let update = staged.update.restart_after_install(restart);
            if let Err(e) = update.install(&bytes) {
                log::warn!("mise à jour : échec de l'installeur ({e})");
                let _ = std::fs::remove_file(app.state::<AppState>().config_dir.join(RELAUNCH_HIDDEN_MARKER));
            }
        }
        None if restart => app.restart(),
        None => {}
    }
}

/// Libère le verrou d'installation même en cas d'erreur.
struct InstallGuard<'a>(&'a AtomicBool);
impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Bouton « Installer » / « Redémarrer maintenant » (`install_update`) : applique la mise à jour
/// prête, sinon télécharge (événements `update-progress`), installe puis relance.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("Les mises à jour ne s'installent pas en mode développement.".into());
    }
    if app.state::<AppState>().phase() != Phase::Idle {
        return Err("Terminez la dictée en cours avant d'installer la mise à jour.".into());
    }
    if app.state::<UpdaterState>().staged.lock().is_some() {
        apply_staged(app, false, true);
        return Ok(());
    }
    let state = app.state::<UpdaterState>();
    if state.installing.swap(true, Ordering::Relaxed) {
        return Err("Mise à jour déjà en cours de téléchargement.".into());
    }
    let _guard = InstallGuard(&state.installing);

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
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let result = update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let total = total.unwrap_or(0);
                if last_emit.elapsed() >= Duration::from_millis(100) || (total > 0 && downloaded >= total) {
                    last_emit = Instant::now();
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
    fn redemarrage_seulement_inactif_et_masque() {
        let long = Some(IDLE_BEFORE_RESTART);
        assert!(restart_allowed(long, false, false));
        assert!(!restart_allowed(long, true, false), "fenêtre visible");
        assert!(!restart_allowed(long, false, true), "modèle en téléchargement");
        assert!(!restart_allowed(Some(Duration::from_secs(10)), false, false), "inactif depuis peu");
        assert!(!restart_allowed(None, false, false), "dictée en cours");
    }

    #[test]
    fn une_dictee_remet_l_inactivite_a_zero() {
        let mut t = IdleTracker::default();
        t.observe(Phase::Idle, 0);
        t.since = Some(Instant::now() - IDLE_BEFORE_RESTART);
        t.observe(Phase::Idle, 0);
        assert!(t.idle_for().unwrap() >= IDLE_BEFORE_RESTART);
        t.observe(Phase::Idle, 1);
        assert!(t.idle_for().unwrap() < Duration::from_secs(1));
        t.observe(Phase::Recording, 2);
        assert!(t.idle_for().is_none());
    }

    #[test]
    fn marqueur_de_relance_consomme_une_fois() {
        let dir = std::env::temp_dir().join(format!("murmure-relaunch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!take_relaunch_hidden(&dir));
        std::fs::write(dir.join(RELAUNCH_HIDDEN_MARKER), b"").unwrap();
        assert!(take_relaunch_hidden(&dir));
        assert!(!take_relaunch_hidden(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn payloads_serialises() {
        let i = UpdateInfo { version: "0.2.0".into(), notes: None };
        assert_eq!(serde_json::to_string(&i).unwrap(), r#"{"version":"0.2.0","notes":null}"#);
        let p = UpdateProgress { downloaded: 5, total: 10 };
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"downloaded":5,"total":10}"#);
    }
}

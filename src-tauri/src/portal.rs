//! Portails XDG sous Wayland (Linux uniquement) :
//!
//! - `org.freedesktop.portal.GlobalShortcuts` : raccourci global `dictation`. `Activated` = appui,
//!   `Deactivated` = relâchement, injectés dans la machine d'états de `hotkey.rs` (modes hybride,
//!   maintien, bascule identiques à X11). Le raccourci attribué par l'utilisateur (modifiable dans
//!   la boîte de dialogue du bureau) est relu via la réponse de `BindShortcuts`, `ListShortcuts`
//!   et le signal `ShortcutsChanged`.
//! - `org.freedesktop.portal.RemoteDesktop` : clavier seul, `persist_mode` = persistant, jeton de
//!   restauration dans les réglages (`wayland_restore_token`) → l'autorisation n'est demandée qu'une
//!   fois. Le collage envoie Ctrl+V par `NotifyKeyboardKeysym`.
//!
//! Identité de l'app : une app hors bac à sable n'a pas d'app id pour le portail, sauf si elle est
//! lancée par le bureau dans une unité systemd `app-Murmure@….service`. Sans app id, KDE crée un
//! composant `token_…` à chaque lancement (raccourci redemandé à chaque fois). On s'enregistre donc
//! via `org.freedesktop.host.portal.Registry.Register("Murmure")` (xdg-desktop-portal ≥ 1.19), qui
//! exige un fichier `Murmure.desktop` (installé par les paquets .deb/.rpm ; créé dans
//! `~/.local/share/applications` s'il manque, cas de l'AppImage).
//!
//! Toutes les connexions D-Bus passent par une même `zbus::Connection` : l'enregistrement de l'app
//! id est lié à la connexion et doit précéder tout autre appel de portail.

use crate::hotkey::{self, KeyEvent};
use crate::settings::Settings;
use crate::state::AppState;
use crate::wayland::{self, PortalState, SHORTCUT_ID};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut, Shortcut};
use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, ResponseError, Session, SessionPortal};
use ashpd::enumflags2::BitFlags;
use ashpd::zbus;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::{Mutex, OnceCell};

/// App id présenté au portail = nom du fichier .desktop installé par Tauri (`productName`).
pub const APP_ID: &str = "Murmure";
const SHORTCUT_DESCRIPTION: &str = "Dicter avec Murmure (maintenir ou appuyer)";

const XK_CONTROL_L: i32 = 0xffe3;
const XK_V: i32 = 0x0076;
/// Délai entre deux événements clavier simulés.
const KEY_GAP: Duration = Duration::from_millis(12);
/// Délai maximal pour un appel « rapide » (hors boîtes de dialogue).
const CALL_TIMEOUT: Duration = Duration::from_secs(3);

struct ShortcutsSession {
    proxy: GlobalShortcuts,
    session: Session<GlobalShortcuts>,
}

struct KeyboardSession {
    proxy: RemoteDesktop,
    session: Session<RemoteDesktop>,
}

/// État des sessions de portail (géré par Tauri, Linux).
#[derive(Default)]
pub struct Portals {
    connection: OnceCell<zbus::Connection>,
    shortcuts: Mutex<Option<ShortcutsSession>>,
    keyboard: Mutex<Option<KeyboardSession>>,
}

fn portals(app: &AppHandle) -> tauri::State<'_, Portals> {
    app.state::<Portals>()
}

/// Message lisible pour une erreur de portail.
fn describe(e: &ashpd::Error) -> String {
    match e {
        ashpd::Error::Response(ResponseError::Cancelled) => "Demande refusée.".into(),
        ashpd::Error::PortalNotFound(iface) => format!("Portail absent ({iface})."),
        ashpd::Error::RequiresVersion(want, have) => {
            format!("Version du portail trop ancienne ({have}, {want} requise).")
        }
        other => other.to_string(),
    }
}

fn is_cancelled(e: &ashpd::Error) -> bool {
    matches!(e, ashpd::Error::Response(ResponseError::Cancelled))
}

/// Connexion D-Bus partagée, avec l'app id enregistré auprès du portail (une seule fois).
async fn connection(app: &AppHandle) -> Result<zbus::Connection, String> {
    let p = portals(app);
    p.connection
        .get_or_try_init(|| async {
            let conn = zbus::Connection::session()
                .await
                .map_err(|e| format!("Bus de session D-Bus injoignable : {e}"))?;
            register_app_id(&conn).await;
            Ok::<_, String>(conn)
        })
        .await
        .cloned()
}

/// `org.freedesktop.host.portal.Registry.Register` : best effort (absent avant xdg-desktop-portal 1.19,
/// refusé si le bureau a déjà détecté l'app id depuis l'unité systemd — dans les deux cas on continue).
async fn register_app_id(conn: &zbus::Connection) {
    let created = match ensure_desktop_file() {
        Ok(created) => created,
        Err(e) => {
            log::warn!("fichier {APP_ID}.desktop : {e}");
            false
        }
    };
    // Un .desktop tout juste créé n'est pas encore indexé par le portail (« App info not found ») :
    // quelques nouvelles tentatives espacées.
    let attempts = if created { 8 } else { 1 };
    for attempt in 1..=attempts {
        let options: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        let res = tokio::time::timeout(
            CALL_TIMEOUT,
            conn.call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.host.portal.Registry"),
                "Register",
                &(APP_ID, options),
            ),
        )
        .await;
        match res {
            Ok(Ok(_)) => {
                log::info!("portail : app id « {APP_ID} » enregistré");
                return;
            }
            Ok(Err(e)) if attempt < attempts && e.to_string().contains("not found") => {
                log::debug!("portail : {APP_ID}.desktop pas encore indexé ({e}), nouvel essai");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Ok(Err(e)) => {
                log::info!("portail : enregistrement de l'app id impossible ({e}) — app id détecté par le bureau");
                return;
            }
            Err(_) => {
                log::warn!("portail : délai dépassé pour l'enregistrement de l'app id");
                return;
            }
        }
    }
}

/// Garantit qu'un `Murmure.desktop` existe dans les dossiers XDG (exigé par le registre du portail).
/// Renvoie `true` s'il vient d'être créé.
fn ensure_desktop_file() -> Result<bool, String> {
    let name = format!("{APP_ID}.desktop");
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
        .ok_or("dossier personnel introuvable")?;
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let exists = std::iter::once(data_home.clone())
        .chain(data_dirs.split(':').map(std::path::PathBuf::from))
        .any(|d| d.join("applications").join(&name).is_file());
    if exists {
        return Ok(false);
    }
    let exec = std::env::var_os("APPIMAGE")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .ok_or("exécutable introuvable")?;
    let dir = data_home.join("applications");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let content = format!(
        "[Desktop Entry]\nType=Application\nName=Murmure\nComment=Dictée vocale\nExec=\"{}\" %U\nIcon=murmure\nTerminal=false\nCategories=Utility;\n",
        exec.display()
    );
    std::fs::write(dir.join(&name), content).map_err(|e| e.to_string())?;
    log::info!("{name} créé dans {}", dir.display());
    Ok(true)
}

// ───────────────────────────────── Démarrage ─────────────────────────────────

/// Au lancement sous Wayland : rebranche le raccourci (sans boîte de dialogue si déjà accordé) et
/// restaure la session clavier si un jeton existe. Avant la fin de l'onboarding, on attend le bouton
/// « Autoriser Murmure » pour ne pas ouvrir de boîte de dialogue par surprise.
pub fn init(app: &AppHandle, settings: &Settings) {
    wayland::update(app, |s| {
        s.wayland = true;
        s.paste_fallback = crate::inject::wayland_fallback_tool().map(str::to_string);
    });
    let restore_keyboard = settings.wayland_restore_token.is_some();
    if !settings.onboarding_done && !restore_keyboard {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = bind_shortcut(&app).await {
            log::warn!("raccourci Wayland : {e}");
        }
        if restore_keyboard && let Err(e) = start_keyboard(&app).await {
            log::warn!("contrôle clavier Wayland : {e}");
        }
    });
}

/// Bouton « Autoriser Murmure » : raccourci puis contrôle clavier (deux boîtes de dialogue au plus).
pub async fn authorize(app: &AppHandle) -> wayland::WaylandStatus {
    if let Err(e) = bind_shortcut(app).await {
        log::warn!("raccourci Wayland : {e}");
    }
    if let Err(e) = start_keyboard(app).await {
        log::warn!("contrôle clavier Wayland : {e}");
    }
    wayland::status(app)
}

// ───────────────────────────── Raccourci global ──────────────────────────────

fn trigger_of(shortcuts: &[Shortcut]) -> Option<String> {
    shortcuts
        .iter()
        .find(|s| s.id() == SHORTCUT_ID)
        .map(|s| s.trigger_description().trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Crée la session GlobalShortcuts et déclare le raccourci (idempotent).
pub async fn bind_shortcut(app: &AppHandle) -> Result<(), String> {
    let p = portals(app);
    let mut guard = p.shortcuts.lock().await;
    if guard.is_some() {
        // Déjà actif : relit simplement le raccourci attribué.
        drop(guard);
        refresh_trigger(app).await;
        return Ok(());
    }
    wayland::update(app, |s| {
        s.shortcut = PortalState::Pending;
        s.shortcut_error = None;
    });
    let fail = |state: PortalState, msg: String| {
        wayland::update(app, |s| {
            s.shortcut = state;
            s.shortcut_error = Some(msg.clone());
        });
        msg
    };

    let conn = connection(app).await.map_err(|e| fail(PortalState::Unavailable, e))?;
    let proxy = match GlobalShortcuts::with_connection(conn).await {
        Ok(p) => p,
        Err(e) => {
            return Err(fail(
                PortalState::Unavailable,
                format!("Le bureau ne propose pas le portail des raccourcis globaux : {}", describe(&e)),
            ));
        }
    };
    let configurable = proxy.version() >= 2;

    // Abonnement aux signaux AVANT la déclaration, pour ne rien manquer (filtrés par session ensuite).
    let activated = proxy.receive_activated().await.map_err(|e| fail(PortalState::Unavailable, describe(&e)))?;
    let deactivated = proxy.receive_deactivated().await.map_err(|e| fail(PortalState::Unavailable, describe(&e)))?;
    let changed = proxy.receive_shortcuts_changed().await.map_err(|e| fail(PortalState::Unavailable, describe(&e)))?;

    let accel = app.state::<AppState>().settings().hotkey;
    let preferred = wayland::accelerator_to_portal(&accel);
    let shortcut = NewShortcut::new(SHORTCUT_ID, SHORTCUT_DESCRIPTION).preferred_trigger(preferred.as_deref());
    log::info!("portail : déclaration du raccourci (préféré : {preferred:?})");
    let denied = |e: &ashpd::Error| {
        let state = if is_cancelled(e) { PortalState::Denied } else { PortalState::Unavailable };
        fail(state, format!("Raccourci non accordé : {}", describe(e)))
    };
    let (mut session, mut trigger, known) = open_bound_session(&proxy, &shortcut).await.map_err(|e| denied(&e))?;
    if !known && is_kde() {
        // KDE (xdg-desktop-portal-kde) lie la session au composant kglobalaccel à sa création : une
        // session ouverte avant la toute première déclaration ne reçoit jamais Activated. On en rouvre
        // une ; le raccourci est désormais connu, donc pas de nouvelle boîte de dialogue.
        log::info!("portail : premier raccourci déclaré, réouverture de la session");
        let _ = session.close().await;
        let (s, t, _) = open_bound_session(&proxy, &shortcut).await.map_err(|e| denied(&e))?;
        session = s;
        trigger = t.or(trigger);
    }
    log::info!("portail : raccourci actif ({})", trigger.as_deref().unwrap_or("aucune touche attribuée"));
    hotkey::reset_machine(app);

    let handle = session_path(&session);
    spawn_listeners(app, handle, activated, deactivated, changed);
    *guard = Some(ShortcutsSession { proxy, session });
    drop(guard);
    wayland::update(app, |s| {
        s.shortcut = PortalState::Active;
        s.shortcut_trigger = trigger;
        s.shortcut_configurable = configurable;
        s.shortcut_error = None;
    });
    // Certains bureaux ne renvoient pas la description dans BindShortcuts : on relit.
    refresh_trigger(app).await;
    Ok(())
}

fn is_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("KDE")))
}

/// Ouvre une session et y déclare le raccourci (peut ouvrir la boîte de dialogue du bureau : pas de
/// délai). Renvoie la session, le raccourci attribué et si `dictation` était déjà connu du bureau.
async fn open_bound_session(
    proxy: &GlobalShortcuts,
    shortcut: &NewShortcut,
) -> Result<(Session<GlobalShortcuts>, Option<String>, bool), ashpd::Error> {
    let session = proxy.create_session(Default::default()).await?;
    let known = match async { proxy.list_shortcuts(&session, Default::default()).await?.response() }.await {
        Ok(list) => list.shortcuts().iter().any(|s| s.id() == SHORTCUT_ID),
        // Inconnu : on ne rouvrira pas de session.
        Err(_) => true,
    };
    let bound = async {
        proxy
            .bind_shortcuts(&session, std::slice::from_ref(shortcut), None, Default::default())
            .await?
            .response()
    }
    .await;
    match bound {
        Ok(b) => Ok((session, trigger_of(b.shortcuts()), known)),
        Err(e) => {
            let _ = session.close().await;
            Err(e)
        }
    }
}

fn spawn_listeners(
    app: &AppHandle,
    handle: String,
    activated: impl futures_util::Stream<Item = ashpd::desktop::global_shortcuts::Activated> + Send + Unpin + 'static,
    deactivated: impl futures_util::Stream<Item = ashpd::desktop::global_shortcuts::Deactivated> + Send + Unpin + 'static,
    changed: impl futures_util::Stream<Item = ashpd::desktop::global_shortcuts::ShortcutsChanged> + Send + Unpin + 'static,
) {
    let (a, h) = (app.clone(), handle.clone());
    tauri::async_runtime::spawn(async move {
        let mut s = activated;
        while let Some(ev) = s.next().await {
            if ev.session_handle().as_str() == h && ev.shortcut_id() == SHORTCUT_ID {
                hotkey::on_key_event(&a, KeyEvent::Pressed);
            }
        }
        log::warn!("portail : flux Activated terminé");
    });
    let (a, h) = (app.clone(), handle.clone());
    tauri::async_runtime::spawn(async move {
        let mut s = deactivated;
        while let Some(ev) = s.next().await {
            if ev.session_handle().as_str() == h && ev.shortcut_id() == SHORTCUT_ID {
                hotkey::on_key_event(&a, KeyEvent::Released);
            }
        }
    });
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut s = changed;
        while let Some(ev) = s.next().await {
            if ev.session_handle().as_str() == handle {
                let trigger = trigger_of(ev.shortcuts());
                log::info!("portail : raccourci modifié ({})", trigger.as_deref().unwrap_or("aucune touche"));
                wayland::update(&a, |st| st.shortcut_trigger = trigger);
            }
        }
    });
}

/// `ListShortcuts` : relit le raccourci attribué (l'utilisateur a pu le changer dans KDE).
pub async fn refresh_trigger(app: &AppHandle) {
    let p = portals(app);
    let guard = p.shortcuts.lock().await;
    let Some(s) = guard.as_ref() else { return };
    let res = tokio::time::timeout(CALL_TIMEOUT, async {
        s.proxy.list_shortcuts(&s.session, Default::default()).await?.response()
    })
    .await;
    match res {
        Ok(Ok(list)) => {
            let trigger = trigger_of(list.shortcuts());
            wayland::update(app, |st| st.shortcut_trigger = trigger);
        }
        Ok(Err(e)) => log::debug!("ListShortcuts : {}", describe(&e)),
        Err(_) => log::debug!("ListShortcuts : délai dépassé"),
    }
}

/// `ConfigureShortcuts` (portail v2) : ouvre le réglage du raccourci du bureau.
pub async fn configure_shortcut(app: &AppHandle) -> Result<(), String> {
    let p = portals(app);
    let guard = p.shortcuts.lock().await;
    let Some(s) = guard.as_ref() else {
        return Err("Autorisez d'abord le raccourci de Murmure.".into());
    };
    s.proxy
        .configure_shortcuts(&s.session, None, Default::default())
        .await
        .map_err(|e| {
            format!(
                "Le bureau ne permet pas d'ouvrir ce réglage ({}). KDE : Configuration du système › Clavier › Raccourcis › Murmure.",
                describe(&e)
            )
        })
}

// ───────────────────────────── Contrôle du clavier ───────────────────────────

/// Démarre (ou restaure sans boîte de dialogue grâce au jeton) la session RemoteDesktop clavier.
pub async fn start_keyboard(app: &AppHandle) -> Result<(), String> {
    let p = portals(app);
    let mut guard = p.keyboard.lock().await;
    if guard.is_some() {
        return Ok(());
    }
    wayland::update(app, |s| {
        s.keyboard = PortalState::Pending;
        s.keyboard_error = None;
    });
    let fail = |state: PortalState, msg: String| {
        wayland::update(app, |s| {
            s.keyboard = state;
            s.keyboard_error = Some(msg.clone());
        });
        msg
    };

    let conn = connection(app).await.map_err(|e| fail(PortalState::Unavailable, e))?;
    let proxy = RemoteDesktop::with_connection(conn.clone()).await.map_err(|e| {
        fail(PortalState::Unavailable, format!("Le bureau ne propose pas le portail de contrôle à distance : {}", describe(&e)))
    })?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| fail(PortalState::Unavailable, format!("Session clavier impossible : {}", describe(&e))))?;

    let token = app.state::<AppState>().settings().wayland_restore_token;
    let options = SelectDevicesOptions::default()
        .set_devices(BitFlags::from_flag(DeviceType::Keyboard))
        .set_persist_mode(PersistMode::ExplicitlyRevoked)
        .set_restore_token(token.as_deref());
    let started = async {
        proxy.select_devices(&session, options).await?.response()?;
        // Boîte de dialogue du bureau (sauf restauration) : pas de délai.
        proxy.start(&session, None, Default::default()).await?.response()
    }
    .await;
    let selected = match started {
        Ok(s) => s,
        Err(e) => {
            let _ = session.close().await;
            let state = if is_cancelled(&e) { PortalState::Denied } else { PortalState::Unavailable };
            if is_cancelled(&e) {
                // Jeton refusé ou révoqué : on n'essaiera plus de restaurer au démarrage.
                save_token(app, None).await;
            }
            return Err(fail(state, format!("Contrôle du clavier non accordé : {}", describe(&e))));
        }
    };
    if !selected.devices().contains(DeviceType::Keyboard) {
        let _ = session.close().await;
        return Err(fail(PortalState::Denied, "Le bureau n'a pas accordé le clavier.".into()));
    }
    let new_token = selected.restore_token().map(str::to_string);
    log::info!(
        "portail : contrôle clavier actif (jeton de restauration {})",
        if new_token.is_some() { "reçu" } else { "absent : l'autorisation sera redemandée au prochain lancement" }
    );
    // Le jeton est à usage unique : on enregistre toujours le nouveau.
    save_token(app, new_token).await;

    // Session fermée par le bureau (l'utilisateur a arrêté le contrôle à distance) → repli.
    watch_closed(app, &conn, session_path(&session)).await;
    *guard = Some(KeyboardSession { proxy, session });
    drop(guard);
    wayland::update(app, |s| {
        s.keyboard = PortalState::Active;
        s.keyboard_error = None;
    });
    Ok(())
}

/// Chemin D-Bus d'une session (ashpd ne l'expose que via `Serialize`).
fn session_path<T: SessionPortal>(session: &Session<T>) -> String {
    serde_json::to_value(session)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Surveille le signal `org.freedesktop.portal.Session.Closed` de la session clavier.
async fn watch_closed(app: &AppHandle, conn: &zbus::Connection, path: String) {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.Session")
        .and_then(|b| b.member("Closed"))
        .and_then(|b| b.path(path))
        .map(|b| b.build());
    let rule = match rule {
        Ok(r) => r,
        Err(e) => {
            log::debug!("règle Closed : {e}");
            return;
        }
    };
    let mut stream = match zbus::MessageStream::for_match_rule(rule, conn, None).await {
        Ok(s) => s,
        Err(e) => {
            log::debug!("abonnement Closed : {e}");
            return;
        }
    };
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        if stream.next().await.is_some() {
            log::info!("portail : session clavier fermée par le bureau");
            *portals(&a).keyboard.lock().await = None;
            wayland::update(&a, |s| s.keyboard = PortalState::Inactive);
        }
    });
}

async fn save_token(app: &AppHandle, token: Option<String>) {
    let state = app.state::<AppState>();
    let to_save = {
        let mut s = state.settings.lock();
        if s.wayland_restore_token == token {
            return;
        }
        s.wayland_restore_token = token;
        s.clone()
    };
    let dir = state.config_dir.clone();
    let res = tauri::async_runtime::spawn_blocking(move || crate::settings::save(&dir, &to_save)).await;
    if !matches!(res, Ok(Ok(()))) {
        log::warn!("enregistrement du jeton de restauration impossible");
    }
}

/// La session clavier est-elle prête (sans attendre) ?
pub fn keyboard_ready(app: &AppHandle) -> bool {
    wayland::status(app).keyboard == PortalState::Active
}

/// Envoie Ctrl+V par le portail. Bloquant : à appeler hors du runtime async (`spawn_blocking`).
pub fn paste_blocking(app: &AppHandle) -> Result<(), String> {
    let app = app.clone();
    tauri::async_runtime::block_on(async move {
        tokio::time::timeout(CALL_TIMEOUT, paste(&app))
            .await
            .map_err(|_| "Délai dépassé pendant le collage via le portail".to_string())?
    })
}

async fn paste(app: &AppHandle) -> Result<(), String> {
    let p = portals(app);
    let guard = p.keyboard.lock().await;
    let Some(k) = guard.as_ref() else {
        return Err("Contrôle du clavier non autorisé".into());
    };
    let key = |sym: i32, state: KeyState| async move {
        k.proxy.notify_keyboard_keysym(&k.session, sym, state, Default::default()).await
    };
    let err = |e: ashpd::Error| format!("Collage via le portail impossible : {}", describe(&e));
    key(XK_CONTROL_L, KeyState::Pressed).await.map_err(err)?;
    tokio::time::sleep(KEY_GAP).await;
    let click = async {
        key(XK_V, KeyState::Pressed).await?;
        tokio::time::sleep(KEY_GAP).await;
        key(XK_V, KeyState::Released).await
    }
    .await;
    tokio::time::sleep(KEY_GAP).await;
    // Toujours relâcher Ctrl, même si « v » a échoué.
    let release = key(XK_CONTROL_L, KeyState::Released).await;
    click.map_err(err)?;
    release.map_err(err)
}

// ───────────────────────────────── Notification ──────────────────────────────

/// Notification du bureau (portail), sans bloquer ; ignorée en cas d'échec.
pub fn notify(app: &AppHandle, title: &str, body: &str) {
    use ashpd::desktop::notification::{Notification, NotificationProxy, Priority};
    let (app, title, body) = (app.clone(), title.to_string(), body.to_string());
    tauri::async_runtime::spawn(async move {
        let res = async {
            let conn = connection(&app).await.map_err(|e| e.to_string())?;
            let proxy = NotificationProxy::with_connection(conn).await.map_err(|e| e.to_string())?;
            let n = Notification::new(&title).body(body.as_str()).priority(Priority::Normal);
            proxy.add_notification("murmure-copied", n).await.map_err(|e| e.to_string())
        }
        .await;
        if let Err(e) = res {
            log::debug!("notification : {e}");
        }
    });
}

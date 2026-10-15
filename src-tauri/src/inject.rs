// SPDX-License-Identifier: AGPL-3.0-or-later
//! Insertion du texte dans l'application active.
//!
//! Mode `paste` : sauvegarde du presse-papiers (texte seulement), écriture du texte,
//! simulation de Cmd+V / Ctrl+V, puis restauration après ~250 ms.
//! Mode `clipboard` : copie seule.
//!
//! macOS : enigo interroge le clavier courant via Text Input Sources (TIS) pour
//! `Key::Unicode`, API qui exige le thread principal sur les macOS récents :
//! la frappe est donc exécutée via `run_on_main_thread`.
//! Linux Wayland : enigo (x11rb) n'atteint pas les fenêtres natives. Ordre ([`paste_plan`]) :
//! portail RemoteDesktop (`portal.rs`) → `ydotool` (si `ydotoold` répond) → `wtype` (refusé
//! par KWin, accepté par les compositeurs wlroots) → à défaut « copié seulement » + notification.
//! Presse-papiers Wayland : arboard avec `wayland-data-control` (wlr / ext-data-control, KWin
//! et wlroots), repli X11 via XWayland sinon (GNOME) ; le contenu est servi par un thread de
//! fond tant que Murmure tourne.
//!
//! Ces fonctions sont bloquantes : à appeler depuis `spawn_blocking`.

use crate::settings::PasteMode;
use arboard::Clipboard;
use parking_lot::Mutex;
use std::time::Duration;
use tauri::AppHandle;

/// Délai avant restauration du presse-papiers (laisse l'app cible lire le contenu).
const RESTORE_DELAY: Duration = Duration::from_millis(250);
/// Petit délai entre l'écriture du presse-papiers et la frappe (gestionnaires de presse-papiers).
const SETTLE_DELAY: Duration = Duration::from_millis(40);

/// Presse-papiers gardé vivant : sous Linux (X11), le contenu appartient au
/// processus tant que l'objet `Clipboard` existe.
static CLIPBOARD: Mutex<Option<Clipboard>> = Mutex::new(None);

/// Résultat de l'insertion.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Collé dans l'application active.
    Pasted,
    /// Copié seulement (mode `clipboard`).
    Copied,
    /// Collage impossible : le texte est dans le presse-papiers. Message à afficher.
    CopiedOnly(String),
}

fn with_clipboard<T>(f: impl FnOnce(&mut Clipboard) -> Result<T, arboard::Error>) -> Result<T, String> {
    let mut guard = CLIPBOARD.lock();
    if guard.is_none() {
        *guard = Some(Clipboard::new().map_err(|e| format!("Presse-papiers inaccessible : {e}"))?);
    }
    let Some(cb) = guard.as_mut() else {
        return Err("Presse-papiers inaccessible".into());
    };
    f(cb).map_err(|e| format!("Presse-papiers : {e}"))
}

/// Copie du texte dans le presse-papiers.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    with_clipboard(|cb| cb.set_text(text.to_owned()))
}

fn read_clipboard_text() -> Option<String> {
    with_clipboard(|cb| cb.get_text()).ok()
}

/// Insère `text` selon le mode demandé.
pub fn deliver(app: &AppHandle, text: &str, mode: PasteMode, restore: bool) -> Result<Outcome, String> {
    match mode {
        PasteMode::Clipboard => {
            copy_to_clipboard(text)?;
            Ok(Outcome::Copied)
        }
        PasteMode::Paste => paste(app, text, restore),
    }
}

fn paste(app: &AppHandle, text: &str, restore: bool) -> Result<Outcome, String> {
    let previous = if restore { read_clipboard_text() } else { None };
    copy_to_clipboard(text)?;
    std::thread::sleep(SETTLE_DELAY);

    if let Err(msg) = send_paste_keystroke(app) {
        log::warn!("collage automatique impossible : {msg}");
        // On laisse le texte dans le presse-papiers (pas de restauration).
        return Ok(Outcome::CopiedOnly(msg));
    }

    if let Some(prev) = previous {
        std::thread::sleep(RESTORE_DELAY);
        // Ne restaure que si personne n'a modifié le presse-papiers entre-temps.
        if read_clipboard_text().as_deref() == Some(text)
            && let Err(e) = copy_to_clipboard(&prev)
        {
            log::warn!("restauration du presse-papiers : {e}");
        }
    }
    Ok(Outcome::Pasted)
}

/// Session Wayland ?
pub fn is_wayland() -> bool {
    crate::wayland::is_wayland_session()
}

/// Méthode de simulation de Ctrl+V.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteMethod {
    /// enigo (X11, macOS, Windows).
    Enigo,
    /// Portail `org.freedesktop.portal.RemoteDesktop` (Wayland).
    Portal,
    /// `ydotool` + démon `ydotoold` (uinput).
    Ydotool,
    /// `wtype` (protocole virtual-keyboard, compositeurs wlroots).
    Wtype,
}

/// Ce qui est disponible sur la machine au moment du collage.
#[derive(Clone, Copy, Debug, Default)]
pub struct PasteEnv {
    pub wayland: bool,
    pub portal_ready: bool,
    pub ydotoold: bool,
    pub wtype: bool,
}

/// Méthodes à essayer, dans l'ordre. Vide → copie seule.
pub fn paste_plan(env: PasteEnv) -> Vec<PasteMethod> {
    if !env.wayland {
        return vec![PasteMethod::Enigo];
    }
    let mut plan = Vec::with_capacity(3);
    if env.portal_ready {
        plan.push(PasteMethod::Portal);
    }
    if env.ydotoold {
        plan.push(PasteMethod::Ydotool);
    }
    if env.wtype {
        plan.push(PasteMethod::Wtype);
    }
    plan
}

/// Message quand le texte n'a pu qu'être copié sous Wayland.
pub const WAYLAND_COPIED_MSG: &str = "Texte copié — collez avec Ctrl+V. Pour coller automatiquement, \
     autorisez Murmure dans Réglages › Raccourci.";

fn send_paste_keystroke(app: &AppHandle) -> Result<(), String> {
    if is_wayland() {
        return paste_wayland(app);
    }
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(enigo_paste());
        })
        .map_err(|e| format!("Thread principal indisponible : {e}"))?;
        rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| "Délai dépassé pendant la simulation du collage".to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        enigo_paste()
    }
}

fn enigo_paste() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| match e {
        enigo::NewConError::NoPermission => {
            "Autorisez Murmure dans Réglages Système › Confidentialité et sécurité › Accessibilité \
             pour coller automatiquement — le texte est dans le presse-papiers."
                .to_string()
        }
        other => format!("Simulation clavier indisponible ({other}) — le texte est dans le presse-papiers."),
    })?;

    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    let err = |e: enigo::InputError| format!("Simulation du collage impossible : {e}");
    enigo.key(modifier, Direction::Press).map_err(err)?;
    let click = enigo.key(Key::Unicode('v'), Direction::Click).map_err(err);
    // Toujours relâcher le modificateur, même si le clic a échoué.
    let release = enigo.key(modifier, Direction::Release).map_err(err);
    click.and(release)
}

/// Exécutable présent dans le PATH ?
fn in_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let p = dir.join(bin);
            p.is_file()
        })
    })
}

/// Socket de `ydotoold` joignable ? (`YDOTOOL_SOCKET`, sinon `$XDG_RUNTIME_DIR/.ydotool_socket`,
/// sinon `/tmp/.ydotool_socket`). Sans démon, `ydotool` échoue ou ne fait rien.
#[cfg(unix)]
fn ydotoold_reachable() -> bool {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("YDOTOOL_SOCKET") {
        candidates.push(p.into());
    } else {
        if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR") {
            candidates.push(std::path::Path::new(&d).join(".ydotool_socket"));
        }
        candidates.push("/tmp/.ydotool_socket".into());
    }
    candidates.iter().any(|path| {
        std::os::unix::net::UnixDatagram::unbound().is_ok_and(|sock| sock.connect(path).is_ok())
    }) && in_path("ydotool")
}

#[cfg(not(unix))]
fn ydotoold_reachable() -> bool {
    false
}

/// Outil de secours détecté (affiché dans les réglages).
pub fn wayland_fallback_tool() -> Option<&'static str> {
    if ydotoold_reachable() {
        Some("ydotool")
    } else if in_path("wtype") {
        Some("wtype")
    } else {
        None
    }
}

fn run_tool(bin: &str, args: &[&str]) -> Result<(), String> {
    match std::process::Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{bin} a échoué ({status})")),
        Err(e) => Err(format!("{bin} indisponible : {e}")),
    }
}

/// Collage sous Wayland selon [`paste_plan`].
fn paste_wayland(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let portal_ready = crate::portal::keyboard_ready(app);
    #[cfg(not(target_os = "linux"))]
    let portal_ready = {
        let _ = app;
        false
    };
    let env = PasteEnv { wayland: true, portal_ready, ydotoold: ydotoold_reachable(), wtype: in_path("wtype") };
    for method in paste_plan(env) {
        let res = match method {
            #[cfg(target_os = "linux")]
            PasteMethod::Portal => crate::portal::paste_blocking(app),
            // KEY_LEFTCTRL = 29, KEY_V = 47 (codes evdev).
            PasteMethod::Ydotool => run_tool("ydotool", &["key", "29:1", "47:1", "47:0", "29:0"]),
            PasteMethod::Wtype => run_tool("wtype", &["-M", "ctrl", "v", "-m", "ctrl"]),
            _ => continue,
        };
        match res {
            Ok(()) => {
                log::debug!("collage Wayland via {method:?}");
                return Ok(());
            }
            Err(e) => log::debug!("collage Wayland via {method:?} : {e}"),
        }
    }
    #[cfg(target_os = "linux")]
    crate::portal::notify(app, "Texte copié", "Collez-le avec Ctrl+V.");
    Err(WAYLAND_COPIED_MSG.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use PasteMethod::*;

    fn env(wayland: bool, portal_ready: bool, ydotoold: bool, wtype: bool) -> PasteEnv {
        PasteEnv { wayland, portal_ready, ydotoold, wtype }
    }

    #[test]
    fn x11_et_autres_plateformes_utilisent_enigo() {
        assert_eq!(paste_plan(env(false, false, false, false)), [Enigo]);
        // Hors Wayland, les outils Wayland sont ignorés même s'ils sont installés.
        assert_eq!(paste_plan(env(false, true, true, true)), [Enigo]);
    }

    #[test]
    fn wayland_ordre_de_repli() {
        assert_eq!(paste_plan(env(true, true, true, true)), [Portal, Ydotool, Wtype]);
        assert_eq!(paste_plan(env(true, true, false, false)), [Portal]);
        assert_eq!(paste_plan(env(true, false, true, true)), [Ydotool, Wtype]);
        assert_eq!(paste_plan(env(true, false, false, true)), [Wtype]);
        assert_eq!(paste_plan(env(true, true, false, true)), [Portal, Wtype]);
    }

    #[test]
    fn wayland_sans_outil_copie_seulement() {
        assert!(paste_plan(env(true, false, false, false)).is_empty());
        assert!(!paste_plan(env(true, false, false, false)).contains(&Enigo));
    }
}

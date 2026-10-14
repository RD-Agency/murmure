//! Insertion du texte dans l'application active.
//!
//! Mode `paste` : sauvegarde du presse-papiers (texte seulement), écriture du texte,
//! simulation de Cmd+V / Ctrl+V, puis restauration après ~250 ms.
//! Mode `clipboard` : copie seule.
//!
//! macOS : enigo interroge le clavier courant via Text Input Sources (TIS) pour
//! `Key::Unicode`, API qui exige le thread principal sur les macOS récents :
//! la frappe est donc exécutée via `run_on_main_thread`.
//! Linux Wayland : enigo (x11rb) ne fonctionne pas → `wtype`, `ydotool`, puis
//! `xdotool` (XWayland) ; à défaut le texte reste dans le presse-papiers.
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
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

fn send_paste_keystroke(app: &AppHandle) -> Result<(), String> {
    if is_wayland() {
        return paste_wayland();
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

/// Essaie successivement les outils de frappe disponibles sous Wayland.
fn paste_wayland() -> Result<(), String> {
    const TOOLS: &[(&str, &[&str])] = &[
        ("wtype", &["-M", "ctrl", "v", "-m", "ctrl"]),
        ("ydotool", &["key", "29:1", "47:1", "47:0", "29:0"]),
        ("xdotool", &["key", "--clearmodifiers", "ctrl+v"]),
    ];
    for (bin, args) in TOOLS {
        match std::process::Command::new(bin)
            .args(*args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            Ok(status) if status.success() => {
                log::debug!("collage Wayland via {bin}");
                return Ok(());
            }
            Ok(status) => log::debug!("{bin} a échoué ({status})"),
            Err(e) => log::debug!("{bin} indisponible : {e}"),
        }
    }
    Err("Texte copié dans le presse-papiers — collez-le avec Ctrl+V \
         (installez wtype ou ydotool pour le collage automatique sous Wayland)."
        .to_string())
}

//! Session Wayland : détection, état des portails XDG exposé au front, conversions pures.
//!
//! Sous Wayland, les raccourcis globaux X11 (`tauri-plugin-global-shortcut`) ne sont pas reçus
//! et la simulation clavier X11 (enigo/xdotool) n'atteint pas les fenêtres natives. Murmure passe
//! alors par les portails `org.freedesktop.portal.GlobalShortcuts` (raccourci) et
//! `org.freedesktop.portal.RemoteDesktop` (Ctrl+V) — voir `portal.rs` (Linux uniquement).
//! Ce module reste compilé partout : les fonctions pures sont testées sur toutes les plateformes.

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Identifiant du raccourci déclaré au portail.
pub const SHORTCUT_ID: &str = "dictation";

/// Session graphique Wayland ?
///
/// `XDG_SESSION_TYPE=wayland` ou `WAYLAND_DISPLAY` défini → Wayland ; `XDG_SESSION_TYPE=x11`
/// explicite l'emporte sur un `WAYLAND_DISPLAY` hérité (session X11 lancée depuis Wayland).
pub fn detect_wayland(xdg_session_type: Option<&str>, wayland_display: Option<&str>) -> bool {
    match xdg_session_type.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("wayland") => true,
        Some("x11") => false,
        _ => wayland_display.is_some_and(|d| !d.trim().is_empty()),
    }
}

/// Session Wayland en cours (toujours `false` hors Linux).
pub fn is_wayland_session() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    let st = std::env::var("XDG_SESSION_TYPE").ok();
    let wd = std::env::var("WAYLAND_DISPLAY").ok();
    detect_wayland(st.as_deref(), wd.as_deref())
}

/// Convertit un accélérateur Tauri (`Ctrl+Alt+Space`) au format « shortcuts » de la
/// spécification XDG (`CTRL+ALT+space`) : modificateurs `CTRL`, `ALT`, `SHIFT`, `LOGO`,
/// touche = nom de keysym xkb. `None` si une partie n'a pas d'équivalent.
pub fn accelerator_to_portal(accelerator: &str) -> Option<String> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut logo = false;
    let mut key: Option<String> = None;
    for raw in accelerator.split('+') {
        let part = raw.trim();
        if part.is_empty() {
            return None;
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "commandorcontrol" | "commandorctrl" | "cmdorctrl" | "cmdorcontrol" => ctrl = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            "super" | "meta" | "command" | "cmd" => logo = true,
            _ => {
                if key.is_some() {
                    return None; // une seule touche principale
                }
                key = Some(portal_key(part)?);
            }
        }
    }
    let key = key?;
    let mut out: Vec<&str> = Vec::new();
    if ctrl {
        out.push("CTRL");
    }
    if alt {
        out.push("ALT");
    }
    if shift {
        out.push("SHIFT");
    }
    if logo {
        out.push("LOGO");
    }
    let mut s = out.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(&key);
    Some(s)
}

/// Nom de keysym xkb pour une touche d'accélérateur Tauri.
fn portal_key(part: &str) -> Option<String> {
    // Lettres : « A » ou « KeyA » → « a ».
    let letter = part.strip_prefix("Key").filter(|r| r.len() == 1).unwrap_or(part);
    if letter.len() == 1 {
        let c = letter.chars().next()?;
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_lowercase().to_string());
        }
    }
    if let Some(d) = part.strip_prefix("Digit").filter(|d| d.len() == 1 && d.chars().all(|c| c.is_ascii_digit())) {
        return Some(d.to_string());
    }
    if let Some(d) = part.strip_prefix("Numpad").filter(|d| d.len() == 1 && d.chars().all(|c| c.is_ascii_digit())) {
        return Some(format!("KP_{d}"));
    }
    if let Some(n) = part.strip_prefix('F').or_else(|| part.strip_prefix('f'))
        && let Ok(n) = n.parse::<u8>()
        && (1..=24).contains(&n)
    {
        return Some(format!("F{n}"));
    }
    let k = match part.to_ascii_lowercase().as_str() {
        "space" => "space",
        "enter" | "return" => "Return",
        "tab" => "Tab",
        "backspace" => "BackSpace",
        "delete" | "del" => "Delete",
        "insert" => "Insert",
        "escape" | "esc" => "Escape",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "backquote" | "`" => "grave",
        "minus" | "-" => "minus",
        "equal" | "=" => "equal",
        "bracketleft" | "[" => "bracketleft",
        "bracketright" | "]" => "bracketright",
        "backslash" | "\\" => "backslash",
        "semicolon" | ";" => "semicolon",
        "quote" | "'" => "apostrophe",
        "comma" | "," => "comma",
        "period" | "." => "period",
        "slash" | "/" => "slash",
        _ => return None,
    };
    Some(k.to_string())
}

// ─────────────────────────────── État exposé au front ───────────────────────────────

/// État d'une autorisation de portail.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortalState {
    /// Sans objet (X11, macOS, Windows) ou pas encore demandé.
    #[default]
    Inactive,
    /// Demande en cours (boîte de dialogue du bureau ouverte).
    Pending,
    /// Autorisé et opérationnel.
    Active,
    /// Refusé par l'utilisateur.
    Denied,
    /// Portail absent ou en erreur.
    Unavailable,
}

/// État Wayland (commande `get_wayland_status`, événement `wayland-status`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq, Default)]
pub struct WaylandStatus {
    /// Session Wayland détectée (sinon tout le reste est sans objet).
    pub wayland: bool,
    /// Raccourci global via `org.freedesktop.portal.GlobalShortcuts`.
    pub shortcut: PortalState,
    /// Raccourci réellement attribué par le bureau (ex. « Ctrl+Alt+Espace »), modifiable dans KDE.
    pub shortcut_trigger: Option<String>,
    /// Le portail permet d'ouvrir sa boîte de réglage du raccourci (`ConfigureShortcuts`, v2).
    pub shortcut_configurable: bool,
    pub shortcut_error: Option<String>,
    /// Collage via `org.freedesktop.portal.RemoteDesktop` (clavier uniquement).
    pub keyboard: PortalState,
    pub keyboard_error: Option<String>,
    /// Outil de collage de secours détecté : "ydotool" | "wtype" | null.
    pub paste_fallback: Option<String>,
}

/// État partagé (géré par Tauri).
#[derive(Default)]
pub struct WaylandShared(pub Mutex<WaylandStatus>);

/// Instantané de l'état courant.
pub fn status(app: &AppHandle) -> WaylandStatus {
    match app.try_state::<WaylandShared>() {
        Some(s) => s.0.lock().clone(),
        None => WaylandStatus::default(),
    }
}

/// Modifie l'état puis émet `wayland-status` si quelque chose a changé.
pub fn update(app: &AppHandle, f: impl FnOnce(&mut WaylandStatus)) {
    let Some(shared) = app.try_state::<WaylandShared>() else { return };
    let snapshot = {
        let mut st = shared.0.lock();
        let before = st.clone();
        f(&mut st);
        if *st == before {
            return;
        }
        st.clone()
    };
    if let Err(e) = app.emit("wayland-status", &snapshot) {
        log::warn!("émission wayland-status : {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_de_session() {
        assert!(detect_wayland(Some("wayland"), None));
        assert!(detect_wayland(Some("Wayland"), Some("wayland-0")));
        assert!(detect_wayland(None, Some("wayland-0")));
        assert!(detect_wayland(Some("tty"), Some("wayland-1")));
        assert!(!detect_wayland(Some("x11"), None));
        // Session X11 explicite : un WAYLAND_DISPLAY hérité ne compte pas.
        assert!(!detect_wayland(Some("x11"), Some("wayland-0")));
        assert!(!detect_wayland(None, None));
        assert!(!detect_wayland(None, Some("")));
        assert!(!detect_wayland(Some(""), Some("  ")));
    }

    #[test]
    fn accelerateur_vers_portail() {
        assert_eq!(accelerator_to_portal("Ctrl+Alt+Space").as_deref(), Some("CTRL+ALT+space"));
        assert_eq!(accelerator_to_portal("Alt+Space").as_deref(), Some("ALT+space"));
        assert_eq!(accelerator_to_portal("CommandOrControl+Shift+D").as_deref(), Some("CTRL+SHIFT+d"));
        assert_eq!(accelerator_to_portal("Super+KeyM").as_deref(), Some("LOGO+m"));
        // Ordre canonique quel que soit l'ordre d'origine.
        assert_eq!(accelerator_to_portal("Shift+Alt+Control+Digit5").as_deref(), Some("CTRL+ALT+SHIFT+5"));
        assert_eq!(accelerator_to_portal("F9").as_deref(), Some("F9"));
        assert_eq!(accelerator_to_portal("Ctrl+F12").as_deref(), Some("CTRL+F12"));
        assert_eq!(accelerator_to_portal("Alt+Numpad0").as_deref(), Some("ALT+KP_0"));
        assert_eq!(accelerator_to_portal("Ctrl+PageDown").as_deref(), Some("CTRL+Page_Down"));
        assert_eq!(accelerator_to_portal("Ctrl+ArrowUp").as_deref(), Some("CTRL+Up"));
        assert_eq!(accelerator_to_portal("Ctrl+Backquote").as_deref(), Some("CTRL+grave"));
        assert_eq!(accelerator_to_portal("Ctrl+Enter").as_deref(), Some("CTRL+Return"));
        assert_eq!(accelerator_to_portal(" Ctrl + Alt + Space ").as_deref(), Some("CTRL+ALT+space"));
    }

    #[test]
    fn accelerateur_invalide() {
        assert_eq!(accelerator_to_portal(""), None);
        assert_eq!(accelerator_to_portal("Ctrl+Alt"), None); // pas de touche principale
        assert_eq!(accelerator_to_portal("Ctrl++Space"), None);
        assert_eq!(accelerator_to_portal("Ctrl+A+B"), None);
        assert_eq!(accelerator_to_portal("Alt+Pomme"), None);
        assert_eq!(accelerator_to_portal("Ctrl+F25"), None);
    }

    #[test]
    fn statut_serialise_en_snake_case() {
        let s = WaylandStatus { wayland: true, shortcut: PortalState::Active, ..Default::default() };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"shortcut\":\"active\""));
        assert!(json.contains("\"keyboard\":\"inactive\""));
    }
}

//! Pastille flottante (fenêtre `overlay` déclarée dans `tauri.conf.json`).
//!
//! Règle d'or : elle ne doit **jamais** prendre le focus de l'application active,
//! sinon le collage partirait au mauvais endroit. D'où :
//! - `focusable(false)` (macOS : `canBecomeKeyWindow` renvoie NO),
//! - `set_ignore_cursor_events(true)` (les clics traversent),
//! - sur macOS, affichage par `orderFrontRegardless` (n'active pas l'app, ne rend pas
//!   la fenêtre « key »), niveau au-dessus des apps plein écran et comportement
//!   `CanJoinAllSpaces | FullScreenAuxiliary | Stationary | IgnoresCycle`.

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow};

pub const LABEL: &str = "overlay";
/// Distance au bas de la zone de travail (points logiques).
const BOTTOM_MARGIN: f64 = 72.0;

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// Configuration initiale (au setup).
pub fn init(app: &AppHandle) {
    let Some(w) = window(app) else {
        log::warn!("fenêtre overlay introuvable");
        return;
    };
    let log_err = |what: &str, r: tauri::Result<()>| {
        if let Err(e) = r {
            log::warn!("overlay : {what} : {e}");
        }
    };
    log_err("set_focusable", w.set_focusable(false));
    // Linux : la fenêtre, déclarée invisible, n'est pas encore réalisée (pas de GdkWindow) et tao
    // panique (`window().unwrap()`) : on l'applique à chaque affichage, dans `show`.
    #[cfg(not(target_os = "linux"))]
    log_err("set_ignore_cursor_events", w.set_ignore_cursor_events(true));
    log_err("set_always_on_top", w.set_always_on_top(true));
    log_err("set_visible_on_all_workspaces", w.set_visible_on_all_workspaces(true));
    log_err("set_skip_taskbar", w.set_skip_taskbar(true));

    #[cfg(target_os = "macos")]
    {
        let w2 = w.clone();
        log_err(
            "configuration macOS",
            w.run_on_main_thread(move || macos::configure(&w2)),
        );
    }
}

/// Positionne la pastille en bas au centre de l'écran qui contient le curseur.
fn position(app: &AppHandle, w: &WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let Ok(size) = w.outer_size() else { return };
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let y = area.position.y + area.size.height as i32
        - size.height as i32
        - (BOTTOM_MARGIN * scale).round() as i32;
    if let Err(e) = w.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("overlay : positionnement : {e}");
    }
}

pub fn show(app: &AppHandle) {
    let Some(w) = window(app) else { return };
    position(app, &w);
    #[cfg(target_os = "macos")]
    {
        let w2 = w.clone();
        if let Err(e) = w.run_on_main_thread(move || macos::order_front(&w2)) {
            log::warn!("overlay : affichage : {e}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    if let Err(e) = w.show() {
        log::warn!("overlay : affichage : {e}");
    }
    #[cfg(target_os = "linux")]
    if let Err(e) = w.set_ignore_cursor_events(true) {
        log::warn!("overlay : set_ignore_cursor_events : {e}");
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(w) = window(app)
        && let Err(e) = w.hide()
    {
        log::warn!("overlay : masquage : {e}");
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use tauri::WebviewWindow;

    // NSWindowCollectionBehavior
    const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
    const STATIONARY: usize = 1 << 4;
    const IGNORES_CYCLE: usize = 1 << 6;
    const FULL_SCREEN_AUXILIARY: usize = 1 << 8;
    /// NSPopUpMenuWindowLevel (101) : au-dessus des fenêtres plein écran et du Dock.
    const POPUP_MENU_LEVEL: isize = 101;

    fn ns_window(w: &WebviewWindow) -> Option<*mut AnyObject> {
        w.ns_window().ok().map(|p| p.cast::<AnyObject>()).filter(|p| !p.is_null())
    }

    /// À appeler sur le thread principal.
    pub fn configure(w: &WebviewWindow) {
        let Some(ns) = ns_window(w) else { return };
        // SAFETY: pointeur NSWindow valide fourni par Tauri, appel sur le thread principal.
        unsafe {
            let _: () = msg_send![ns, setLevel: POPUP_MENU_LEVEL];
            let behavior =
                CAN_JOIN_ALL_SPACES | STATIONARY | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY;
            let _: () = msg_send![ns, setCollectionBehavior: behavior];
            let _: () = msg_send![ns, setHidesOnDeactivate: false];
            let _: () = msg_send![ns, setIgnoresMouseEvents: true];
        }
    }

    /// Affiche sans activer l'application ni rendre la fenêtre « key ».
    pub fn order_front(w: &WebviewWindow) {
        let Some(ns) = ns_window(w) else { return };
        // Le niveau peut être réinitialisé par Tauri (always_on_top) : on le réapplique.
        configure(w);
        // SAFETY: idem.
        unsafe {
            let _: () = msg_send![ns, orderFrontRegardless];
        }
    }
}

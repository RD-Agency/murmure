//! Raccourci global : modes maintien (push-to-talk), bascule et hybride.
//!
//! La logique de décision ([`HotkeyMachine`]) est pure et testée sans Tauri ;
//! le reste câble `tauri-plugin-global-shortcut` et la file de commandes du pipeline.

use crate::pipeline::{self, Command};
use crate::settings::HotkeyMode;
use crate::state::AppState;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// En mode hybride, un maintien au moins aussi long passe en push-to-talk.
pub const HOLD_THRESHOLD: Duration = Duration::from_millis(300);

const ESCAPE: &str = "Escape";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    Pressed,
    Released,
}

/// Action demandée au pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Start,
    Stop,
    /// Hybride : appui bref → l'enregistrement continue en mains libres.
    HandsFree,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Press {
    /// Touche relâchée.
    #[default]
    Up,
    /// Appui qui a démarré un enregistrement (à l'instant donné).
    Started(Instant),
    /// Appui qui a arrêté un enregistrement : le relâchement est ignoré.
    Stopping,
    /// Appui sans effet (mode bascule) : le relâchement est ignoré.
    Inert,
}

/// Machine d'états du raccourci (pure).
#[derive(Debug, Default)]
pub struct HotkeyMachine {
    press: Press,
}

impl HotkeyMachine {
    /// `recording` : une dictée est-elle en cours d'enregistrement au moment de l'événement ?
    pub fn on_event(&mut self, mode: HotkeyMode, ev: KeyEvent, now: Instant, recording: bool) -> Action {
        match ev {
            KeyEvent::Pressed => {
                // Auto-répétition : la touche est déjà enfoncée.
                if self.press != Press::Up {
                    return Action::None;
                }
                match mode {
                    HotkeyMode::Toggle => {
                        self.press = Press::Inert;
                        if recording { Action::Stop } else { Action::Start }
                    }
                    HotkeyMode::Hold | HotkeyMode::Hybrid => {
                        if recording {
                            // Mains libres (ou démarrage depuis l'UI) : un nouvel appui arrête.
                            self.press = Press::Stopping;
                            Action::Stop
                        } else {
                            self.press = Press::Started(now);
                            Action::Start
                        }
                    }
                }
            }
            KeyEvent::Released => {
                let press = std::mem::take(&mut self.press);
                match (mode, press) {
                    (HotkeyMode::Hold, Press::Started(_)) => Action::Stop,
                    // `recording` n'est volontairement pas consulté ici : le démarrage peut
                    // encore être en file (appui très bref). Le pipeline ignore Stop /
                    // HandsFree si aucune dictée n'enregistre (ex. annulée par Échap).
                    (HotkeyMode::Hybrid, Press::Started(t)) => {
                        if now.saturating_duration_since(t) >= HOLD_THRESHOLD {
                            Action::Stop
                        } else {
                            Action::HandsFree
                        }
                    }
                    _ => Action::None,
                }
            }
        }
    }

    pub fn reset(&mut self) {
        self.press = Press::Up;
    }
}

fn handle_main_shortcut(app: &AppHandle, event: ShortcutEvent) {
    let ev = match event.state {
        ShortcutState::Pressed => KeyEvent::Pressed,
        ShortcutState::Released => KeyEvent::Released,
    };
    let Some(state) = app.try_state::<AppState>() else { return };
    let mode = state.settings.lock().hotkey_mode;
    let recording = state.is_recording();
    let action = state.hotkey.lock().on_event(mode, ev, Instant::now(), recording);
    log::debug!("raccourci {ev:?} (mode {}) → {action:?}", mode_name(mode));
    match action {
        Action::None => {}
        Action::Start => pipeline::send(app, Command::Start { hands_free: mode == HotkeyMode::Toggle }),
        Action::Stop => pipeline::send(app, Command::Stop),
        Action::HandsFree => pipeline::send(app, Command::HandsFree),
    }
}

fn mode_name(mode: HotkeyMode) -> &'static str {
    match mode {
        HotkeyMode::Hybrid => "hybride",
        HotkeyMode::Hold => "maintien",
        HotkeyMode::Toggle => "bascule",
    }
}

/// Raccourci actuellement enregistré (pour pouvoir le désenregistrer à chaud).
#[derive(Default)]
pub struct Registered(parking_lot::Mutex<Option<Shortcut>>);

fn parse(accelerator: &str) -> Result<Shortcut, String> {
    let acc = accelerator.trim();
    if acc.is_empty() {
        return Err("Raccourci vide — choisissez une combinaison de touches.".into());
    }
    acc.parse::<Shortcut>()
        .map_err(|e| format!("Raccourci invalide « {acc} » : {e}"))
}

/// Vérifie qu'un accélérateur est syntaxiquement valide (sans l'enregistrer).
pub fn validate(accelerator: &str) -> Result<(), String> {
    parse(accelerator).map(|_| ())
}

/// (Ré)enregistre le raccourci principal. En cas d'échec, l'ancien est restauré.
pub fn apply(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    let new = parse(accelerator)?;
    let gs = app.global_shortcut();
    let registered = app.state::<Registered>();
    let mut current = registered.0.lock();

    if *current == Some(new) && gs.is_registered(new) {
        return Ok(());
    }
    if let Some(old) = current.take()
        && let Err(e) = gs.unregister(old)
    {
        log::warn!("désenregistrement de l'ancien raccourci : {e}");
    }
    if let Some(s) = app.try_state::<AppState>() {
        s.hotkey.lock().reset();
    }
    let result = gs.on_shortcut(new, |app, _shortcut, event| handle_main_shortcut(app, event));
    match result {
        Ok(()) => {
            log::info!("raccourci global enregistré : {}", new.into_string());
            *current = Some(new);
            Ok(())
        }
        Err(e) => {
            log::error!("enregistrement du raccourci « {} » : {e}", accelerator.trim());
            Err(format!(
                "Impossible d'utiliser le raccourci « {} » : il est peut-être déjà pris par une autre application ({e}).",
                accelerator.trim()
            ))
        }
    }
}

/// Réapplique le raccourci si le réglage a changé ; restaure l'ancien en cas d'échec.
pub fn reapply(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    if old.trim() == new.trim() {
        return Ok(());
    }
    match apply(app, new) {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Err(e2) = apply(app, old) {
                log::error!("restauration de l'ancien raccourci impossible : {e2}");
            }
            Err(e)
        }
    }
}

/// Pendant l'enregistrement : Échap annule la dictée.
pub fn register_escape(app: &AppHandle) {
    let gs = app.global_shortcut();
    if gs.is_registered(ESCAPE) {
        return;
    }
    let res = gs.on_shortcut(ESCAPE, |app, _s, event| {
        if event.state == ShortcutState::Pressed {
            pipeline::send(app, Command::Cancel);
        }
    });
    if let Err(e) = res {
        log::warn!("Échap non enregistrée : {e}");
    }
}

pub fn unregister_escape(app: &AppHandle) {
    let gs = app.global_shortcut();
    if gs.is_registered(ESCAPE)
        && let Err(e) = gs.unregister(ESCAPE)
    {
        log::warn!("désenregistrement d'Échap : {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyEvent::*;

    /// Simule une séquence (événement, délai en ms depuis le début) ; `recording`
    /// suit les actions renvoyées comme le ferait le pipeline.
    fn run(mode: HotkeyMode, seq: &[(KeyEvent, u64)]) -> Vec<Action> {
        let t0 = Instant::now();
        let mut m = HotkeyMachine::default();
        let mut recording = false;
        seq.iter()
            .map(|&(ev, ms)| {
                let a = m.on_event(mode, ev, t0 + Duration::from_millis(ms), recording);
                match a {
                    Action::Start => recording = true,
                    Action::Stop => recording = false,
                    _ => {}
                }
                a
            })
            .collect()
    }

    #[test]
    fn hybrid_long_hold_is_push_to_talk() {
        assert_eq!(run(HotkeyMode::Hybrid, &[(Pressed, 0), (Released, 1200)]), [Action::Start, Action::Stop]);
    }

    #[test]
    fn hybrid_threshold_is_inclusive() {
        assert_eq!(run(HotkeyMode::Hybrid, &[(Pressed, 0), (Released, 300)]), [Action::Start, Action::Stop]);
        assert_eq!(run(HotkeyMode::Hybrid, &[(Pressed, 0), (Released, 299)]), [Action::Start, Action::HandsFree]);
    }

    #[test]
    fn hybrid_tap_then_second_press_stops() {
        let got = run(
            HotkeyMode::Hybrid,
            &[(Pressed, 0), (Released, 120), (Pressed, 4000), (Released, 4100)],
        );
        assert_eq!(got, [Action::Start, Action::HandsFree, Action::Stop, Action::None]);
    }

    #[test]
    fn hybrid_second_press_held_long_still_only_stops() {
        let got = run(
            HotkeyMode::Hybrid,
            &[(Pressed, 0), (Released, 100), (Pressed, 2000), (Released, 3000)],
        );
        assert_eq!(got, [Action::Start, Action::HandsFree, Action::Stop, Action::None]);
    }

    #[test]
    fn auto_repeat_is_ignored() {
        let got = run(
            HotkeyMode::Hybrid,
            &[(Pressed, 0), (Pressed, 30), (Pressed, 60), (Pressed, 500), (Released, 900)],
        );
        assert_eq!(got, [Action::Start, Action::None, Action::None, Action::None, Action::Stop]);
        let got = run(HotkeyMode::Toggle, &[(Pressed, 0), (Pressed, 40), (Released, 80)]);
        assert_eq!(got, [Action::Start, Action::None, Action::None]);
    }

    #[test]
    fn hybrid_release_after_cancel_restarts_cleanly() {
        let t0 = Instant::now();
        let mut m = HotkeyMachine::default();
        assert_eq!(m.on_event(HotkeyMode::Hybrid, Pressed, t0, false), Action::Start);
        // Échap a annulé pendant le maintien : le Stop émis est ignoré par le pipeline.
        let a = m.on_event(HotkeyMode::Hybrid, Released, t0 + Duration::from_millis(900), false);
        assert_eq!(a, Action::Stop);
        // Le prochain appui redémarre normalement.
        assert_eq!(m.on_event(HotkeyMode::Hybrid, Pressed, t0 + Duration::from_secs(2), false), Action::Start);
    }

    #[test]
    fn hybrid_very_fast_tap_before_start_is_processed() {
        // Le relâchement arrive alors que le pipeline n'a pas encore démarré l'enregistrement.
        let t0 = Instant::now();
        let mut m = HotkeyMachine::default();
        assert_eq!(m.on_event(HotkeyMode::Hybrid, Pressed, t0, false), Action::Start);
        let a = m.on_event(HotkeyMode::Hybrid, Released, t0 + Duration::from_millis(20), false);
        assert_eq!(a, Action::HandsFree);
    }

    #[test]
    fn hold_mode() {
        assert_eq!(run(HotkeyMode::Hold, &[(Pressed, 0), (Released, 50)]), [Action::Start, Action::Stop]);
        assert_eq!(
            run(HotkeyMode::Hold, &[(Pressed, 0), (Released, 2000), (Pressed, 3000), (Released, 3500)]),
            [Action::Start, Action::Stop, Action::Start, Action::Stop]
        );
    }

    #[test]
    fn hold_mode_press_while_recording_from_ui_stops() {
        let t0 = Instant::now();
        let mut m = HotkeyMachine::default();
        assert_eq!(m.on_event(HotkeyMode::Hold, Pressed, t0, true), Action::Stop);
        assert_eq!(m.on_event(HotkeyMode::Hold, Released, t0, false), Action::None);
    }

    #[test]
    fn toggle_mode() {
        let got = run(
            HotkeyMode::Toggle,
            &[(Pressed, 0), (Released, 50), (Pressed, 3000), (Released, 3050)],
        );
        assert_eq!(got, [Action::Start, Action::None, Action::Stop, Action::None]);
    }

    #[test]
    fn stray_release_is_ignored() {
        for mode in [HotkeyMode::Hybrid, HotkeyMode::Hold, HotkeyMode::Toggle] {
            assert_eq!(run(mode, &[(Released, 0)]), [Action::None]);
        }
    }

    #[test]
    fn parse_validation() {
        assert!(validate("Alt+Space").is_ok());
        assert!(validate("Ctrl+Alt+Space").is_ok());
        assert!(validate("CommandOrControl+Shift+D").is_ok());
        assert!(validate("").is_err());
        assert!(validate("Alt+Pomme").is_err());
    }
}

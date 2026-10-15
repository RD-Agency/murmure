// SPDX-License-Identifier: AGPL-3.0-or-later
//! Arrêt automatique sur silence (modes bascule et mains libres).
//!
//! [`SilenceDetector`] reçoit le niveau du micro (0..1, échelle en dB de `audio::rms_to_level`,
//! environ 30 fois par seconde) et signale quand un silence a duré assez longtemps **après** de la
//! parole : tant que l'utilisateur n'a rien dit, la dictée attend.
//!
//! Le seuil s'adapte au bruit de fond : le plancher suit le minimum du niveau et remonte lentement,
//! la parole doit le dépasser d'environ 12 dB, et le silence repasser sous ce seuil moins 4 dB
//! (hystérésis : une voix qui faiblit en fin de mot ne compte pas comme un silence).

use std::time::{Duration, Instant};

/// Niveau minimal de la parole (-45 dB), même dans une pièce très calme.
const MIN_VOICE: f32 = 0.30;
/// Marge de la parole au-dessus du bruit de fond (12 dB sur l'échelle 0..1 de 50 dB).
const VOICE_MARGIN: f32 = 0.24;
/// Hystérésis entre parole et silence (4 dB).
const HYSTERESIS: f32 = 0.08;
/// Remontée du plancher par mesure (environ 1,5 dB par seconde à 30 mesures par seconde).
const FLOOR_RISE: f32 = 0.001;

#[derive(Debug)]
pub struct SilenceDetector {
    hold: Duration,
    floor: Option<f32>,
    voiced: bool,
    quiet_since: Option<Instant>,
}

impl SilenceDetector {
    /// `hold` : durée de silence qui termine la dictée.
    pub fn new(hold: Duration) -> Self {
        Self { hold, floor: None, voiced: false, quiet_since: None }
    }

    /// Ajoute une mesure ; vrai tant que le silence qui suit la parole dure depuis au moins `hold`.
    pub fn feed(&mut self, level: f32, now: Instant) -> bool {
        let level = level.clamp(0.0, 1.0);
        let floor = match self.floor {
            Some(f) if level < f => level,
            Some(f) => (f + FLOOR_RISE).min(level),
            None => level,
        };
        self.floor = Some(floor);
        let voice_threshold = (floor + VOICE_MARGIN).max(MIN_VOICE);
        if level >= voice_threshold {
            self.voiced = true;
            self.quiet_since = None;
            return false;
        }
        if !self.voiced {
            return false;
        }
        if level >= voice_threshold - HYSTERESIS {
            // Zone intermédiaire : ni parole franche ni silence, le compte à rebours continue.
            return self.quiet_since.is_some_and(|t| now.saturating_duration_since(t) >= self.hold);
        }
        let since = *self.quiet_since.get_or_insert(now);
        now.saturating_duration_since(since) >= self.hold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(33);

    /// Rejoue une suite de (niveau, nombre de mesures) ; renvoie l'instant (en ms) du premier signal.
    fn first_fire(hold_ms: u64, seq: &[(f32, usize)]) -> Option<u64> {
        let t0 = Instant::now();
        let mut d = SilenceDetector::new(Duration::from_millis(hold_ms));
        let mut i = 0u32;
        for &(level, n) in seq {
            for _ in 0..n {
                let now = t0 + FRAME * i;
                if d.feed(level, now) {
                    return Some((FRAME * i).as_millis() as u64);
                }
                i += 1;
            }
        }
        None
    }

    #[test]
    fn silence_apres_parole_termine() {
        // Bruit de fond, une seconde de parole, puis silence : signal après environ 2 s de silence.
        let fired = first_fire(2000, &[(0.10, 15), (0.70, 30), (0.10, 120)]).expect("devait s'arrêter");
        let silence_start = 45 * 33;
        assert!((silence_start + 1980..=silence_start + 2100).contains(&fired), "{fired}");
    }

    #[test]
    fn rien_avant_la_parole() {
        // Dix secondes de silence sans avoir parlé : on attend.
        assert_eq!(first_fire(2000, &[(0.05, 300)]), None);
    }

    #[test]
    fn courtes_pauses_ignorees() {
        // Pauses d'une seconde entre les phrases : jamais d'arrêt.
        let mut seq = vec![(0.10, 10)];
        for _ in 0..8 {
            seq.push((0.70, 40));
            seq.push((0.10, 30));
        }
        assert_eq!(first_fire(2000, &seq), None);
    }

    #[test]
    fn piece_bruyante() {
        // Bruit de fond à -40 dB (0,4), voix à -15 dB (0,9) : le seuil s'adapte.
        assert!(first_fire(1500, &[(0.40, 30), (0.90, 30), (0.40, 80)]).is_some());
        // Le bruit seul ne déclenche rien, même s'il dépasse le minimum absolu de la parole.
        assert_eq!(first_fire(1500, &[(0.40, 200)]), None);
    }

    #[test]
    fn voix_qui_faiblit_ne_compte_pas_comme_silence() {
        // Juste sous le seuil de parole (zone d'hystérésis) : le compte à rebours ne démarre pas.
        assert_eq!(first_fire(300, &[(0.10, 10), (0.70, 30), (0.33, 15)]), None);
        // Un vrai silence de même durée, lui, termine.
        assert!(first_fire(300, &[(0.10, 10), (0.70, 30), (0.10, 15)]).is_some());
    }

    #[test]
    fn le_signal_persiste() {
        let t0 = Instant::now();
        let mut d = SilenceDetector::new(Duration::from_millis(500));
        d.feed(0.1, t0);
        d.feed(0.8, t0 + FRAME);
        assert!(!d.feed(0.05, t0 + FRAME * 2));
        assert!(d.feed(0.05, t0 + FRAME * 2 + Duration::from_millis(600)));
        // Toujours vrai tant que le silence dure (le pipeline décide quand agir).
        assert!(d.feed(0.05, t0 + FRAME * 2 + Duration::from_millis(700)));
        // La parole qui reprend remet le compteur à zéro.
        assert!(!d.feed(0.8, t0 + Duration::from_secs(1)));
        assert!(!d.feed(0.05, t0 + Duration::from_millis(1100)));
    }
}

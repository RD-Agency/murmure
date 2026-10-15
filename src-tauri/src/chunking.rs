// SPDX-License-Identifier: AGPL-3.0-or-later
//! Découpage de la dictée en segments sur les silences, et transcription au fil de l'enregistrement.
//!
//! Pendant l'enregistrement, chaque bloc d'audio en 16 kHz passe par un détecteur d'activité vocale
//! (énergie RMS par trame de 30 ms, plancher de bruit adaptatif). Une fois le segment courant long
//! d'au moins [`SegmentConfig::min_segment`], le premier silence d'au moins
//! [`SegmentConfig::min_silence`] le termine : il part aussitôt en transcription dans un thread de fond
//! pendant que l'enregistrement continue. Sans silence, le segment est coupé de force avant
//! [`SegmentConfig::max_segment`] (Whisper travaille par fenêtres de 30 s), au point le plus calme des
//! dernières secondes, avec un recouvrement d'une seconde si ce point tombe en pleine parole ; les mots
//! répétés à la jointure sont retirés. Au relâchement, seul le dernier segment reste à transcrire.
//!
//! Le texte brut des segments est assemblé ici ; la ponctuation dictée s'applique ensuite au texte
//! complet (dans `pipeline.rs`), si bien qu'une commande à cheval sur une coupe est reconnue.
//!
//! [`split_on_silence`] et [`transcribe_segments`] découpent et transcrivent un audio déjà complet
//! (fichier audio), avec les mêmes règles.

use crate::engine::{self, TranscribeOptions};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Taux d'échantillonnage de l'audio découpé (celui de Whisper).
pub const SAMPLE_RATE: usize = 16_000;
/// Trame d'analyse du détecteur d'activité vocale : 30 ms.
const FRAME: usize = SAMPLE_RATE * 30 / 1000;
/// Seuil de parole : RMS supérieur au plancher de bruit d'au moins ce facteur (environ 10 dB).
const SPEECH_RATIO: f32 = 3.0;
/// Seuil absolu : en dessous, une trame est toujours du silence (même valeur que l'abandon sur
/// silence de `pipeline.rs`).
const SPEECH_MIN_RMS: f32 = 0.002;
/// Plancher de bruit minimal, pour qu'un silence numérique ne fige pas le seuil à zéro.
const FLOOR_MIN: f32 = 1e-4;
/// Plancher initial au plus : bruit de fond d'un micro dans un bureau calme. Si la dictée commence
/// par la voix, le plancher ne part pas de son niveau.
const FLOOR_INIT_MAX: f32 = 0.003;
/// Trame forte : comptée comme parole même si le détecteur l'a classée en silence (segment jamais
/// écarté par erreur).
const LOUD_RMS: f32 = 0.01;
/// Pause assez longue, au moment d'une coupe forcée, pour couper sans recouvrement : 150 ms.
const FORCED_GAP_FRAMES: usize = 5;
/// Nombre de mots du texte déjà transcrit repris dans le prompt du segment suivant.
const PROMPT_TAIL_WORDS: usize = 30;
/// Recherche des doublons à la jointure : mots comparés au plus, et mots tronqués tolérés.
const DEDUP_MAX_WORDS: usize = 8;
const DEDUP_SKIP_NEXT: usize = 2;
const DEDUP_DROP_PREV: usize = 1;

/// Règles de découpage, en échantillons à 16 kHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentConfig {
    /// Pas de coupe sur silence avant cette durée de segment.
    pub min_segment: usize,
    /// Silence qui termine un segment une fois `min_segment` atteint.
    pub min_silence: usize,
    /// Coupe forcée au plus tard à cette durée (sous les 30 s d'une fenêtre Whisper).
    pub max_segment: usize,
    /// Recouvrement d'une coupe forcée en pleine parole.
    pub overlap: usize,
    /// Zone, avant `max_segment`, où chercher le point le plus calme d'une coupe forcée.
    pub search: usize,
}

impl Default for SegmentConfig {
    /// Règles avec GPU : un segment ne coûte que quelques dixièmes de seconde, on coupe tôt.
    fn default() -> Self {
        Self {
            min_segment: 15 * SAMPLE_RATE,
            min_silence: SAMPLE_RATE / 2,
            max_segment: 28 * SAMPLE_RATE,
            overlap: SAMPLE_RATE,
            search: 4 * SAMPLE_RATE,
        }
    }
}

impl SegmentConfig {
    /// Règles sur processeur : chaque appel à Whisper y coûte une fenêtre de 30 s entière (environ 7 s
    /// sur 8 threads d'un Ryzen 9 9950X), quelle que soit la durée du segment. Couper une dictée de
    /// moins de 30 s ne ferait qu'ajouter un appel ; on ne coupe donc qu'à 28 s, en remontant jusqu'à
    /// la plus longue pause des 8 dernières secondes.
    pub fn cpu() -> Self {
        Self { min_segment: 28 * SAMPLE_RATE, search: 8 * SAMPLE_RATE, ..Self::default() }
    }

    /// Règles adaptées au moteur : avec ou sans GPU.
    pub fn for_engine(gpu: bool) -> Self {
        if gpu { Self::default() } else { Self::cpu() }
    }
}

/// Un segment de l'audio : `[start, end)` en échantillons, recouvrement compris.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    /// Échantillons du début du segment déjà présents à la fin du précédent (coupe forcée).
    pub overlap: usize,
    /// Au moins une trame de parole ou une trame forte : un segment qui n'en a aucune n'est pas
    /// transcrit (Whisper invente du texte sur le silence).
    pub speech: bool,
}

impl Segment {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// Détecteur d'activité vocale par énergie, à plancher de bruit adaptatif : le plancher descend vite
/// vers les trames calmes et ne remonte que lentement, si bien qu'il suit le bruit de fond de la pièce
/// sans se laisser entraîner par la voix.
#[derive(Clone, Debug, Default)]
pub struct Vad {
    floor: Option<f32>,
}

impl Vad {
    /// Classe une trame d'après son RMS : `true` si c'est de la parole.
    pub fn is_speech(&mut self, rms: f32) -> bool {
        // Descente rapide (≈ 150 ms) vers une trame plus calme, remontée lente (≈ 1 min).
        let floor = match self.floor {
            None => rms.min(FLOOR_INIT_MAX),
            Some(f) if rms < f => f * 0.8 + rms * 0.2,
            Some(f) => f * 0.9995 + rms * 0.0005,
        }
        .max(FLOOR_MIN);
        self.floor = Some(floor);
        rms > SPEECH_MIN_RMS && rms > floor * SPEECH_RATIO
    }
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f64 = frame.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / frame.len() as f64).sqrt() as f32
}

/// Découpeur en continu : on lui pousse l'audio au fil de l'eau, il renvoie les segments terminés.
#[derive(Clone, Debug)]
pub struct Segmenter {
    cfg: SegmentConfig,
    vad: Vad,
    /// Fin de la dernière trame analysée.
    analyzed: usize,
    pending: Vec<f32>,
    /// Début du segment courant (recouvrement compris).
    start: usize,
    overlap: usize,
    speech: bool,
    /// Début du silence en cours, s'il y en a un.
    silence_since: Option<usize>,
    /// Trames du segment courant : (début, RMS, parole).
    frames: Vec<(usize, f32, bool)>,
}

impl Segmenter {
    pub fn new(cfg: SegmentConfig) -> Self {
        Self {
            cfg,
            vad: Vad::default(),
            analyzed: 0,
            pending: Vec::with_capacity(FRAME),
            start: 0,
            overlap: 0,
            speech: false,
            silence_since: None,
            frames: Vec::new(),
        }
    }

    /// Début du segment en cours.
    pub fn current_start(&self) -> usize {
        self.start
    }

    /// Analyse de nouveaux échantillons ; renvoie les segments terminés.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Segment> {
        let mut done = Vec::new();
        let mut rest = samples;
        while !rest.is_empty() {
            let take = (FRAME - self.pending.len()).min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() == FRAME {
                let level = rms(&self.pending);
                self.pending.clear();
                if let Some(seg) = self.on_frame(level) {
                    done.push(seg);
                }
            }
        }
        done
    }

    fn on_frame(&mut self, level: f32) -> Option<Segment> {
        let at = self.analyzed;
        self.analyzed += FRAME;
        let speech = self.vad.is_speech(level);
        self.frames.push((at, level, speech));
        if speech || level > LOUD_RMS {
            self.speech = true;
        }
        if speech {
            self.silence_since = None;
        } else if self.silence_since.is_none() {
            self.silence_since = Some(at);
        }

        // Coupe sur silence : au milieu du silence requis, une fois le segment assez long.
        if let Some(since) = self.silence_since
            && self.speech
            && self.analyzed - since >= self.cfg.min_silence
            && since.saturating_sub(self.start) >= self.cfg.min_segment
        {
            return Some(self.cut(since + self.cfg.min_silence / 2, 0));
        }
        // Coupe forcée : au point le plus calme de la fin du segment.
        if self.analyzed - self.start >= self.cfg.max_segment {
            let (cut, gap) = self.quietest_point();
            return Some(self.cut(cut, if gap { 0 } else { self.cfg.overlap }));
        }
        None
    }

    /// Point de coupe forcée : milieu de la plus longue pause de la zone de recherche si elle dure
    /// au moins 150 ms (`true` : pas besoin de recouvrement), sinon trame la plus calme.
    fn quietest_point(&self) -> (usize, bool) {
        let from = (self.start + self.cfg.max_segment).saturating_sub(self.cfg.search).max(self.start + self.overlap);
        let zone: Vec<_> = self.frames.iter().filter(|f| f.0 >= from).collect();
        let (mut best_run, mut best_mid, mut run) = (0usize, 0usize, 0usize);
        for (i, f) in zone.iter().enumerate() {
            run = if f.2 { 0 } else { run + 1 };
            if run > best_run {
                best_run = run;
                best_mid = zone[i + 1 - run].0 + run * FRAME / 2;
            }
        }
        if best_run >= FORCED_GAP_FRAMES {
            return (best_mid, true);
        }
        let quiet = zone
            .iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(self.analyzed, |f| f.0 + FRAME / 2);
        (quiet, false)
    }

    /// Termine le segment courant en `cut` ; le suivant commence `overlap` échantillons plus tôt.
    fn cut(&mut self, cut: usize, overlap: usize) -> Segment {
        let seg = Segment { start: self.start, end: cut, overlap: self.overlap, speech: self.speech };
        let next = cut.saturating_sub(overlap).max(self.start);
        self.start = next;
        self.overlap = cut - next;
        self.frames.retain(|f| f.0 >= next);
        self.speech = self.frames.iter().any(|f| f.2 || f.1 > LOUD_RMS);
        self.silence_since = None;
        // Silence en cours prolongé dans le nouveau segment : il ne compte qu'à partir de la coupe.
        if let Some(last) = self.frames.last()
            && !last.2
        {
            self.silence_since = Some(self.frames.iter().rev().take_while(|f| !f.2).last().map_or(last.0, |f| f.0).max(next));
        }
        seg
    }

    /// Dernier segment, jusqu'à `total` (longueur finale de l'audio). Les trames incomplètes de la
    /// fin comptent comme parole si elles dépassent le seuil absolu.
    pub fn finish(&mut self, total: usize) -> Option<Segment> {
        let total = total.max(self.start);
        if !self.pending.is_empty() && rms(&self.pending) > SPEECH_MIN_RMS {
            self.speech = true;
        }
        self.pending.clear();
        let seg = Segment { start: self.start, end: total, overlap: self.overlap, speech: self.speech };
        self.start = total;
        self.overlap = 0;
        self.speech = false;
        self.frames.clear();
        (!seg.is_empty()).then_some(seg)
    }
}

/// Découpe un audio complet (mono, 16 kHz) en segments sur les silences, avec les règles de la
/// dictée. Les segments couvrent tout l'audio, dans l'ordre ; ceux sans parole sont marqués
/// (`speech: false`). Pour un fichier audio : transcrire chaque segment avec
/// [`transcribe_segments`], qui gère le prompt de continuité et la jointure.
pub fn split_on_silence(samples: &[f32], cfg: SegmentConfig) -> Vec<Segment> {
    let mut seg = Segmenter::new(cfg);
    let mut out = seg.push(samples);
    out.extend(seg.finish(samples.len()));
    out
}

/// Transcrit des segments dans l'ordre, chacun avec la fin du texte déjà transcrit en prompt, et
/// assemble le texte brut (sans ponctuation dictée). `transcribe` reçoit l'audio du segment et ses
/// options.
pub fn transcribe_segments(
    samples: &[f32],
    segments: &[Segment],
    opts: &TranscribeOptions,
    mut transcribe: impl FnMut(&[f32], &TranscribeOptions) -> Result<String, String>,
) -> Result<String, String> {
    let mut text = Transcript::default();
    for seg in segments.iter().filter(|s| s.speech) {
        let part = transcribe(&samples[seg.start..seg.end.min(samples.len())], &text.options(opts))?;
        text.push(&part, seg.overlap > 0);
    }
    Ok(text.into_text())
}

/// Texte brut assemblé segment par segment.
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    text: String,
}

impl Transcript {
    /// Ajoute le texte d'un segment ; `overlap` : le segment recouvre la fin du précédent.
    pub fn push(&mut self, part: &str, overlap: bool) {
        self.text = join(&self.text, part, overlap);
    }

    /// Options du segment suivant : la fin du texte déjà transcrit s'ajoute au prompt initial.
    pub fn options(&self, base: &TranscribeOptions) -> TranscribeOptions {
        let tail = prompt_tail(&self.text, PROMPT_TAIL_WORDS);
        let initial_prompt = match (base.prompt(), tail.is_empty()) {
            (_, true) => base.initial_prompt.clone(),
            (None, false) => Some(tail.to_string()),
            (Some(p), false) => Some(format!("{p} {tail}")),
        };
        TranscribeOptions { language: base.language.clone(), initial_prompt }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }
}

/// Les `max_words` derniers mots de `text`.
pub fn prompt_tail(text: &str, max_words: usize) -> &str {
    let text = text.trim_end();
    let mut count = 0;
    let mut start = text.len();
    for (i, _) in text.match_indices(char::is_whitespace).collect::<Vec<_>>().into_iter().rev() {
        if !text[i + 1..start].is_empty() {
            count += 1;
            if count == max_words {
                return text[i + 1..].trim_start();
            }
        }
        start = i;
    }
    text.trim_start()
}

/// Forme comparable d'un mot : minuscules, lettres, chiffres et apostrophes seulement.
fn norm(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '’')
        .map(|c| if c == '’' { '\'' } else { c })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Mots-outils que Whisper capitalise au début d'un segment comme en début de phrase.
const LOWER_AFTER_JOIN: &[&str] = &[
    "je", "j'", "nous", "vous", "il", "elle", "ils", "elles", "on", "le", "la", "les", "l'", "un", "une", "des",
    "de", "du", "d'", "que", "qu'", "qui", "et", "mais", "ou", "donc", "car", "ce", "c'", "cette", "ces", "en",
    "dans", "pour", "par", "sur", "avec", "sans", "si", "puis", "à", "au", "aux", "est", "sont", "mon", "ma",
    "mes", "son", "sa", "ses", "notre", "nos", "votre", "vos", "leur", "leurs", "ne", "n'", "y", "se", "s'",
];

/// Assemble le texte de deux segments consécutifs. Après une coupe forcée (`overlap`), les mots
/// répétés à la jointure ne sont gardés qu'une fois, dans la version du segment suivant, qui les a
/// entendus en entier ; jusqu'à deux mots tronqués au début du suivant et un à la fin du précédent
/// sont tolérés (un seul quand la répétition ne porte que sur un mot). Si la phrase continue (pas de ponctuation finale), le mot-outil que Whisper a mis
/// en majuscule au début du segment repasse en minuscule, et un point final ajouté par Whisper à la
/// fin du précédent disparaît quand le suivant commence par une minuscule.
pub fn join(prev: &str, next: &str, overlap: bool) -> String {
    let prev_words: Vec<&str> = prev.split_whitespace().collect();
    let mut next_words: Vec<&str> = next.split_whitespace().collect();
    if prev_words.is_empty() {
        return next_words.join(" ");
    }
    if next_words.is_empty() {
        return prev_words.join(" ");
    }
    let mut keep = prev_words.len();
    if overlap && let Some((k, j, i)) = find_overlap(&prev_words, &next_words) {
        keep = prev_words.len() - j - k;
        next_words.drain(..i);
    }
    let mut head: Vec<String> = prev_words[..keep].iter().map(|w| w.to_string()).collect();
    let Some(first) = next_words.first().copied() else {
        return head.join(" ");
    };
    let mut first = first.to_string();
    let ends_sentence = |w: &str| w.ends_with(['.', '?', '!', '…', ':']);
    if let Some(last) = head.last_mut() {
        let starts_lower = first.chars().next().is_some_and(char::is_lowercase);
        if starts_lower && last.ends_with('.') && !last.ends_with("..") {
            last.pop();
        }
        if !ends_sentence(last) {
            let lower = first.to_lowercase();
            if LOWER_AFTER_JOIN.iter().any(|w| lower == *w || lower.trim_end_matches([',', '.']) == *w || (w.ends_with('\'') && lower.starts_with(w))) {
                first = lower_first(&first);
            }
        }
    }
    head.push(first);
    head.extend(next_words[1..].iter().map(|w| w.to_string()));
    head.join(" ")
}

fn lower_first(w: &str) -> String {
    let mut c = w.chars();
    c.next().map_or_else(String::new, |f| f.to_lowercase().collect::<String>() + c.as_str())
}

/// Recouvrement entre la fin de `prev` et le début de `next` : `(k, j, i)` = `k` mots identiques,
/// `j` mots tronqués retirés de la fin de `prev`, `i` du début de `next`. Le plus long d'abord,
/// puis le moins de mots tronqués.
fn find_overlap(prev: &[&str], next: &[&str]) -> Option<(usize, usize, usize)> {
    let p: Vec<String> = prev.iter().map(|w| norm(w)).collect();
    let n: Vec<String> = next.iter().map(|w| norm(w)).collect();
    for k in (1..=DEDUP_MAX_WORDS.min(p.len()).min(n.len())).rev() {
        let mut best: Option<(usize, usize, usize)> = None;
        for j in 0..=DEDUP_DROP_PREV {
            for i in 0..=DEDUP_SKIP_NEXT {
                if (k == 1 && (i > 1 || j > 0)) || p.len() < j + k || n.len() < i + k {
                    continue;
                }
                let tail = &p[p.len() - j - k..p.len() - j];
                let head = &n[i..i + k];
                if tail.iter().any(String::is_empty) || tail != head {
                    continue;
                }
                if k == 1 && tail[0].chars().count() < 2 {
                    continue;
                }
                if best.is_none_or(|(_, bj, bi)| i + j < bi + bj) {
                    best = Some((k, j, i));
                }
            }
        }
        if best.is_some() {
            return best;
        }
    }
    None
}

/// Transcription d'un segment pendant l'enregistrement : audio, options, drapeau d'annulation.
pub type SegmentFn =
    dyn FnMut(&[f32], &TranscribeOptions, &Arc<AtomicBool>) -> Result<engine::Transcript, String> + Send;

/// Une dictée transcrite au fil de l'enregistrement. L'audio arrive par [`ChunkedSession::sink`]
/// (thread audio) ; les segments terminés sont transcrits dans l'ordre par un thread de fond ;
/// [`ChunkedSession::finish`] transcrit le dernier et renvoie le texte brut assemblé, avec la
/// confiance des mots de tous les segments. Abandonner
/// la session (drop) annule les transcriptions en cours.
pub struct ChunkedSession {
    feeder: Arc<Mutex<Feeder>>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<engine::Transcript, String>>>,
}

struct Feeder {
    segmenter: Segmenter,
    audio: Vec<f32>,
    jobs: Option<Sender<Job>>,
    sent: usize,
}

struct Job {
    samples: Vec<f32>,
    overlap: bool,
}

impl Feeder {
    fn send(&mut self, seg: Segment, audio: &[f32]) {
        if !seg.speech {
            return;
        }
        if let Some(tx) = &self.jobs {
            let end = seg.end.min(audio.len());
            let samples = audio[seg.start.min(end)..end].to_vec();
            if tx.send(Job { samples, overlap: seg.overlap > 0 }).is_ok() {
                self.sent += 1;
            }
        }
    }

    fn push(&mut self, samples: &[f32]) {
        self.audio.extend_from_slice(samples);
        for seg in self.segmenter.push(samples) {
            let audio = std::mem::take(&mut self.audio);
            self.send(seg, &audio);
            self.audio = audio;
        }
    }
}

impl ChunkedSession {
    /// Démarre le thread de transcription. `opts` porte la langue et le prompt de base
    /// (vocabulaire, `PROMPT_FR`) ; la fin du texte transcrit y est ajoutée à chaque segment.
    pub fn start(
        cfg: SegmentConfig,
        opts: TranscribeOptions,
        cancel: Arc<AtomicBool>,
        mut transcribe: Box<SegmentFn>,
    ) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let flag = cancel.clone();
        let worker = std::thread::Builder::new().name("murmure-segments".into()).spawn(move || {
            let mut text = Transcript::default();
            let mut words = Vec::new();
            for (n, job) in rx.into_iter().enumerate() {
                if flag.load(Ordering::Relaxed) {
                    return Err("transcription annulée".to_string());
                }
                let started = std::time::Instant::now();
                let part = transcribe(&job.samples, &text.options(&opts), &flag)?;
                log::debug!(
                    "segment {} ({:.1} s) transcrit en {} ms",
                    n + 1,
                    job.samples.len() as f32 / SAMPLE_RATE as f32,
                    started.elapsed().as_millis()
                );
                text.push(&part.text, job.overlap);
                // Les mots du recouvrement sont en double : l'alignement de la correction les saute.
                words.extend(part.words);
            }
            Ok(engine::Transcript { text: text.into_text(), words })
        })?;
        let feeder = Feeder { segmenter: Segmenter::new(cfg), audio: Vec::new(), jobs: Some(tx), sent: 0 };
        Ok(Self { feeder: Arc::new(Mutex::new(feeder)), cancel, worker: Some(worker) })
    }

    /// Fonction à donner au thread audio : reçoit l'audio en 16 kHz au fil de l'eau.
    pub fn sink(&self) -> impl FnMut(&[f32]) + Send + 'static {
        let feeder = self.feeder.clone();
        move |samples: &[f32]| feeder.lock().push(samples)
    }

    /// Drapeau d'annulation partagé avec les transcriptions en cours.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Segments déjà partis en transcription.
    pub fn segments_sent(&self) -> usize {
        self.feeder.lock().sent
    }

    /// Envoie le dernier segment (jusqu'à la fin de `full`, l'audio complet de la dictée) et attend
    /// le texte brut assemblé. Bloquant.
    pub fn finish(mut self, full: &[f32]) -> Result<engine::Transcript, String> {
        {
            let mut f = self.feeder.lock();
            // Fin de l'audio que le thread audio n'a pas transmise (vidange du rééchantillonneur).
            if full.len() > f.audio.len() {
                let extra = full[f.audio.len()..].to_vec();
                f.push(&extra);
            }
            if let Some(seg) = f.segmenter.finish(full.len()) {
                f.send(seg, full);
            }
            f.jobs = None;
        }
        let worker = self.worker.take().ok_or("transcription déjà terminée")?;
        worker.join().map_err(|_| "le thread de transcription s'est arrêté brutalement".to_string())?
    }
}

impl Drop for ChunkedSession {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.cancel.store(true, Ordering::Relaxed);
            self.feeder.lock().jobs = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: usize = SAMPLE_RATE;

    /// Parole simulée : mots de 400 ms (bruit d'amplitude 0,1) séparés de 90 ms de bruit de fond,
    /// comme les creux entre les mots ; silence : bruit de fond d'amplitude 0,0005.
    fn signal(parts: &[(bool, f32)]) -> Vec<f32> {
        let mut seed = 0x2545F491u32;
        let mut out = Vec::new();
        for &(speech, secs) in parts {
            for i in 0..(secs * S as f32) as usize {
                let in_word = speech && i % (S * 49 / 100) < S * 40 / 100;
                let amp = if in_word { 0.1 } else { 0.0005 };
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                out.push((seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp);
            }
        }
        out
    }

    fn secs(n: usize) -> f32 {
        n as f32 / S as f32
    }

    #[test]
    fn vad_plancher_adaptatif() {
        let mut vad = Vad::default();
        // Bruit de fond de la pièce, puis voix : seule la voix est de la parole.
        assert!((0..50).all(|_| !vad.is_speech(0.004)));
        assert!(vad.is_speech(0.05));
        // Pièce bruyante : le plancher rattrape le bruit en quelques secondes, la voix reste détectée.
        let mut vad = Vad::default();
        let noisy: Vec<bool> = (0..3000).map(|_| vad.is_speech(0.02)).collect();
        assert!(!noisy[1000..].iter().any(|&s| s));
        assert!(vad.is_speech(0.2));
        // La voix dès la première trame ne fige pas le plancher à son niveau.
        let mut vad = Vad::default();
        assert!(vad.is_speech(0.05));
        assert!((0..200).all(|i| vad.is_speech(if i % 16 < 13 { 0.05 } else { 0.001 }) || i % 16 >= 13));
        // Silence numérique : seuil absolu.
        let mut vad = Vad::default();
        assert!(!vad.is_speech(0.0));
        assert!(!vad.is_speech(0.0015));
    }

    #[test]
    fn dictee_courte_un_seul_segment() {
        let audio = signal(&[(false, 0.3), (true, 4.0), (false, 0.8), (true, 5.0), (false, 0.2)]);
        let segs = split_on_silence(&audio, SegmentConfig::default());
        assert_eq!(segs.len(), 1);
        assert_eq!((segs[0].start, segs[0].end, segs[0].overlap, segs[0].speech), (0, audio.len(), 0, true));
    }

    #[test]
    fn coupe_sur_silence_apres_15_s() {
        // Une pause de 0,8 s à 8 s (trop tôt), puis une à 17 s : coupe dans la seconde.
        let audio = signal(&[(true, 8.0), (false, 0.8), (true, 8.2), (false, 0.8), (true, 6.0)]);
        let segs = split_on_silence(&audio, SegmentConfig::default());
        assert_eq!(segs.len(), 2, "{segs:?}");
        let cut = secs(segs[0].end);
        assert!((17.0..17.8).contains(&cut), "coupe à {cut} s");
        assert_eq!(segs[1].start, segs[0].end);
        assert_eq!(segs[1].overlap, 0);
        assert_eq!(segs[1].end, audio.len());
    }

    #[test]
    fn pause_trop_courte_ignoree() {
        // Pauses de 300 ms : aucune coupe sur silence, coupe forcée avant 28 s.
        let mut parts = Vec::new();
        for _ in 0..12 {
            parts.extend([(true, 2.7), (false, 0.3)]);
        }
        let audio = signal(&parts);
        let segs = split_on_silence(&audio, SegmentConfig::default());
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert!(secs(segs[0].end) <= 28.0 && secs(segs[0].end) >= 24.0);
        // Pause de 300 ms (≥ 150 ms) dans la zone de recherche : coupe dedans, sans recouvrement.
        assert_eq!(segs[1].overlap, 0);
        assert_eq!(segs[1].start, segs[0].end);
    }

    #[test]
    fn coupe_forcee_avec_recouvrement() {
        // 60 s de parole continue : coupes forcées avec 1 s de recouvrement, segments ≤ 28 s.
        let audio = signal(&[(true, 60.0)]);
        let segs = split_on_silence(&audio, SegmentConfig::default());
        assert_eq!(segs.len(), 3, "{segs:?}");
        for w in segs.windows(2) {
            assert_eq!(w[1].overlap, S);
            assert_eq!(w[1].start, w[0].end - S);
        }
        assert!(segs.iter().all(|s| s.len() <= 28 * S));
        assert_eq!(segs.last().unwrap().end, audio.len());
    }

    #[test]
    fn processeur_coupe_seulement_a_28_s() {
        // 26 s avec des pauses : jamais coupée sur processeur, coupée sur GPU.
        let parts = [(true, 9.0), (false, 0.8), (true, 8.0), (false, 0.8), (true, 7.4)];
        let audio = signal(&parts);
        assert_eq!(split_on_silence(&audio, SegmentConfig::cpu()).len(), 1);
        assert_eq!(split_on_silence(&audio, SegmentConfig::default()).len(), 2);
        // 40 s : une coupe, dans la pause de 22 s (la plus longue des 8 dernières secondes).
        let audio = signal(&[(true, 22.0), (false, 0.8), (true, 4.0), (false, 0.2), (true, 13.0)]);
        let segs = split_on_silence(&audio, SegmentConfig::cpu());
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert!((22.0..22.8).contains(&secs(segs[0].end)), "coupe à {} s", secs(segs[0].end));
        assert_eq!(segs[1].overlap, 0);
    }

    #[test]
    fn segments_sans_parole_marques() {
        let audio = signal(&[(true, 16.0), (false, 20.0)]);
        let segs = split_on_silence(&audio, SegmentConfig::default());
        assert_eq!(segs.len(), 2);
        assert!(segs[0].speech && !segs[1].speech);
    }

    #[test]
    fn decoupage_en_continu_identique() {
        // Pousser l'audio par blocs de 10 ms ou d'un coup donne les mêmes segments.
        let audio = signal(&[(true, 9.0), (false, 0.6), (true, 9.0), (false, 0.7), (true, 30.0), (false, 1.0)]);
        let whole = split_on_silence(&audio, SegmentConfig::default());
        let mut seg = Segmenter::new(SegmentConfig::default());
        let mut parts = Vec::new();
        for block in audio.chunks(160) {
            parts.extend(seg.push(block));
        }
        parts.extend(seg.finish(audio.len()));
        assert_eq!(parts, whole);
        assert!(whole.len() >= 3);
    }

    #[test]
    fn jointure_simple() {
        assert_eq!(join("", "Bonjour Maître.", false), "Bonjour Maître.");
        assert_eq!(join("Bonjour Maître.", "Je vous écris.", false), "Bonjour Maître. Je vous écris.");
        // Phrase qui continue : point de Whisper retiré, mot-outil en minuscule.
        assert_eq!(join("Je vous écris au sujet du dossier", "Et de la procédure.", false), "Je vous écris au sujet du dossier et de la procédure.");
        assert_eq!(join("Je vous écris au sujet.", "du dossier Durand.", false), "Je vous écris au sujet du dossier Durand.");
        // Nom propre conservé.
        assert_eq!(join("Je vous écris au sujet de", "Durand.", false), "Je vous écris au sujet de Durand.");
    }

    #[test]
    fn dedoublonnage_a_la_jointure() {
        // Recouvrement exact.
        assert_eq!(
            join("Nous contestons le montant réclamé par", "réclamé par la partie adverse.", true),
            "Nous contestons le montant réclamé par la partie adverse."
        );
        // Mot tronqué à la fin du précédent, puis au début du suivant.
        assert_eq!(
            join("Le tribunal a rendu sa déci", "rendu sa décision le 3 octobre.", true),
            "Le tribunal a rendu sa décision le 3 octobre."
        );
        assert_eq!(
            join("Le tribunal a rendu sa décision le 3 oct.", "sion le 3 octobre dernier.", true),
            "Le tribunal a rendu sa décision le 3 octobre dernier."
        );
        assert_eq!(
            join("Le tribunal a rendu sa décision le", "Ision. Le 3 octobre dernier.", true),
            "Le tribunal a rendu sa décision le 3 octobre dernier."
        );
        // Ponctuation et casse ignorées pour comparer ; version du segment suivant gardée.
        assert_eq!(join("Il a signé le contrat.", "Le contrat, puis il est parti.", true), "Il a signé le contrat, puis il est parti.");
        // Sans recouvrement, rien n'est retiré (« nous nous » est du français).
        assert_eq!(join("Nous", "nous sommes vus.", false), "Nous nous sommes vus.");
        // Aucun mot commun : simple concaténation.
        assert_eq!(join("Bonjour Maître", "je vous écris", true), "Bonjour Maître je vous écris");
    }

    #[test]
    fn ponctuation_dictee_a_cheval_sur_une_coupe() {
        use crate::punctuation::apply_fr;
        // « point » puis « à la ligne » après la pause qui a coupé.
        let t = join("Maître virgule, je vous écris au sujet du dossier Durand. Point.", "À la ligne, je reste à votre disposition. Point.", false);
        assert_eq!(apply_fr(&t), "Maître, je vous écris au sujet du dossier Durand.\nJe reste à votre disposition.");
        // Commande coupée en deux par une coupe forcée, retrouvée grâce au recouvrement.
        let t = join("Le client soutient, ouvrez les", "ouvrez les guillemets, qu'il n'a jamais reçu la mise en demeure, fermez les guillemets. Point.", true);
        assert_eq!(apply_fr(&t), "Le client soutient « qu'il n'a jamais reçu la mise en demeure ».");
        // Commande coupée sans recouvrement (pause au milieu) : les deux moitiés se rejoignent.
        let t = join("Objet deux", "points affaire Durand contre Martin point", false);
        assert_eq!(apply_fr(&t), "Objet : affaire Durand contre Martin.");
    }

    #[test]
    fn fin_du_texte_pour_le_prompt() {
        assert_eq!(prompt_tail("un deux trois quatre", 2), "trois quatre");
        assert_eq!(prompt_tail("  un  deux ", 5), "un  deux");
        assert_eq!(prompt_tail("", 3), "");
        let mut t = Transcript::default();
        let base = TranscribeOptions { language: Some("fr".into()), initial_prompt: Some("Durand, Batimex.".into()) };
        assert_eq!(t.options(&base).initial_prompt.as_deref(), Some("Durand, Batimex."));
        t.push("Je vous écris au sujet du dossier.", false);
        assert_eq!(t.options(&base).initial_prompt.as_deref(), Some("Durand, Batimex. Je vous écris au sujet du dossier."));
        let none = TranscribeOptions { language: None, initial_prompt: None };
        assert_eq!(t.options(&none).initial_prompt.as_deref(), Some("Je vous écris au sujet du dossier."));
    }

    #[test]
    fn transcription_de_segments_dans_l_ordre() {
        let audio = signal(&[(true, 16.0), (false, 1.0), (true, 5.0), (false, 10.0)]);
        let segs = split_on_silence(&audio, SegmentConfig::default());
        let mut prompts = Vec::new();
        let texts = ["Premier segment.", "Second segment."];
        let mut n = 0;
        let text = transcribe_segments(&audio, &segs, &TranscribeOptions::default(), |_, o| {
            prompts.push(o.initial_prompt.clone());
            n += 1;
            Ok(texts[n - 1].to_string())
        })
        .unwrap();
        assert_eq!(text, "Premier segment. Second segment.");
        assert_eq!(prompts, vec![None, Some("Premier segment.".to_string())]);
    }

    /// Appels reçus par le faux moteur : longueur de l'audio, prompt.
    type Calls = Arc<Mutex<Vec<(usize, Option<String>)>>>;

    fn fake(texts: Vec<&'static str>) -> (Box<SegmentFn>, Calls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let mut n = 0;
        let f = move |s: &[f32], o: &TranscribeOptions, _: &Arc<AtomicBool>| {
            seen.lock().push((s.len(), o.initial_prompt.clone()));
            n += 1;
            Ok(texts[(n - 1).min(texts.len() - 1)].to_string().into())
        };
        (Box::new(f), calls)
    }

    #[test]
    fn session_au_fil_de_l_enregistrement() {
        let audio = signal(&[(true, 16.0), (false, 1.0), (true, 30.0), (false, 0.5)]);
        let (f, calls) = fake(vec!["Bonjour Maître.", "Je vous écris au sujet", "au sujet du dossier."]);
        let base = TranscribeOptions { language: Some("fr".into()), initial_prompt: Some("Vocabulaire.".into()) };
        let session = ChunkedSession::start(SegmentConfig::default(), base, Arc::default(), f).unwrap();
        let mut sink = session.sink();
        // Le thread audio ne transmet pas les derniers échantillons (vidange du rééchantillonneur).
        let fed = audio.len() - 800;
        for block in audio[..fed].chunks(480) {
            sink(block);
        }
        assert_eq!(session.segments_sent(), 2, "deux segments partis pendant l'enregistrement");
        let text = session.finish(&audio).unwrap().text;
        assert_eq!(text, "Bonjour Maître. Je vous écris au sujet du dossier.");
        let calls = calls.lock();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].1.as_deref(), Some("Vocabulaire."));
        assert_eq!(calls[1].1.as_deref(), Some("Vocabulaire. Bonjour Maître."));
        assert_eq!(calls.iter().map(|c| c.0).sum::<usize>(), audio.len() + S, "tout l'audio, plus 1 s de recouvrement");
    }

    #[test]
    fn session_courte_un_seul_appel() {
        let audio = signal(&[(true, 6.0)]);
        let (f, calls) = fake(vec!["Texte."]);
        let session = ChunkedSession::start(SegmentConfig::default(), TranscribeOptions::default(), Arc::default(), f).unwrap();
        session.sink()(&audio);
        assert_eq!(session.finish(&audio).unwrap().text, "Texte.");
        assert_eq!(calls.lock().clone(), vec![(audio.len(), None)]);
    }

    #[test]
    fn session_annulee() {
        let audio = signal(&[(true, 16.0), (false, 1.0), (true, 2.0)]);
        let started = Arc::new(AtomicBool::new(false));
        let aborted = Arc::new(AtomicBool::new(false));
        let (s2, a2) = (started.clone(), aborted.clone());
        let f = move |_: &[f32], _: &TranscribeOptions, cancel: &Arc<AtomicBool>| {
            s2.store(true, Ordering::SeqCst);
            // Transcription longue qui surveille le drapeau, comme le rappel d'abandon de whisper.cpp.
            for _ in 0..500 {
                if cancel.load(Ordering::Relaxed) {
                    a2.store(true, Ordering::SeqCst);
                    return Err("annulée".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok("trop tard".to_string().into())
        };
        let session = ChunkedSession::start(SegmentConfig::default(), TranscribeOptions::default(), Arc::default(), Box::new(f)).unwrap();
        session.sink()(&audio);
        assert_eq!(session.segments_sent(), 1);
        while !started.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        let flag = session.cancel_flag();
        drop(session);
        assert!(flag.load(Ordering::SeqCst));
        let t = std::time::Instant::now();
        while !aborted.load(Ordering::SeqCst) {
            assert!(t.elapsed() < std::time::Duration::from_secs(2), "segment en cours non annulé");
            std::thread::yield_now();
        }
    }
}

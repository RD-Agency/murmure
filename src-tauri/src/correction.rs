// SPDX-License-Identifier: AGPL-3.0-or-later
//! Corrections du texte dicté.
//!
//! - **Automatiques, limitées aux mots douteux** : après la transcription locale, un mot dont la
//!   probabilité Whisper est faible (< [`LOW_CONFIDENCE`]) et qui ressemble à un terme du
//!   vocabulaire (distance de Levenshtein : 1 édition dès 5 caractères, 2 dès 8) est remplacé par ce
//!   terme. Un mot bien reconnu n'est jamais touché.
//! - **Règles de remplacement** (réglages) : expression exacte remplacée après la ponctuation.
//! - **Corrections manuelles** : diff mot à mot entre le texte avant et après modification, pour en
//!   extraire les termes corrigés.

use serde::{Deserialize, Serialize};

/// Seuil de confiance (probabilité du token Whisper le plus faible du mot) sous lequel un mot est
/// « douteux ». Calibré sur le banc d'essai (corpus juridique et FLEURS) : voir ARCHITECTURE.md.
pub const LOW_CONFIDENCE: f32 = 0.6;

/// Termes du vocabulaire au-delà de cette longueur (en mots) : phrases de style, ignorées.
const MAX_TERM_WORDS: usize = 3;
/// Au-delà, un remplacement n'est plus une correction de terme mais une réécriture.
const MAX_CHANGE_WORDS: usize = 6;

/// Mot reconnu par Whisper et sa confiance (minimum des probabilités de ses tokens).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WordConfidence {
    pub word: String,
    pub probability: f32,
}

/// Correction appliquée automatiquement, conservée dans l'historique.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoCorrection {
    pub from: String,
    pub to: String,
    pub probability: f32,
}

/// Règle de remplacement : `from` (insensible à la casse, mots entiers) devient `to`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

/// Paire avant/après extraite d'une correction manuelle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermChange {
    pub wrong: String,
    pub right: String,
}

// ─────────────────────────────── Outils ───────────────────────────────

/// Distance de Levenshtein en caractères.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Éditions tolérées selon la longueur (en caractères) du plus court des deux mots.
pub fn max_edits(len: usize) -> usize {
    match len {
        0..=4 => 0,
        5..=7 => 1,
        _ => 2,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Découpe un mot en (ponctuation de tête, cœur, ponctuation de queue). Apostrophes et traits
/// d'union internes restent dans le cœur ; le point final d'une abréviation (« L. ») est de la
/// ponctuation ici, ce qui est sans effet sur les comparaisons.
fn split_punct(token: &str) -> (&str, &str, &str) {
    let start = token.find(is_word_char).unwrap_or(token.len());
    let end = token.rfind(is_word_char).map(|i| i + token[i..].chars().next().map_or(1, char::len_utf8)).unwrap_or(start);
    (&token[..start], &token[start..end.max(start)], &token[end.max(start)..])
}

/// Élision française en tête de mot (« l' », « d' », « qu' »…) : renvoie (préfixe, reste).
fn split_elision(core: &str) -> (&str, &str) {
    const PREFIXES: &[&str] = &["jusqu", "lorsqu", "puisqu", "quoiqu", "qu", "l", "d", "j", "m", "n", "s", "t", "c"];
    let lower = core.to_lowercase();
    for p in PREFIXES {
        for apo in ['\'', '’'] {
            let pat = format!("{p}{apo}");
            if lower.starts_with(&pat) && core.len() > pat.len() && lower.len() == core.len() {
                return core.split_at(pat.len());
            }
        }
    }
    ("", core)
}

fn normalize(s: &str) -> String {
    s.to_lowercase().replace('’', "'")
}

// ───────────────────── Corrections automatiques ─────────────────────

/// Terme du vocabulaire : texte tel qu'écrit, et forme normalisée de chaque mot.
#[derive(Clone, Debug)]
struct Term {
    text: String,
    norm: String,
    words: usize,
}

/// Dictionnaire de correction : vocabulaire (prompt initial) et cibles des règles de remplacement.
#[derive(Clone, Debug, Default)]
pub struct Dictionary {
    terms: Vec<Term>,
}

impl Dictionary {
    /// Termes du vocabulaire (séparés par virgules, points-virgules ou retours à la ligne) et cibles
    /// des règles. Les éléments de plus de trois mots (phrases d'exemple) sont ignorés.
    pub fn new(vocabulary: &str, replacements: &[Replacement]) -> Self {
        let mut terms: Vec<Term> = Vec::new();
        let candidates = vocabulary
            .split([',', ';', '\n'])
            .map(str::to_string)
            .chain(replacements.iter().map(|r| r.to.clone()));
        for raw in candidates {
            let text = raw.trim().trim_matches(|c: char| !is_word_char(c)).trim();
            let text = text.strip_prefix("Ex. :").unwrap_or(text).trim();
            let words = text.split_whitespace().count();
            if words == 0 || words > MAX_TERM_WORDS || text.chars().filter(|c| is_word_char(*c)).count() < 5 {
                continue;
            }
            let norm = normalize(&text.split_whitespace().collect::<Vec<_>>().join(" "));
            if !terms.iter().any(|t| t.norm == norm) {
                terms.push(Term { text: text.to_string(), norm, words });
            }
        }
        Self { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn len(&self) -> usize {
        self.terms.len()
    }
}

/// Confiance de chaque mot du texte, par alignement séquentiel sur les mots de Whisper.
/// Un mot non retrouvé (ponctuation dictée convertie, nettoyage) reçoit `None` : jamais corrigé.
fn align_confidence(tokens: &[&str], words: &[WordConfidence]) -> Vec<Option<f32>> {
    const LOOKAHEAD: usize = 6;
    let norm_words: Vec<String> = words.iter().map(|w| normalize(split_punct(w.word.trim()).1)).collect();
    let mut cursor = 0;
    tokens
        .iter()
        .map(|t| {
            let core = normalize(split_punct(t).1);
            if core.is_empty() {
                return None;
            }
            let end = (cursor + LOOKAHEAD).min(words.len());
            let found = (cursor..end).find(|&i| norm_words[i] == core)?;
            cursor = found + 1;
            Some(words[found].probability)
        })
        .collect()
}

/// Applique le dictionnaire aux seuls mots douteux. Renvoie le texte et les corrections faites.
pub fn correct_low_confidence(
    text: &str,
    words: &[WordConfidence],
    dict: &Dictionary,
    threshold: f32,
) -> (String, Vec<AutoCorrection>) {
    if dict.is_empty() || words.is_empty() {
        return (text.to_string(), Vec::new());
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let conf = align_confidence(&tokens, words);
    let max_n = dict.terms.iter().map(|t| t.words).max().unwrap_or(1);
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut fixes = Vec::new();
    let mut i = 0;
    'outer: while i < tokens.len() {
        for n in (1..=max_n.min(tokens.len() - i)).rev() {
            let window = &tokens[i..i + n];
            let window_conf = &conf[i..i + n];
            // Tous les mots alignés, au moins un douteux.
            if window_conf.iter().any(Option::is_none) {
                continue;
            }
            let min_p = window_conf.iter().flatten().copied().fold(f32::INFINITY, f32::min);
            if min_p >= threshold {
                continue;
            }
            // Ponctuation interne (fin de phrase au milieu de la fenêtre) : pas un terme.
            if n > 1 && window[..n - 1].iter().any(|t| !split_punct(t).2.is_empty()) {
                continue;
            }
            let (lead, first_core, _) = split_punct(window[0]);
            let (elision, first_core) = split_elision(first_core);
            let trail = split_punct(window[n - 1]).2;
            let mut cores: Vec<&str> = window.iter().map(|t| split_punct(t).1).collect();
            cores[0] = first_core;
            let candidate = normalize(&cores.join(" "));
            if candidate.is_empty() || candidate.chars().any(|c| c.is_ascii_digit()) {
                continue;
            }
            let mut best: Option<(&Term, usize)> = None;
            let mut tie = false;
            for term in dict.terms.iter().filter(|t| t.words == n) {
                let len = candidate.chars().count().min(term.norm.chars().count());
                let d = levenshtein(&candidate, &term.norm);
                if d == 0 {
                    // Déjà correct : on n'y touche pas.
                    best = None;
                    tie = false;
                    break;
                }
                if d > max_edits(len) {
                    continue;
                }
                match best {
                    Some((_, bd)) if d > bd => {}
                    Some((_, bd)) if d == bd => tie = true,
                    _ => {
                        best = Some((term, d));
                        tie = false;
                    }
                }
            }
            let Some((term, _)) = best.filter(|_| !tie) else { continue };
            let mut replacement = term.text.clone();
            // Majuscule de début de phrase conservée pour un terme écrit en minuscules.
            let starts_upper = first_core.chars().next().is_some_and(char::is_uppercase);
            if starts_upper && elision.is_empty() && replacement.chars().next().is_some_and(char::is_lowercase) {
                let mut c = replacement.chars();
                replacement = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
            }
            let original = window.join(" ");
            let fixed = format!("{lead}{elision}{replacement}{trail}");
            fixes.push(AutoCorrection { from: original, to: fixed.clone(), probability: min_p });
            out.push(fixed);
            i += n;
            continue 'outer;
        }
        out.push(tokens[i].to_string());
        i += 1;
    }
    (out.join(" "), fixes)
}

/// Texte d'une dictée après traitement.
#[derive(Clone, Debug, Default)]
pub struct Processed {
    /// Sortie du modèle (nettoyée des hallucinations), avant corrections et mise en forme.
    pub raw_text: String,
    /// Texte collé.
    pub text: String,
    /// Mot de validation prononcé : Entrée après le collage.
    pub submit: bool,
    pub auto_corrections: Vec<AutoCorrection>,
}

/// Chaîne de traitement après la transcription : corrections des mots douteux (moteur local),
/// mise en forme ([`crate::postprocess::finalize`] : mot de validation, ponctuation dictée, textes
/// courts), puis règles de remplacement.
pub fn finalize(t: &crate::engine::Transcript, settings: &crate::settings::Settings, spoken_punctuation: bool) -> Processed {
    let raw_text = crate::pipeline::clean_text(&t.text);
    let (text, auto_corrections) = if settings.auto_correct && !t.words.is_empty() {
        let dict = Dictionary::new(&settings.initial_prompt, &settings.replacements);
        correct_low_confidence(&raw_text, &t.words, &dict, LOW_CONFIDENCE)
    } else {
        (raw_text.clone(), Vec::new())
    };
    if !auto_corrections.is_empty() {
        log::info!("corrections automatiques : {}", auto_corrections.len());
    }
    let submit_words: &[String] = if settings.submit_word_enabled { &settings.submit_words } else { &[] };
    let post = crate::postprocess::Options {
        spoken_punctuation,
        short_text_plain: settings.short_text_plain,
        submit_words,
        vocabulary: &settings.initial_prompt,
    };
    let crate::postprocess::Finalized { text, submit } = crate::postprocess::finalize(&text, &post);
    let text = apply_replacements(&text, &settings.replacements);
    Processed { raw_text, text, submit, auto_corrections }
}

// ─────────────────────── Règles de remplacement ───────────────────────

/// Remplace chaque expression `from` (mots entiers, insensible à la casse) par `to`.
pub fn apply_replacements(text: &str, rules: &[Replacement]) -> String {
    let mut text = text.to_string();
    for rule in rules {
        let from = rule.from.trim();
        if from.is_empty() {
            continue;
        }
        let Ok(re) = regex::RegexBuilder::new(&regex::escape(from)).case_insensitive(true).build() else {
            continue;
        };
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for m in re.find_iter(&text) {
            let before_ok = !text[..m.start()].chars().next_back().is_some_and(is_word_char)
                || !from.chars().next().is_some_and(is_word_char);
            let after_ok = !text[m.end()..].chars().next().is_some_and(is_word_char)
                || !from.chars().next_back().is_some_and(is_word_char);
            if before_ok && after_ok {
                out.push_str(&text[last..m.start()]);
                out.push_str(rule.to.trim());
                last = m.end();
            }
        }
        out.push_str(&text[last..]);
        text = out;
    }
    text
}

// ───────────────────────── Corrections manuelles ─────────────────────────

/// Termes corrigés entre deux versions d'un texte (diff mot à mot, plus longue sous-suite commune).
/// Seuls les remplacements courts (au plus six mots de chaque côté) sont retenus ; la ponctuation
/// commune aux deux côtés est retirée (« affirmation. » → « infirmation. » donne le couple sans point).
pub fn word_diff(before: &str, after: &str) -> Vec<TermChange> {
    let a: Vec<&str> = before.split_whitespace().collect();
    let b: Vec<&str> = after.split_whitespace().collect();
    // Préfixe et suffixe communs : le cas courant (une correction locale) reste linéaire.
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    // Texte entièrement réécrit (milieu très long) : pas de diff quadratique.
    if a.len().saturating_mul(b.len()) > 4_000_000 {
        return Vec::new();
    }
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let mut changes = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut del, mut ins): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    let mut flush = |del: &mut Vec<&str>, ins: &mut Vec<&str>| {
        if !del.is_empty() && !ins.is_empty() && del.len() <= MAX_CHANGE_WORDS && ins.len() <= MAX_CHANGE_WORDS {
            let (w, r) = trim_common_punct(&del.join(" "), &ins.join(" "));
            if !w.is_empty() && !r.is_empty() && w != r {
                changes.push(TermChange { wrong: w, right: r });
            }
        }
        del.clear();
        ins.clear();
    };
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            flush(&mut del, &mut ins);
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            ins.push(b[j]);
            j += 1;
        } else {
            del.push(a[i]);
            i += 1;
        }
    }
    flush(&mut del, &mut ins);
    changes
}

/// Retire la ponctuation de tête et de queue identique des deux côtés, et l'élision commune
/// (« l'affirmation » → « l'infirmation » donne « affirmation » → « infirmation »).
fn trim_common_punct(wrong: &str, right: &str) -> (String, String) {
    let (wl, wc, wt) = split_punct(wrong);
    let (rl, rc, rt) = split_punct(right);
    let (we, wrest) = split_elision(wc);
    let (re, rrest) = split_elision(rc);
    let (wc, rc) = if !we.is_empty() && normalize(we) == normalize(re) { (wrest, rrest) } else { (wc, rc) };
    let w = format!("{}{wc}{}", if wl == rl { "" } else { wl }, if wt == rt { "" } else { wt });
    let r = format!("{}{rc}{}", if wl == rl { "" } else { rl }, if wt == rt { "" } else { rt });
    (w, r)
}

/// Petits mots grammaticaux : jamais proposés pour le vocabulaire.
const STOPWORDS: &[&str] = &[
    "le", "la", "les", "un", "une", "des", "du", "de", "et", "ou", "en", "au", "aux", "à", "a", "il", "elle", "ils",
    "elles", "on", "nous", "vous", "je", "tu", "ce", "cet", "cette", "ces", "que", "qui", "quoi", "ne", "pas", "par",
    "pour", "sur", "dans", "avec", "sans", "est", "sont", "mais", "donc", "car", "ni", "se", "sa", "son", "ses",
    "leur", "leurs", "mon", "ma", "mes", "ton", "ta", "tes", "notre", "votre", "nos", "vos", "y", "été", "être",
    "avoir", "plus", "moins", "très", "bien",
];

/// Un terme corrigé mérite-t-il d'être proposé pour le vocabulaire ?
pub fn is_vocabulary_candidate(term: &str) -> bool {
    let t = term.trim();
    let letters = t.chars().filter(|c| c.is_alphanumeric()).count();
    if letters < 3 || t.split_whitespace().count() > MAX_TERM_WORDS {
        return false;
    }
    !STOPWORDS.contains(&normalize(t).as_str())
}

/// Le vocabulaire contient-il déjà ce terme (comparaison insensible à la casse) ?
pub fn vocabulary_contains(vocabulary: &str, term: &str) -> bool {
    let t = normalize(term.trim());
    vocabulary.split([',', ';', '\n']).any(|v| normalize(v.trim().trim_end_matches('.')) == t)
}

/// Ajoute un terme au vocabulaire (séparé par une virgule), sans doublon.
pub fn append_vocabulary(vocabulary: &str, term: &str) -> String {
    let term = term.trim();
    if term.is_empty() || vocabulary_contains(vocabulary, term) {
        return vocabulary.to_string();
    }
    let v = vocabulary.trim_end();
    if v.is_empty() {
        term.to_string()
    } else if v.ends_with([',', ';']) {
        format!("{v} {term}")
    } else {
        format!("{v}, {term}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(spec: &[(&str, f32)]) -> Vec<WordConfidence> {
        spec.iter().map(|(w, p)| WordConfidence { word: w.to_string(), probability: *p }).collect()
    }

    #[test]
    fn levenshtein_et_seuils() {
        assert_eq!(levenshtein("affirmation", "infirmation"), 2);
        assert_eq!(levenshtein("Lefèvre", "Lefebvre"), 2);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("dépens", "dépends"), 1);
        assert_eq!([4, 5, 7, 8, 12].map(max_edits), [0, 1, 1, 2, 2]);
    }

    #[test]
    fn dictionnaire_depuis_le_vocabulaire() {
        let d = Dictionary::new(
            "Ex. : Murmure, Kubernetes, Mme Lefèvre, RGPD, infirmation\nUne phrase de style bien trop longue pour un terme.",
            &[Replacement { from: "l 12 32 2".into(), to: "L. 1232-2".into() }],
        );
        let names: Vec<_> = d.terms.iter().map(|t| t.text.as_str()).collect();
        // « RGPD » (4 caractères) est trop court pour une correction approximative.
        assert_eq!(names, ["Murmure", "Kubernetes", "Mme Lefèvre", "infirmation", "L. 1232-2"]);
    }

    #[test]
    fn corrige_seulement_les_mots_douteux() {
        let dict = Dictionary::new("infirmation, Batimex", &[]);
        let text = "Nous sollicitons l'affirmation du jugement. Une affirmation claire.";
        let w = words(&[
            ("Nous", 0.98),
            ("sollicitons", 0.95),
            ("l'affirmation", 0.31),
            ("du", 0.99),
            ("jugement.", 0.97),
            ("Une", 0.99),
            ("affirmation", 0.92),
            ("claire.", 0.96),
        ]);
        let (out, fixes) = correct_low_confidence(text, &w, &dict, LOW_CONFIDENCE);
        // Le premier mot (douteux) est corrigé, élision conservée ; le second (sûr) reste intact.
        assert_eq!(out, "Nous sollicitons l'infirmation du jugement. Une affirmation claire.");
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].from, "l'affirmation");
        assert_eq!(fixes[0].to, "l'infirmation");
    }

    #[test]
    fn respecte_les_seuils_de_longueur_et_l_ambiguite() {
        // « Batimex » / « Batiment » : 2 éditions mais 7 caractères → 1 seule tolérée.
        let dict = Dictionary::new("Batimex", &[]);
        let (out, _) = correct_low_confidence("la société Batiment", &words(&[("la", 0.9), ("société", 0.9), ("Batiment", 0.1)]), &dict, 0.45);
        assert_eq!(out, "la société Batiment");
        // « Batimax » : 1 édition → corrigé.
        let (out, _) = correct_low_confidence("la société Batimax.", &words(&[("la", 0.9), ("société", 0.9), ("Batimax.", 0.2)]), &dict, 0.45);
        assert_eq!(out, "la société Batimex.");
        // Deux termes à égale distance : aucune correction.
        let dict = Dictionary::new("Dupont, Dupond", &[]);
        let (out, fixes) = correct_low_confidence("M. Dupons", &words(&[("M.", 0.9), ("Dupons", 0.1)]), &dict, 0.45);
        assert_eq!(out, "M. Dupons");
        assert!(fixes.is_empty());
    }

    #[test]
    fn termes_de_plusieurs_mots_et_majuscules() {
        let dict = Dictionary::new("Mme Lefebvre, mise en état", &[]);
        let w = words(&[("Bonjour", 0.99), ("Mme", 0.9), ("Lefèvre,", 0.3), ("Mise", 0.4), ("en", 0.9), ("étang.", 0.2)]);
        let (out, fixes) = correct_low_confidence("Bonjour Mme Lefèvre, Mise en étang.", &w, &dict, 0.45);
        assert_eq!(out, "Bonjour Mme Lefebvre, Mise en état.");
        assert_eq!(fixes.len(), 2);
    }

    #[test]
    fn mots_non_alignes_jamais_touches() {
        // Ponctuation dictée convertie, mot absent des mots Whisper : pas de confiance, pas de correction.
        let dict = Dictionary::new("infirmation", &[]);
        let (out, _) = correct_low_confidence("l'affirmation", &words(&[("autre", 0.1)]), &dict, 0.45);
        assert_eq!(out, "l'affirmation");
        let (out, _) = correct_low_confidence("l'affirmation", &[], &dict, 0.45);
        assert_eq!(out, "l'affirmation");
    }

    #[test]
    fn chaine_complete_apres_transcription() {
        use crate::engine::Transcript;
        use crate::settings::Settings;
        let t = Transcript {
            text: " Nous sollicitons l'information du jugement point à la ligne Article L 12 32 2.".into(),
            words: words(&[
                ("Nous", 0.99),
                ("sollicitons", 0.97),
                ("l'information", 0.41),
                ("du", 0.99),
                ("jugement", 0.98),
                ("point", 0.9),
                ("à", 0.9),
                ("la", 0.9),
                ("ligne", 0.9),
                ("Article", 0.9),
                ("L", 0.5),
                ("12", 0.6),
                ("32", 0.6),
                ("2.", 0.6),
            ]),
        };
        let s = Settings {
            initial_prompt: "infirmation".into(),
            replacements: vec![Replacement { from: "L 12 32 2".into(), to: "L. 1232-2".into() }],
            ..Settings::default()
        };
        let p = finalize(&t, &s, true);
        assert_eq!(p.raw_text, "Nous sollicitons l'information du jugement point à la ligne Article L 12 32 2.");
        assert_eq!(p.text, "Nous sollicitons l'infirmation du jugement.\nArticle L. 1232-2.");
        assert_eq!(p.auto_corrections.len(), 1);
        // Réglage coupé : seule la règle s'applique.
        let p = finalize(&t, &Settings { auto_correct: false, ..s }, true);
        assert!(p.text.contains("l'information") && p.text.contains("L. 1232-2"));
    }

    #[test]
    fn regles_de_remplacement() {
        let rules = [
            Replacement { from: "L 1232-2".into(), to: "L. 1232-2".into() },
            Replacement { from: "chat".into(), to: "CHAT".into() },
            Replacement { from: "  ".into(), to: "x".into() },
        ];
        assert_eq!(
            apply_replacements("aux articles l 1232-2 et suivants, le chat et les chats", &rules),
            "aux articles L. 1232-2 et suivants, le CHAT et les chats"
        );
    }

    #[test]
    fn diff_des_corrections() {
        assert_eq!(
            word_diff("Nous sollicitons l'affirmation du jugement.", "Nous sollicitons l'infirmation du jugement."),
            vec![TermChange { wrong: "affirmation".into(), right: "infirmation".into() }]
        );
        assert_eq!(
            word_diff("prévue aux articles L 12 cent trente 2 et suivants", "prévue aux articles L. 1232-2 et suivants"),
            vec![TermChange { wrong: "L 12 cent trente 2".into(), right: "L. 1232-2".into() }]
        );
        // Ponctuation commune retirée ; deux corrections distinctes.
        assert_eq!(
            word_diff("Le dossier Lefèvre contre Batimax.", "Le dossier Lefebvre contre Batimex."),
            vec![
                TermChange { wrong: "Lefèvre".into(), right: "Lefebvre".into() },
                TermChange { wrong: "Batimax".into(), right: "Batimex".into() },
            ]
        );
        // Ajout ou suppression pure, réécriture complète : rien à apprendre.
        assert!(word_diff("un texte", "un texte complété").is_empty());
        assert!(word_diff("identique", "identique").is_empty());
        assert!(word_diff("a b c d e f g h", "un tout autre texte qui n'a rien à voir avec").is_empty());
    }

    #[test]
    fn candidats_au_vocabulaire() {
        assert!(is_vocabulary_candidate("infirmation"));
        assert!(is_vocabulary_candidate("L. 1232-2"));
        assert!(!is_vocabulary_candidate("les"));
        assert!(!is_vocabulary_candidate("à"));
        assert!(!is_vocabulary_candidate("une phrase de quatre mots"));
        assert!(vocabulary_contains("Murmure, Infirmation.", "infirmation"));
        assert_eq!(append_vocabulary("", "Batimex"), "Batimex");
        assert_eq!(append_vocabulary("Murmure", "Batimex"), "Murmure, Batimex");
        assert_eq!(append_vocabulary("Murmure, Batimex", "batimex"), "Murmure, Batimex");
    }

    /// Banc d'essai des corrections filtrées par la confiance (et de l'aller-retour Opus) :
    /// `MURMURE_TEST_MODEL=… MURMURE_BENCH_DIR=…/data MURMURE_BENCH_CORPORA=legal,real
    ///  MURMURE_VOCAB="…" MURMURE_BENCH_PROMPT=1 MURMURE_BENCH_OUT=out.jsonl
    ///  cargo test --release --lib correction::tests::banc_corrections -- --ignored --nocapture`
    ///
    /// Chaque ligne de sortie : transcription brute, mots et confiances, texte corrigé pour chaque
    /// seuil, et transcription de l'audio après encodage Opus (score : script Python du banc).
    #[test]
    #[ignore = "nécessite un modèle Whisper et le corpus du banc d'essai"]
    fn banc_corrections() {
        use crate::engine::{LocalEngine, TranscribeOptions};
        use std::path::{Path, PathBuf};
        let env = |k: &str| std::env::var(k).ok();
        let model = PathBuf::from(env("MURMURE_TEST_MODEL").expect("MURMURE_TEST_MODEL"));
        let base = PathBuf::from(env("MURMURE_BENCH_DIR").expect("MURMURE_BENCH_DIR"));
        let corpora = env("MURMURE_BENCH_CORPORA").unwrap_or_else(|| "legal".into());
        let vocab = env("MURMURE_VOCAB").unwrap_or_default();
        let in_prompt = env("MURMURE_BENCH_PROMPT").is_none_or(|v| v != "0");
        let with_opus = env("MURMURE_BENCH_OPUS").is_some_and(|v| v == "1");
        let out_path = env("MURMURE_BENCH_OUT").unwrap_or_else(|| "banc-corrections.jsonl".into());
        let thresholds = [0.2f32, 0.3, 0.4, 0.45, 0.5, 0.6, 0.7, 0.8, 1.01];
        let read_wav = |p: &Path| -> Vec<f32> {
            let r = hound::WavReader::open(p).unwrap();
            assert_eq!(r.spec().sample_rate, 16_000);
            r.into_samples::<i16>().map(|s| f32::from(s.unwrap()) / 32768.0).collect()
        };
        let engine = LocalEngine::load(&model).unwrap();
        let prompt = if in_prompt { format!("{vocab} {}", crate::punctuation::PROMPT_FR) } else { crate::punctuation::PROMPT_FR.to_string() };
        let opts = TranscribeOptions { language: Some("fr".into()), initial_prompt: Some(prompt.trim().to_string()) };
        let dict = Dictionary::new(&vocab, &[]);
        println!("dictionnaire : {} termes ; vocabulaire dans le prompt : {in_prompt}", dict.len());
        let mut out = String::new();
        for corpus in corpora.split(',') {
            let manifest = std::fs::read_to_string(base.join(corpus).join("manifest.jsonl")).unwrap();
            for line in manifest.lines().filter(|l| !l.trim().is_empty()) {
                let m: serde_json::Value = serde_json::from_str(line).unwrap();
                let wav = m["wav"].as_str().unwrap();
                let samples = read_wav(&base.join(corpus).join(wav));
                let t0 = std::time::Instant::now();
                let t = engine.transcribe_detailed(&samples, &opts).unwrap();
                let ms = t0.elapsed().as_millis();
                let raw = crate::pipeline::clean_text(&t.text);
                let mut corrected = serde_json::Map::new();
                let mut fixes = serde_json::Map::new();
                for th in thresholds {
                    let (text, f) = correct_low_confidence(&raw, &t.words, &dict, th);
                    corrected.insert(format!("{th}"), text.into());
                    fixes.insert(format!("{th}"), serde_json::to_value(&f).unwrap());
                }
                let opus = with_opus.then(|| {
                    let ogg = crate::history::audio::encode_ogg_opus(&samples).unwrap();
                    let back = crate::history::audio::decode_ogg_opus(&ogg).unwrap();
                    let o = engine.transcribe_detailed(&back, &opts).unwrap();
                    serde_json::json!({ "bytes": ogg.len(), "raw": crate::pipeline::clean_text(&o.text) })
                });
                let doubtful: Vec<String> =
                    t.words.iter().filter(|w| w.probability < 0.6).map(|w| format!("{} {:.2}", w.word, w.probability)).collect();
                println!("{corpus}/{wav} ({ms} ms) : {raw}\n  douteux : {}", doubtful.join(" | "));
                let row = serde_json::json!({
                    "corpus": corpus, "wav": wav, "ref": m["ref"], "dur": m["dur"], "ms": ms, "raw": raw,
                    "words": t.words, "corrected": corrected, "fixes": fixes, "opus": opus,
                });
                out.push_str(&row.to_string());
                out.push('\n');
            }
        }
        std::fs::write(&out_path, out).unwrap();
        println!("résultats : {out_path}");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mise en forme finale du texte dicté, après Whisper et avant le collage.
//!
//! Ordre : espaces normalisés, mot de validation retiré, ponctuation dictée, puis règle des textes
//! courts. Tout est pur et testé ; le pipeline ne fait qu'appeler [`finalize`].
//!
//! - **Textes courts** : Whisper rédige toujours une phrase (« Oui. », « Merci. »). Pour trois mots
//!   ou moins, Murmure retire le point final qu'il a ajouté et la majuscule qu'il a imposée en tête.
//!   Un point dicté (« merci point ») reste, une majuscule propre à un sigle (« RGPD ») ou à un mot du
//!   vocabulaire (« Kubernetes ») aussi.
//! - **Mot de validation** : « … à demain. Envoyer. » donne « … à demain. » puis Entrée. Le mot n'est
//!   reconnu qu'en toute fin, sans tenir compte de la casse, des accents, de la ponctuation qui
//!   l'entoure, et avec une lettre d'écart pour les mots d'au moins cinq lettres (« terminer »,
//!   « envoyez »).

/// Au-delà de ce nombre de mots, le texte est laissé tel que Whisper l'a rédigé.
pub const SHORT_TEXT_MAX_WORDS: usize = 3;

/// Réglages utiles à la mise en forme.
#[derive(Clone, Debug, Default)]
pub struct Options<'a> {
    /// Ponctuation dictée (« virgule », « point »…), français.
    pub spoken_punctuation: bool,
    /// Règle des textes courts.
    pub short_text_plain: bool,
    /// Mots de validation actifs (vide = fonction désactivée).
    pub submit_words: &'a [String],
    /// Vocabulaire de l'utilisateur : ses majuscules sont respectées.
    pub vocabulary: &'a str,
}

/// Texte prêt à coller, et faut-il appuyer sur Entrée ensuite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finalized {
    pub text: String,
    pub submit: bool,
}

/// Mise en forme complète d'une transcription brute.
pub fn finalize(raw: &str, opts: &Options) -> Finalized {
    let cleaned = crate::pipeline::clean_text(raw);
    let (text, submit) = match strip_submit_word(&cleaned, opts.submit_words) {
        Some(rest) => (rest, true),
        None => (cleaned, false),
    };
    let punctuate = |t: &str| {
        if opts.spoken_punctuation {
            crate::punctuation::apply_fr(t)
        } else {
            t.to_string()
        }
    };
    let mut out = punctuate(&text);
    if opts.short_text_plain && count_words(&out) <= SHORT_TEXT_MAX_WORDS {
        // La règle s'applique au texte de Whisper, avant la ponctuation dictée : un point prononcé
        // (« merci point ») est donc conservé.
        out = punctuate(&relax_short(&text, opts.vocabulary));
    }
    Finalized { text: out, submit }
}

/// Mots : jetons qui contiennent au moins une lettre ou un chiffre.
pub fn count_words(text: &str) -> usize {
    text.split_whitespace().filter(|t| t.chars().any(char::is_alphanumeric)).count()
}

/// Retire le point final et la majuscule de tête ajoutés par Whisper.
fn relax_short(text: &str, vocabulary: &str) -> String {
    let mut s = text.trim_end().to_string();
    // Un seul point : « … » ou « ... » sont voulus.
    if s.ends_with('.') && !s.ends_with("..") {
        s.pop();
        s.truncate(s.trim_end().len());
    }
    let first = s.split_whitespace().next().unwrap_or("");
    if should_lowercase(first, vocabulary) {
        let mut chars = s.chars();
        if let Some(c) = chars.next() {
            s = c.to_lowercase().collect::<String>() + chars.as_str();
        }
    }
    s
}

/// Majuscule de tête à retirer ? Non pour un sigle (« RGPD »), une casse mixte (« iPhone »,
/// « McDonald »), une lettre seule, ou un mot écrit ainsi dans le vocabulaire.
fn should_lowercase(word: &str, vocabulary: &str) -> bool {
    let core = word.trim_matches(|c: char| !c.is_alphanumeric());
    let mut letters = core.chars().filter(|c| c.is_alphabetic());
    let Some(first) = letters.next() else { return false };
    if !first.is_uppercase() || !core.starts_with(first) {
        return false;
    }
    let rest: Vec<char> = letters.collect();
    if rest.is_empty() || rest.iter().any(|c| c.is_uppercase()) {
        return false;
    }
    let in_vocabulary = vocabulary
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '.' | ':' | '!' | '?' | '(' | ')' | '"'))
        .any(|v| v == core);
    !in_vocabulary
}

/// Si `text` se termine par l'un des `words`, renvoie le texte sans ce mot (ni la ponctuation
/// faible qui le précédait : virgule, deux-points, tiret). `None` si aucun ne correspond.
pub fn strip_submit_word(text: &str, words: &[String]) -> Option<String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    // Ponctuation isolée en fin (« envoyer . ») : ignorée.
    let mut end = tokens.len();
    while end > 0 && !tokens[end - 1].chars().any(char::is_alphanumeric) {
        end -= 1;
    }
    for phrase in words {
        let wanted: Vec<String> = phrase.split_whitespace().map(normalize).filter(|w| !w.is_empty()).collect();
        if wanted.is_empty() || wanted.len() > end {
            continue;
        }
        let start = end - wanted.len();
        let matches = tokens[start..end].iter().zip(&wanted).all(|(tok, want)| close_enough(&normalize(tok), want));
        if matches {
            let rest = tokens[..start].join(" ");
            let rest = rest.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-' | '–' | '—'));
            return Some(rest.to_string());
        }
    }
    None
}

/// Minuscules, sans accents ni ponctuation autour (« Terminé ! » → « termine »).
fn normalize(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.trim_matches(|c: char| !c.is_alphanumeric()).chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' => out.push('a'),
            'ç' => out.push('c'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'î' | 'ï' | 'í' | 'ì' => out.push('i'),
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => out.push('o'),
            'ù' | 'û' | 'ü' | 'ú' => out.push('u'),
            'ÿ' => out.push('y'),
            'ñ' => out.push('n'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            '’' => out.push('\''),
            c => out.push(c),
        }
    }
    out
}

/// Égalité, ou une lettre d'écart pour les mots d'au moins cinq lettres.
fn close_enough(got: &str, wanted: &str) -> bool {
    if got == wanted {
        return true;
    }
    let len = wanted.chars().count();
    len >= 5 && got.chars().count() >= 4 && levenshtein(got, wanted) <= 1
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Vec<String> {
        crate::settings::default_submit_words()
    }

    fn opts(words: &[String]) -> Options<'_> {
        Options { spoken_punctuation: true, short_text_plain: true, submit_words: words, vocabulary: "" }
    }

    fn run(raw: &str) -> Finalized {
        finalize(raw, &opts(&[]))
    }

    #[test]
    fn textes_courts_sans_point_ni_majuscule() {
        assert_eq!(run(" Oui. ").text, "oui");
        assert_eq!(run("Merci beaucoup.").text, "merci beaucoup");
        assert_eq!(run("Rendez-vous demain matin.").text, "rendez-vous demain matin");
        assert_eq!(run("D'accord.").text, "d'accord");
    }

    #[test]
    fn textes_courts_signes_voulus_conserves() {
        assert_eq!(run("Vraiment ?").text, "vraiment ?");
        assert_eq!(run("Bravo !").text, "bravo !");
        assert_eq!(run("Bon...").text, "bon...");
        // Point dicté : la phrase reste une phrase.
        assert_eq!(run("Merci point.").text, "Merci.");
    }

    #[test]
    fn textes_courts_majuscules_legitimes() {
        assert_eq!(run("RGPD.").text, "RGPD");
        assert_eq!(run("iPhone.").text, "iPhone");
        assert_eq!(run("McDonald.").text, "McDonald");
        assert_eq!(run("A.").text, "A");
        let o = Options { vocabulary: "Kubernetes, Mme Lefèvre.", ..opts(&[]) };
        assert_eq!(finalize("Kubernetes.", &o).text, "Kubernetes");
        assert_eq!(finalize("Lefèvre.", &o).text, "Lefèvre");
        assert_eq!(finalize("Bonjour.", &o).text, "bonjour");
    }

    #[test]
    fn textes_longs_intacts() {
        let t = "Je vous rappelle demain matin.";
        assert_eq!(run(t).text, t);
        let off = Options { short_text_plain: false, ..opts(&[]) };
        assert_eq!(finalize("Oui.", &off).text, "Oui.");
    }

    #[test]
    fn compte_des_mots() {
        assert_eq!(count_words("Bonjour , ça va ?"), 3);
        assert_eq!(count_words("15 h 30"), 3);
        assert_eq!(count_words(""), 0);
    }

    #[test]
    fn mot_de_validation_en_fin() {
        let w = words();
        let r = finalize("Je vous rappelle demain matin. Envoyer.", &opts(&w));
        assert_eq!(r, Finalized { text: "Je vous rappelle demain matin.".into(), submit: true });
        let r = finalize("Je vous rappelle demain matin, terminé !", &opts(&w));
        assert_eq!(r, Finalized { text: "Je vous rappelle demain matin".into(), submit: true });
    }

    #[test]
    fn mot_de_validation_tolerant() {
        let w = words();
        for raw in ["C'est noté, Terminé.", "C'est noté. terminer", "C'est noté. TERMINE .", "C'est noté. Envoyez."] {
            let r = finalize(raw, &opts(&w));
            assert!(r.submit, "{raw}");
            // Deux mots restants : règle des textes courts.
            assert_eq!(r.text, "c'est noté", "{raw}");
        }
        // Expression de plusieurs mots.
        let multi = vec!["c'est tout".to_string()];
        assert_eq!(strip_submit_word("Bonne journée. C'est tout.", &multi).as_deref(), Some("Bonne journée."));
    }

    #[test]
    fn mot_de_validation_seulement_en_toute_fin() {
        let w = words();
        let r = finalize("Envoyer le dossier à Paul demain.", &opts(&w));
        assert!(!r.submit);
        assert_eq!(r.text, "Envoyer le dossier à Paul demain.");
        assert!(strip_submit_word("Le travail est terminé depuis hier.", &w).is_none());
        // Mots trop différents, ou trop courts pour la tolérance.
        assert!(strip_submit_word("Il faut le terminal.", &w).is_none());
        assert!(strip_submit_word("Un envoi.", &w).is_none());
        assert!(strip_submit_word("ok", &["oki".to_string()]).is_none());
        // Fonction désactivée : liste vide.
        assert!(!finalize("Bon. Envoyer.", &opts(&[])).submit);
    }

    #[test]
    fn mot_de_validation_seul() {
        let r = finalize("Envoyer.", &opts(&words()));
        assert_eq!(r, Finalized { text: String::new(), submit: true });
    }

    #[test]
    fn mot_de_validation_et_ponctuation_dictee() {
        let r = finalize("Je vous rappelle demain point. Envoyer.", &opts(&words()));
        assert_eq!(r, Finalized { text: "Je vous rappelle demain.".into(), submit: true });
    }

    #[test]
    fn normalisation_et_distance() {
        assert_eq!(normalize("« Terminé ! »"), "termine");
        assert_eq!(normalize("Œuvre"), "oeuvre");
        assert_eq!(levenshtein("termine", "terminer"), 1);
        assert_eq!(levenshtein("envoie", "envoyer"), 2);
        assert_eq!(levenshtein("", "abc"), 3);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ponctuation dictée : « virgule », « point à la ligne », « ouvrez les guillemets »… deviennent les
//! signes correspondants, comme avec une dictée de secrétariat.
//!
//! Whisper ponctue déjà seul une dictée naturelle ; quand on prononce la ponctuation, il écrit le mot
//! (« Maître virgule, je ») en l'entourant souvent de sa propre ponctuation. Chaque commande absorbe
//! donc la ponctuation et les espaces qui l'entourent, puis la typographie est recomposée.
//!
//! Garde-fous : « point » seul n'est une commande qu'en fin de phrase et hors d'un groupe nominal
//! (« caler un point jeudi », « le point de vue » restent intacts) ; « virgule » et « deux points »
//! précédés d'un déterminant restent des mots.

use once_cell::sync::Lazy;
use regex::{Captures, Regex};

/// Exemple de dictée ajouté au prompt initial de Whisper : il reconnaît alors les commandes
/// (« deux points » au lieu de « 2. », « à la ligne » au lieu de « à la Lille »). Mesuré sur un corpus :
/// la forme « exemple de dictée » ne fait pas sauter le premier mot, contrairement à une liste.
pub const PROMPT_FR: &str = "Maître, virgule, je vous écris, point, à la ligne. Objet, deux points, dossier, \
point-virgule, ouvrez les guillemets, fermez les guillemets, ouvrez la parenthèse, fermez la parenthèse, \
point d'interrogation, nouveau paragraphe.";

/// Commande reconnue → signe émis. Ordre = priorité (les plus longues d'abord).
#[derive(Clone, Copy)]
enum Mark {
    Period,
    PeriodNewline,
    Newline,
    Paragraph,
    Comma,
    Semicolon,
    Colon,
    Question,
    Exclamation,
    Ellipsis,
    OpenQuote,
    CloseQuote,
    OpenParen,
    CloseParen,
}

impl Mark {
    fn text(self) -> &'static str {
        match self {
            Mark::Period => ".",
            Mark::PeriodNewline => ".\n",
            Mark::Newline => "\n",
            Mark::Paragraph => "\n\n",
            Mark::Comma => ",",
            Mark::Semicolon => " ;",
            Mark::Colon => " :",
            Mark::Question => " ?",
            Mark::Exclamation => " !",
            Mark::Ellipsis => "…",
            Mark::OpenQuote => " « ",
            Mark::CloseQuote => " »",
            Mark::OpenParen => " (",
            Mark::CloseParen => ")",
        }
    }
}

/// Ponctuation et espaces que Whisper place autour d'un mot de commande.
const AROUND: &str = r"[\s,.;:!?…]*";

/// Déterminants / prépositions qui font d'un mot de commande un simple nom.
const NOMINAL: &str = r"(?i:le|la|les|un|une|des|du|au|aux|ce|cet|cette|ces|mon|ma|mes|ton|ta|tes|son|sa|ses|notre|nos|votre|vos|leur|leurs|chaque|quel|quelle|en|sur|de|d'|à|seule?|dernier|dernière|premier|première)";

static COMMANDS: Lazy<Vec<(Regex, Mark)>> = Lazy::new(|| {
    let cmd = |words: &str, mark: Mark| {
        let re = Regex::new(&format!(r"(?i){AROUND}\b(?:{words})\b{AROUND}")).expect("motif de ponctuation");
        (re, mark)
    };
    vec![
        cmd(r"point[,.]?\s+(?:et\s+)?à\s+la\s+ligne", Mark::PeriodNewline),
        cmd(r"nouveau\s+paragraphe|nouvel\s+alinéa|saut\s+de\s+paragraphe", Mark::Paragraph),
        cmd(r"(?:retour|passez|passer|aller)\s+à\s+la\s+ligne|à\s+la\s+ligne|nouvelle\s+ligne", Mark::Newline),
        cmd(r"points?\s+d'interrogation", Mark::Question),
        cmd(r"points?\s+d'exclamation", Mark::Exclamation),
        cmd(r"points\s+de\s+suspension", Mark::Ellipsis),
        cmd(r"point[\s,.-]+virgule", Mark::Semicolon),
        cmd(r"point\s+final", Mark::Period),
        cmd(r"(?:ouvrez|ouvrir|ouvre|ouvrons)\s+(?:les\s+)?guillemets?", Mark::OpenQuote),
        cmd(r"(?:fermez|fermer|ferme|fermons)\s+(?:les\s+)?guillemets?", Mark::CloseQuote),
        cmd(r"(?:ouvrez|ouvrir|ouvre|ouvrons)\s+(?:la\s+)?parenthèses?", Mark::OpenParen),
        cmd(r"(?:fermez|fermer|ferme|fermons)\s+(?:la\s+)?parenthèses?", Mark::CloseParen),
    ]
});

/// Commandes ambiguës : on vérifie le mot qui précède avant de remplacer.
static GUARDED: Lazy<Vec<(Regex, Mark)>> = Lazy::new(|| {
    let cmd = |words: &str, mark: Mark| {
        let re = Regex::new(&format!(r"(?i)(?P<prev>[\p{{L}}']+)?(?P<cmd>{AROUND}\b(?:{words})\b{AROUND})"))
            .expect("motif de ponctuation");
        (re, mark)
    };
    vec![cmd(r"deux[\s-]points|2\s+points", Mark::Colon), cmd(r"virgule", Mark::Comma)]
});

/// « point » seul : commande seulement s'il termine la dictée ou précède une nouvelle phrase / ligne.
static LONE_POINT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?P<prev>[\p{L}\d'»)]+)?(?P<cmd>(?P<before>[\s,.;:!?…]*)\b[Pp]oint\b(?P<after>[\s,.;:!?…]*))(?P<next>$|\n|\p{L})")
        .expect("motif de ponctuation")
});

static NOMINAL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(&format!(r"^{NOMINAL}$")).expect("motif"));

/// Mots-outils mis en majuscule par Whisper après le point qu'il a placé autour d'une commande.
static FUNCTION_WORD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^(?:Je|J'|Nous|Vous|Il|Elle|Ils|Elles|On|Le|La|Les|L'|Un|Une|Des|De|Du|D'|Que|Qu'|Qui|Et|Mais|Ou|Donc|Car|Ce|C'|Cette|Ces|En|Dans|Pour|Par|Sur|Avec|Sans|Si|Soit|Puis|Alors|Ainsi|Afin|À|Au|Aux|Est|Sont|Mes|Nos|Vos|Leur|Leurs|Son|Sa|Ses|Mon|Ma|Ton|Ta|Votre|Notre)$",
    )
    .expect("motif")
});

const SENTINEL: char = '\u{E000}';

/// Remplace la ponctuation prononcée par les signes et recompose la typographie française.
pub fn apply_fr(text: &str) -> String {
    let mut marks: Vec<Mark> = Vec::new();
    let put = |marks: &mut Vec<Mark>, m: Mark| {
        marks.push(m);
        format!(" {SENTINEL}{} ", marks.len() - 1)
    };

    let mut s = text.to_string();
    for (re, mark) in COMMANDS.iter() {
        s = re.replace_all(&s, |_: &Captures| put(&mut marks, *mark)).into_owned();
    }
    for (re, mark) in GUARDED.iter() {
        s = re
            .replace_all(&s, |c: &Captures| {
                let prev = c.name("prev").map_or("", |m| m.as_str());
                if NOMINAL_RE.is_match(prev) {
                    c[0].to_string()
                } else {
                    format!("{prev}{}", put(&mut marks, *mark))
                }
            })
            .into_owned();
    }
    let before_lone = s.clone();
    s = LONE_POINT
        .replace_all(&before_lone, |c: &Captures| {
            let prev = c.name("prev").map_or("", |m| m.as_str());
            let next = c.name("next").map_or("", |m| m.as_str());
            // Whisper isole par des virgules un mot prononcé comme une commande (« reçu, point, est-ce »).
            let start = c.get(0).map_or(0, |m| m.start());
            let after_mark = before_lone[..start].trim_end_matches(|ch: char| ch.is_ascii_digit()).ends_with(SENTINEL);
            let isolated = (after_mark || c["before"].contains([',', '.'])) && c["after"].contains([',', '.']);
            let boundary = next.is_empty() || next == "\n" || next.chars().next().is_some_and(char::is_uppercase);
            if prev.is_empty() || NOMINAL_RE.is_match(prev) || !(boundary || isolated) {
                c[0].to_string()
            } else {
                format!("{prev}{}{next}", put(&mut marks, Mark::Period))
            }
        })
        .into_owned();

    if marks.is_empty() {
        return text.to_string();
    }
    compose(&s, &marks)
}

/// Recompose le texte : signes collés au bon mot, espaces françaises, majuscules en début de phrase.
fn compose(s: &str, marks: &[Mark]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut capitalize = true;
    let mut lower_next = false;
    let mut prev_mark = false;
    for token in s.split(' ').filter(|t| !t.is_empty()) {
        if let Some(idx) = token.strip_prefix(SENTINEL) {
            let mark = marks[idx.parse::<usize>().unwrap_or(0)];
            let text = mark.text();
            while out.ends_with(' ') || (!prev_mark && out.ends_with([',', '.']) && !matches!(mark, Mark::OpenQuote | Mark::OpenParen)) {
                out.pop();
            }
            if text.starts_with(' ') && (out.is_empty() || out.ends_with('\n')) {
                out.push_str(text.trim_start());
            } else {
                out.push_str(text);
            }
            capitalize = matches!(
                mark,
                Mark::Period | Mark::PeriodNewline | Mark::Newline | Mark::Paragraph | Mark::Question | Mark::Exclamation
            ) || (capitalize && matches!(mark, Mark::OpenQuote | Mark::OpenParen));
            lower_next = matches!(mark, Mark::Comma | Mark::Semicolon | Mark::OpenQuote | Mark::OpenParen | Mark::CloseQuote | Mark::CloseParen);
            prev_mark = true;
            continue;
        }
        let word = if capitalize {
            upper_first(token)
        } else if lower_next && FUNCTION_WORD.is_match(token.trim_end_matches([',', '.', ';', ':', '!', '?'])) {
            lower_first(token)
        } else {
            token.to_string()
        };
        if !(out.is_empty() || out.ends_with([' ', '\n', '(']) || out.ends_with("« ")) {
            out.push(' ');
        }
        out.push_str(&word);
        capitalize = word.ends_with(['.', '?', '!']);
        lower_next = false;
        prev_mark = false;
    }
    tidy(&out)
}

fn upper_first(w: &str) -> String {
    let mut c = w.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
}

fn lower_first(w: &str) -> String {
    let mut c = w.chars();
    c.next().map_or_else(String::new, |f| f.to_lowercase().collect::<String>() + c.as_str())
}

static SPACE_BEFORE_NEWLINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t]+\n").expect("motif"));
static SPACE_AFTER_NEWLINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n[ \t]+").expect("motif"));
static MANY_NEWLINES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").expect("motif"));
static DOUBLE_MARK: Lazy<Regex> = Lazy::new(|| Regex::new(r"([.,;:?!])[ ]?[.,]").expect("motif"));

fn tidy(s: &str) -> String {
    let s = SPACE_BEFORE_NEWLINE.replace_all(s, "\n");
    let s = SPACE_AFTER_NEWLINE.replace_all(&s, "\n");
    let s = MANY_NEWLINES.replace_all(&s, "\n\n");
    let s = DOUBLE_MARK.replace_all(&s, "$1");
    s.trim_matches(|c: char| c == ' ' || c == ',').to_string()
}

#[cfg(test)]
mod tests {
    use super::apply_fr;

    #[test]
    fn dictee_naturelle_intacte() {
        let t = "Je vous propose de caler un point jeudi à 14h. Pouvez-vous me confirmer votre disponibilité ?";
        assert_eq!(apply_fr(t), t);
        assert_eq!(apply_fr("Du point de vue du client, c'est réglé."), "Du point de vue du client, c'est réglé.");
    }

    #[test]
    fn commandes_entourees_par_whisper() {
        assert_eq!(
            apply_fr("Maître virgule, je vous prie de trouver ci-joint mes conclusions. Point à la ligne, je reste à votre disposition. Point."),
            "Maître, je vous prie de trouver ci-joint mes conclusions.\nJe reste à votre disposition."
        );
    }

    #[test]
    fn deux_points_et_ligne() {
        assert_eq!(
            apply_fr("Objet deux points affaire Durand contre Martin à la ligne Monsieur le Président virgule nous sollicitons un renvoi point"),
            "Objet : affaire Durand contre Martin\nMonsieur le Président, nous sollicitons un renvoi."
        );
    }

    #[test]
    fn guillemets_parentheses_question() {
        assert_eq!(
            apply_fr("Le client soutient, ouvrez les guillemets, qu'il n'a jamais reçu la mise en demeure, fermez les guillemets. Point. Est-ce exact point d'interrogation."),
            "Le client soutient « qu'il n'a jamais reçu la mise en demeure ». Est-ce exact ?"
        );
        assert_eq!(
            apply_fr("Nous contestons le montant ouvrez la parenthèse soit 15 000 euros. Fermez la parenthèse point virgule. La demande est mal fondée point"),
            "Nous contestons le montant (soit 15 000 euros) ; la demande est mal fondée."
        );
    }

    #[test]
    fn mots_gardes() {
        assert_eq!(apply_fr("La virgule est mal placée."), "La virgule est mal placée.");
        assert_eq!(apply_fr("Il reste les deux points évoqués hier."), "Il reste les deux points évoqués hier.");
        assert_eq!(apply_fr("Faisons le point. Ensuite on verra."), "Faisons le point. Ensuite on verra.");
    }

    #[test]
    fn commandes_isolees_par_des_virgules() {
        assert_eq!(
            apply_fr("Le montant, fermez la parenthèse, point, virgule, la demande est mal fondée, point."),
            "Le montant) ; la demande est mal fondée."
        );
        assert_eq!(
            apply_fr("qu'il n'a jamais reçu la mise en demeure, fermez les guillemets, point, est-ce exact, point d'interrogation."),
            "Qu'il n'a jamais reçu la mise en demeure ». Est-ce exact ?"
        );
        assert_eq!(apply_fr("On fait le point, puis on avance."), "On fait le point, puis on avance.");
    }

    /// Whisper termine souvent un segment par « Point. » et commence le suivant par « À la ligne ».
    #[test]
    fn point_puis_a_la_ligne_separes_par_un_point() {
        assert_eq!(
            apply_fr("Je vous écris au sujet du dossier Durand. Point. À la ligne, je reste à votre disposition."),
            "Je vous écris au sujet du dossier Durand.\nJe reste à votre disposition."
        );
    }

    #[test]
    fn paragraphe() {
        assert_eq!(
            apply_fr("Bonjour virgule nouveau paragraphe veuillez trouver ci-joint la pièce point"),
            "Bonjour,\n\nVeuillez trouver ci-joint la pièce."
        );
    }
}

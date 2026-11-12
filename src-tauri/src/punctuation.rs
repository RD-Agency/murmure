// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ponctuation dictée : « virgule », « point à la ligne », « ouvrez les guillemets »… deviennent les
//! signes correspondants, comme avec une dictée de secrétariat.
//!
//! Whisper ponctue déjà seul une dictée naturelle ; quand on prononce la ponctuation, il écrit le mot
//! (« Maître virgule, je ») en l'entourant souvent de sa propre ponctuation. Chaque commande absorbe
//! donc la ponctuation et les espaces qui l'entourent, puis la typographie est recomposée.
//!
//! Garde-fous : « point » seul n'est une commande qu'en fin de phrase et hors d'un groupe nominal
//! (« caler un point jeudi », « le point de vue » restent intacts) ; « virgule », « deux points » et
//! « tiret » précédés d'un déterminant restent des mots.
//!
//! Réglages ([`Rules`]) :
//! - **ponctuation de Whisper** : gardée, ignorée dès qu'une dictée contient un signe prononcé
//!   (défaut), ou toujours ignorée. Ignorée, elle disparaît avec la majuscule qu'il avait mise
//!   derrière ses points (sauf noms propres probables) ;
//! - **majuscule après un retour à la ligne** (défaut : oui) ;
//! - **pas de majuscule après une virgule, un point-virgule ou deux-points** (défaut : oui) ;
//! - **signes personnels** : « esperluette = & », « barre oblique = / »… ajoutés à la table.
//!
//! Ces réglages s'échangent sous forme d'un fichier texte lisible ([`Rules::to_file`],
//! [`Rules::from_file`]).

use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

/// Exemple de dictée ajouté au prompt initial de Whisper : il reconnaît alors les commandes
/// (« deux points » au lieu de « 2. », « à la ligne » au lieu de « à la Lille »). Mesuré sur un corpus :
/// la forme « exemple de dictée » ne fait pas sauter le premier mot, contrairement à une liste.
pub const PROMPT_FR: &str = "Maître, virgule, je vous écris, point, à la ligne. Objet, deux points, dossier, \
point-virgule, ouvrez les guillemets, fermez les guillemets, ouvrez la parenthèse, fermez la parenthèse, \
point d'interrogation, points de suspension, nouveau paragraphe.";

// ───────────────────────────────── Réglages ─────────────────────────────────

/// Que faire de la ponctuation que Whisper ajoute de lui-même.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhisperPunctuation {
    /// Toujours gardée (comportement des versions 0.1.x).
    Keep,
    /// Ignorée dès que la dictée contient au moins un signe prononcé.
    IgnoreWhenDictated,
    /// Toujours ignorée : seule la ponctuation prononcée compte (par défaut).
    #[default]
    Ignore,
}

/// Signe ajouté par l'utilisateur : `spoken` prononcé → `sign` écrit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomMark {
    pub spoken: String,
    pub sign: String,
}

/// Règles de la ponctuation dictée (Réglages › Langue et vocabulaire › Ponctuation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rules {
    pub whisper_punctuation: WhisperPunctuation,
    /// Majuscule au premier mot après « à la ligne » / « nouveau paragraphe ».
    pub capitalize_after_newline: bool,
    /// Pas de majuscule après une virgule, un point-virgule ou deux-points (sauf nom propre probable).
    pub lowercase_after_comma: bool,
    /// Signes personnels, prioritaires sur la table intégrée.
    pub custom: Vec<CustomMark>,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            whisper_punctuation: WhisperPunctuation::default(),
            capitalize_after_newline: true,
            lowercase_after_comma: true,
            custom: default_custom(),
        }
    }
}

/// Signes personnels proposés au premier lancement (modifiables, supprimables).
pub fn default_custom() -> Vec<CustomMark> {
    [("esperluette", "&"), ("arobase", "@"), ("barre oblique", "/"), ("ouvrez les crochets", "["), ("fermez les crochets", "]")]
        .into_iter()
        .map(|(spoken, sign)| CustomMark { spoken: spoken.into(), sign: sign.into() })
        .collect()
}

impl Rules {
    /// Comportement des versions 0.1.x (aucune règle supplémentaire).
    pub fn legacy() -> Self {
        Self {
            whisper_punctuation: WhisperPunctuation::Keep,
            capitalize_after_newline: true,
            lowercase_after_comma: false,
            custom: Vec::new(),
        }
    }

    /// Fichier d'instructions lisible et modifiable à la main.
    pub fn to_file(&self) -> String {
        let yes = |b: bool| if b { "oui" } else { "non" };
        let whisper = match self.whisper_punctuation {
            WhisperPunctuation::Keep => "garder",
            WhisperPunctuation::IgnoreWhenDictated => "ignorer si je dicte",
            WhisperPunctuation::Ignore => "toujours ignorer",
        };
        let mut out = String::from(
            "# Murmure : fichier d'instructions de ponctuation\n\
             # Les lignes qui commencent par # sont ignorées. Modifiez puis importez ce fichier\n\
             # dans Réglages › Langue et vocabulaire › Ponctuation.\n\n\
             [règles]\n",
        );
        out.push_str("# garder | ignorer si je dicte | toujours ignorer\n");
        out.push_str(&format!("ponctuation de whisper = {whisper}\n"));
        out.push_str(&format!("majuscule après un retour à la ligne = {}\n", yes(self.capitalize_after_newline)));
        out.push_str(&format!(
            "pas de majuscule après une virgule ou un point-virgule = {}\n",
            yes(self.lowercase_after_comma)
        ));
        out.push_str("\n[signes]\n# mot prononcé = signe écrit (\\n pour un retour à la ligne)\n");
        for m in &self.custom {
            out.push_str(&format!("{} = {}\n", m.spoken.trim(), escape_sign(&m.sign)));
        }
        out
    }

    /// Lit un fichier d'instructions. Les règles absentes gardent leur valeur par défaut ; une
    /// ligne illisible est signalée avec son numéro.
    pub fn from_file(text: &str) -> Result<Self, String> {
        let mut rules = Rules { custom: Vec::new(), ..Rules::default() };
        let mut section = "";
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = match fold(&line[1..line.len() - 1]).as_str() {
                    "regles" => "rules",
                    "signes" | "ponctuation" => "marks",
                    other => return Err(format!("ligne {} : section inconnue « {other} »", n + 1)),
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("ligne {} : « mot = valeur » attendu", n + 1));
            };
            let (key, value) = (key.trim(), value.trim());
            match section {
                "rules" => {
                    let k = fold(key);
                    let v = fold(value);
                    let flag = || match v.as_str() {
                        "oui" | "vrai" | "yes" | "true" | "1" => Ok(true),
                        "non" | "faux" | "no" | "false" | "0" => Ok(false),
                        _ => Err(format!("ligne {} : « oui » ou « non » attendu", n + 1)),
                    };
                    if k.contains("whisper") {
                        rules.whisper_punctuation = if v.contains("toujours") {
                            WhisperPunctuation::Ignore
                        } else if v.contains("ignor") {
                            WhisperPunctuation::IgnoreWhenDictated
                        } else if v.contains("garder") {
                            WhisperPunctuation::Keep
                        } else {
                            return Err(format!("ligne {} : « garder », « ignorer si je dicte » ou « toujours ignorer » attendu", n + 1));
                        };
                    } else if k.contains("ligne") || k.contains("paragraphe") {
                        rules.capitalize_after_newline = flag()?;
                    } else if k.contains("virgule") {
                        rules.lowercase_after_comma = flag()?;
                    } else {
                        return Err(format!("ligne {} : règle inconnue « {key} »", n + 1));
                    }
                }
                "marks" => {
                    if key.is_empty() || value.is_empty() {
                        return Err(format!("ligne {} : mot et signe obligatoires", n + 1));
                    }
                    rules.custom.push(CustomMark { spoken: key.to_string(), sign: unescape_sign(value) });
                }
                _ => return Err(format!("ligne {} : section [règles] ou [signes] attendue avant", n + 1)),
            }
        }
        Ok(rules)
    }
}

fn escape_sign(s: &str) -> String {
    s.replace('\n', "\\n")
}

fn unescape_sign(s: &str) -> String {
    s.replace("\\n", "\n")
}

/// Minuscules sans accents (comparaison des clés du fichier).
fn fold(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

// ───────────────────────────────── Signes ─────────────────────────────────

/// Rôle d'un signe dans la phrase : décide des majuscules qui suivent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// . ? ! : fin de phrase.
    Stop,
    /// Retour à la ligne (éventuellement précédé d'un point).
    Newline { period: bool },
    /// , ; :
    Weak,
    /// « ( [
    Open,
    /// » ) ]
    Close,
    /// … : ni majuscule imposée ni retirée.
    Neutral,
}

/// Signe à écrire et espaces qui l'entourent (typographie française).
#[derive(Clone, Debug)]
struct Spec {
    text: String,
    space_before: bool,
    space_after: bool,
    kind: Kind,
}

fn spec(text: &str, space_before: bool, space_after: bool, kind: Kind) -> Spec {
    Spec { text: text.to_string(), space_before, space_after, kind }
}

/// Espaces et rôle déduits du signe écrit par l'utilisateur.
fn spec_for_sign(sign: &str) -> Spec {
    let trimmed = sign.trim_matches(' ');
    if trimmed.contains('\n') {
        let period = trimmed.starts_with('.');
        return spec(trimmed, false, false, Kind::Newline { period });
    }
    match trimmed {
        "." => spec(".", false, true, Kind::Stop),
        "?" | "!" => spec(trimmed, true, true, Kind::Stop),
        "," => spec(",", false, true, Kind::Weak),
        ";" | ":" => spec(trimmed, true, true, Kind::Weak),
        "…" | "..." => spec("…", false, true, Kind::Neutral),
        "«" => spec("«", true, true, Kind::Open),
        "»" => spec("»", true, true, Kind::Close),
        "(" | "[" | "{" => spec(trimmed, true, false, Kind::Open),
        ")" | "]" | "}" => spec(trimmed, false, true, Kind::Close),
        "/" | "@" | "-" | "_" | "'" | "’" => spec(trimmed, false, false, Kind::Neutral),
        _ => spec(trimmed, true, true, Kind::Neutral),
    }
}

/// Ponctuation et espaces que Whisper place autour d'un mot de commande.
const AROUND: &str = r"[\s,.;:!?…]*";

/// Déterminants / prépositions qui font d'un mot de commande un simple nom.
const NOMINAL: &str = r"(?i:le|la|les|un|une|des|du|au|aux|ce|cet|cette|ces|mon|ma|mes|ton|ta|tes|son|sa|ses|notre|nos|votre|vos|leur|leurs|chaque|quel|quelle|en|sur|de|d'|d’|à|seule?|dernier|dernière|premier|première)";

/// Apostrophe droite ou typographique dans les motifs.
fn apos(words: &str) -> String {
    words.replace('\'', "['’]")
}

fn command_re(words: &str) -> Regex {
    Regex::new(&format!(r"(?i){AROUND}\b(?:{})\b{AROUND}", apos(words))).expect("motif de ponctuation")
}

fn guarded_re(words: &str) -> Regex {
    Regex::new(&format!(r"(?i)(?P<prev>[\p{{L}}'’]+)?(?P<cmd>{AROUND}\b(?:{}){AROUND})", apos(words))).expect("motif de ponctuation")
}

/// Table intégrée. Ordre = priorité (les plus longues d'abord).
static COMMANDS: Lazy<Vec<(Regex, Spec)>> = Lazy::new(|| {
    let nl = |t: &str, period: bool| spec(t, false, false, Kind::Newline { period });
    vec![
        (command_re(r"point[,.]?\s+(?:et\s+)?(?:nouveau\s+paragraphe|nouvel\s+alinéa)"), nl(".\n\n", true)),
        (command_re(r"point[,.]?\s+(?:et\s+)?(?:(?:retour|passez|passer|aller)\s+)?à\s+la\s+ligne|point[,.]?\s+nouvelle\s+ligne"), nl(".\n", true)),
        (command_re(r"nouveau\s+paragraphe|nouvel\s+alinéa|saut\s+de\s+paragraphe"), nl("\n\n", false)),
        (command_re(r"(?:retour|passez|passer|aller)\s+à\s+la\s+ligne|à\s+la\s+ligne|nouvelle\s+ligne"), nl("\n", false)),
        (command_re(r"points?\s+d'interrogation"), spec("?", true, true, Kind::Stop)),
        (command_re(r"points?\s+d'exclamation"), spec("!", true, true, Kind::Stop)),
        (command_re(r"points?\s+de\s+suspension|trois\s+petits\s+points"), spec("…", false, true, Kind::Neutral)),
        (command_re(r"point[\s,.-]+virgule"), spec(";", true, true, Kind::Weak)),
        (command_re(r"point\s+final"), spec(".", false, true, Kind::Stop)),
        (
            command_re(r"(?:ouvrez|ouvrir|ouvre|ouvrons|ouvrent)\s+(?:les\s+|des\s+)?guillemets?|guillemets?\s+ouvrants?|début\s+de\s+(?:la\s+)?citation"),
            spec("«", true, true, Kind::Open),
        ),
        (
            command_re(r"(?:fermez|fermer|ferme|fermons|ferment)\s+(?:les\s+|des\s+)?guillemets?|guillemets?\s+fermants?|fin\s+des?\s+guillemets|fin\s+de\s+(?:la\s+)?citation"),
            spec("»", true, true, Kind::Close),
        ),
        (command_re(r"(?:ouvrez|ouvrir|ouvre|ouvrons)\s+(?:la\s+|une\s+)?parenthèses?"), spec("(", true, false, Kind::Open)),
        (command_re(r"(?:fermez|fermer|ferme|fermons)\s+(?:la\s+)?parenthèses?"), spec(")", false, true, Kind::Close)),
    ]
});

/// Commandes ambiguës : on vérifie le mot qui précède avant de remplacer.
static GUARDED: Lazy<Vec<(Regex, Spec)>> = Lazy::new(|| {
    vec![
        (guarded_re(r"\bdeux[\s-]points\b|\b2\s+points\b"), spec(":", true, true, Kind::Weak)),
        (guarded_re(r"\bvirgule\b"), spec(",", false, true, Kind::Weak)),
        (guarded_re(r"\btiret\b"), spec("–", true, true, Kind::Neutral)),
        (guarded_re(r"\btrait\s+d'union\b"), spec("-", false, false, Kind::Neutral)),
    ]
});

/// « point » seul : commande seulement s'il termine la dictée ou précède une nouvelle phrase / ligne.
static LONE_POINT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?P<prev>[\p{L}\d'’»)]+)?(?P<cmd>(?P<before>[\s,.;:!?…]*)\b[Pp]oint\b(?P<after>[\s,.;:!?…]*))(?P<next>$|\n|\p{L}|\x{E000})")
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

/// Mots courants qu'on peut remettre en minuscule sans risque (jamais des noms propres).
static COMMON_WORDS: Lazy<std::collections::HashSet<&'static str>> = Lazy::new(|| {
    "je j tu il elle on nous vous ils elles me te se moi toi lui leur eux y en ne n pas plus moins \
     le la les l un une des du de d au aux à a ce c cet cette ces ceci cela ça ceux celle celles celui \
     mon ma mes ton ta tes son sa ses notre nos votre vos leurs \
     qui que qu quoi dont où quel quelle quels quelles lequel laquelle lesquels lesquelles \
     et ou ni mais car or donc puis ensuite enfin alors ainsi aussi cependant toutefois néanmoins \
     pourtant sinon si quand lorsque lorsqu comme puisque parce afin tandis bien très trop peu \
     dans par pour sur sous chez vers entre contre sans avec avant après depuis pendant selon malgré \
     dès durant envers hormis outre parmi suivant concernant conformément s'agissant nonobstant \
     tout tous toute toutes rien aucun aucune chacun chacune plusieurs certains certaines autre \
     autres même mêmes tel telle tels telles quelque quelques \
     déjà encore toujours jamais souvent parfois ici là oui non peut-être également notamment \
     surtout d'abord d'ailleurs ailleurs enfin bref effectivement évidemment naturellement \
     simplement seulement vraiment actuellement désormais dorénavant aujourd'hui demain hier \
     est sont était étaient été être sera seront serait seraient soit soient \
     ai as avons avez ont avait avaient aura auront aurait auraient avoir eu \
     fait font faire faut peut peuvent pouvez pouvons doit doivent devez devons \
     veuillez merci voici voilà vu compte tenu attendu considérant \
     madame monsieur mesdames messieurs maître cher chère chers chères"
        .split_whitespace()
        .collect()
});

/// Abréviations dont le point appartient au mot (jamais retiré).
static ABBREVIATIONS: Lazy<std::collections::HashSet<&'static str>> = Lazy::new(|| {
    "m mm me mes mme mmes mlle mlles dr pr st ste art al cf etc av bd vol p pp chap n no éd ed op cit ibid \
     janv févr avr juil sept oct nov déc"
        .split_whitespace()
        .collect()
});

const SENTINEL: char = '\u{E000}';

// ───────────────────────────────── Application ─────────────────────────────────

/// Remplace la ponctuation prononcée par les signes, avec les règles des versions 0.1.x.
pub fn apply_fr(text: &str) -> String {
    apply_fr_with(text, &Rules::legacy())
}

/// Remplace la ponctuation prononcée par les signes et recompose la typographie française.
pub fn apply_fr_with(text: &str, rules: &Rules) -> String {
    let mut marks: Vec<Spec> = Vec::new();
    let put = |marks: &mut Vec<Spec>, m: &Spec| {
        marks.push(m.clone());
        format!(" {SENTINEL}{} ", marks.len() - 1)
    };

    let mut s = text.to_string();

    // Signes personnels d'abord (prioritaires), les plus longs en premier.
    let mut custom: Vec<&CustomMark> = rules.custom.iter().filter(|m| !m.spoken.trim().is_empty() && !m.sign.is_empty()).collect();
    custom.sort_by_key(|m| std::cmp::Reverse(m.spoken.trim().chars().count()));
    for m in custom {
        let words = m.spoken.split_whitespace().map(regex::escape).collect::<Vec<_>>().join(r"\s+");
        let Ok(re) = Regex::new(&format!(r"(?i){AROUND}\b(?:{}){AROUND}", apos(&words))) else { continue };
        let sp = spec_for_sign(&m.sign);
        s = re.replace_all(&s, |_: &Captures| put(&mut marks, &sp)).into_owned();
    }
    for (re, sp) in COMMANDS.iter() {
        s = re.replace_all(&s, |_: &Captures| put(&mut marks, sp)).into_owned();
    }
    for (re, sp) in GUARDED.iter() {
        s = re
            .replace_all(&s, |c: &Captures| {
                let prev = c.name("prev").map_or("", |m| m.as_str());
                if NOMINAL_RE.is_match(prev) {
                    c[0].to_string()
                } else {
                    format!("{prev}{}", put(&mut marks, sp))
                }
            })
            .into_owned();
    }
    let before_lone = s.clone();
    let period = spec(".", false, true, Kind::Stop);
    s = LONE_POINT
        .replace_all(&before_lone, |c: &Captures| {
            let prev = c.name("prev").map_or("", |m| m.as_str());
            let next = c.name("next").map_or("", |m| m.as_str());
            // Whisper isole par des virgules un mot prononcé comme une commande (« reçu, point, est-ce »).
            let start = c.get(0).map_or(0, |m| m.start());
            let after_mark = before_lone[..start].trim_end_matches(|ch: char| ch.is_ascii_digit()).ends_with(SENTINEL);
            let isolated = (after_mark || c["before"].contains([',', '.'])) && c["after"].contains([',', '.']);
            let boundary = next.is_empty()
                || next == "\n"
                || next.starts_with(SENTINEL)
                || next.chars().next().is_some_and(char::is_uppercase);
            if prev.is_empty() || NOMINAL_RE.is_match(prev) || !(boundary || isolated) {
                c[0].to_string()
            } else {
                format!("{prev}{}{next}", put(&mut marks, &period))
            }
        })
        .into_owned();

    let strip_whisper = match rules.whisper_punctuation {
        WhisperPunctuation::Keep => false,
        WhisperPunctuation::IgnoreWhenDictated => !marks.is_empty(),
        WhisperPunctuation::Ignore => true,
    };
    if marks.is_empty() && !strip_whisper && !rules.lowercase_after_comma {
        return text.to_string();
    }
    let lower_words = lowercase_vocabulary(text);
    let out = compose(&s, &marks, rules, strip_whisper, &lower_words);
    if rules.whisper_punctuation != WhisperPunctuation::Keep {
        drop_trailing_ellipsis(&out, &marks)
    } else {
        out
    }
}

/// Mots écrits en minuscules ailleurs dans le texte : ce sont des mots courants, pas des noms propres.
fn lowercase_vocabulary(text: &str) -> std::collections::HashSet<String> {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.chars().next().is_some_and(char::is_lowercase))
        .map(str::to_string)
        .collect()
}

/// Peut-on remettre ce mot en minuscule sans risquer d'abîmer un nom propre ?
fn is_common(word: &str, lower_words: &std::collections::HashSet<String>) -> bool {
    let core = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    let mut chars = core.chars();
    let Some(first) = chars.next() else { return false };
    if !first.is_uppercase() {
        return false;
    }
    // Sigle (« RGPD ») ou casse mixte (« McDonald », « iPhone ») : intouchable.
    let rest: String = chars.collect();
    if rest.chars().any(char::is_uppercase) || (rest.is_empty() && !core.contains(['\'', '’'])) {
        return false;
    }
    let lower = core.to_lowercase();
    // Élision : « J'ai », « L'audience », « Qu'il »…
    if let Some((head, _)) = lower.split_once(['\'', '’']) {
        return COMMON_WORDS.contains(head);
    }
    // Inversion verbe-sujet : « Pouvez-vous », « Est-il », « Voulez-vous ».
    if let Some((head, tail)) = lower.split_once('-')
        && (FUNCTION_WORD.is_match(&upper_first(tail)) || COMMON_WORDS.contains(tail))
    {
        return COMMON_WORDS.contains(head) || lower_words.contains(head);
    }
    FUNCTION_WORD.is_match(core) || COMMON_WORDS.contains(lower.as_str()) || lower_words.contains(&lower)
}

/// Point ou signe de Whisper collé au mot, à retirer quand seule la ponctuation dictée compte.
/// Renvoie le mot nettoyé et s'il terminait une phrase.
fn strip_whisper_mark(token: &str) -> (String, bool) {
    let core_end = token.trim_end_matches([',', '.', ';', ':', '!', '?', '…']);
    if core_end.len() == token.len() {
        return (token.to_string(), false);
    }
    let removed = &token[core_end.len()..];
    let bare = core_end.rsplit(['\'', '’']).next().unwrap_or(core_end);
    let bare = bare.trim_start_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    // « M. », « art. », « etc. » : le point fait partie du mot (on retire seulement le reste).
    if removed.starts_with('.') && ABBREVIATIONS.contains(bare.as_str()) {
        return (format!("{core_end}."), false);
    }
    let stop = removed.contains(['.', '!', '?', '…']);
    (core_end.to_string(), stop)
}

/// Recompose le texte : signes collés au bon mot, espaces françaises, majuscules en début de phrase.
fn compose(s: &str, marks: &[Spec], rules: &Rules, strip_whisper: bool, lower_words: &std::collections::HashSet<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut capitalize = true;
    let mut lower_next = false;
    let mut prev_mark = false;
    let mut glue_next = false;
    for token in s.split(' ').filter(|t| !t.is_empty()) {
        if let Some(idx) = token.strip_prefix(SENTINEL) {
            let Some(mark) = marks.get(idx.parse::<usize>().unwrap_or(usize::MAX)) else { continue };
            while out.ends_with(' ')
                || (!prev_mark && out.ends_with([',', '.']) && !matches!(mark.kind, Kind::Open) && !ends_with_abbreviation(&out))
            {
                out.pop();
            }
            if mark.space_before && !(out.is_empty() || out.ends_with(['\n', ' ', '(', '['])) {
                out.push(' ');
            }
            out.push_str(&mark.text);
            capitalize = match mark.kind {
                Kind::Stop => true,
                Kind::Newline { period } => period || rules.capitalize_after_newline,
                Kind::Open => capitalize,
                _ => false,
            };
            lower_next = matches!(mark.kind, Kind::Weak | Kind::Open | Kind::Close)
                || (matches!(mark.kind, Kind::Newline { period: false }) && !rules.capitalize_after_newline);
            glue_next = !mark.space_after;
            prev_mark = true;
            continue;
        }
        // Ponctuation de Whisper : seule sur son jeton (« ? ») ou collée au mot (« point. »).
        let whisper_weak = strip_whisper && token.ends_with([',', ';', ':']);
        let (token, whisper_stop) = if strip_whisper { strip_whisper_mark(token) } else { (token.to_string(), false) };
        if token.is_empty() || !token.chars().any(char::is_alphanumeric) && strip_whisper {
            if (whisper_stop || (rules.lowercase_after_comma && whisper_weak)) && !prev_mark {
                lower_next = true;
            }
            continue;
        }
        let comma_rule = rules.lowercase_after_comma;
        let word = if capitalize {
            upper_first(&token)
        } else if lower_next && (if comma_rule { is_common(&token, lower_words) } else { FUNCTION_WORD.is_match(token.trim_end_matches([',', '.', ';', ':', '!', '?'])) }) {
            lower_first(&token)
        } else {
            token.clone()
        };
        if !(out.is_empty() || glue_next || out.ends_with([' ', '\n', '(', '['])) {
            out.push(' ');
        }
        out.push_str(&word);
        glue_next = false;
        prev_mark = false;
        if strip_whisper {
            // Whisper avait mis un point (ou une virgule) : la phrase continue, sa majuscule suivante
            // est retirée.
            capitalize = false;
            lower_next = whisper_stop || (comma_rule && whisper_weak);
        } else {
            let bare = word.trim_end_matches(['.', '?', '!']);
            capitalize = word.ends_with(['.', '?', '!']) && !is_abbreviation(bare);
            lower_next = comma_rule && word.ends_with([',', ';', ':']);
        }
    }
    tidy(&out)
}

fn is_abbreviation(word_without_point: &str) -> bool {
    let w = word_without_point.rsplit(['\'', '’']).next().unwrap_or(word_without_point);
    let w = w.trim_start_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    ABBREVIATIONS.contains(w.as_str())
}

fn ends_with_abbreviation(out: &str) -> bool {
    out.strip_suffix('.').is_some_and(|rest| is_abbreviation(rest.rsplit([' ', '\n']).next().unwrap_or("")))
}

/// Whisper invente souvent « … » sur le silence du relâchement : retiré s'il n'a pas été dicté.
fn drop_trailing_ellipsis(out: &str, marks: &[Spec]) -> String {
    let dictated = marks.last().is_some_and(|m| m.text == "…");
    let trimmed = out.trim_end();
    if dictated {
        return out.to_string();
    }
    for tail in ["...", "…"] {
        if let Some(rest) = trimmed.strip_suffix(tail) {
            return rest.trim_end().to_string();
        }
    }
    out.to_string()
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
static DOUBLE_MARK: Lazy<Regex> = Lazy::new(|| Regex::new(r"([,;:?!])[ ]?[.,]|(\.)[ ]?,").expect("motif"));
static DOUBLE_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r" {2,}").expect("motif"));

fn tidy(s: &str) -> String {
    let s = SPACE_BEFORE_NEWLINE.replace_all(s, "\n");
    let s = SPACE_AFTER_NEWLINE.replace_all(&s, "\n");
    let s = MANY_NEWLINES.replace_all(&s, "\n\n");
    let s = DOUBLE_MARK.replace_all(&s, "$1$2");
    let s = DOUBLE_SPACE.replace_all(&s, " ");
    s.trim_matches(|c: char| c == ' ' || c == ',').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictee_naturelle_intacte() {
        let t = "Je vous propose de caler un point jeudi à 14h. Pouvez-vous me confirmer votre disponibilité ?";
        assert_eq!(apply_fr(t), t);
        assert_eq!(apply_fr("Du point de vue du client, c'est réglé."), "Du point de vue du client, c'est réglé.");
        // « Si je dicte » : sans signe dicté, la ponctuation de Whisper reste.
        let when = Rules { whisper_punctuation: WhisperPunctuation::IgnoreWhenDictated, ..Rules::default() };
        assert_eq!(apply_fr_with(t, &when), t);
        // Par défaut (« toujours ») : seule la ponctuation dictée est écrite.
        assert_eq!(
            apply_fr_with(t, &Rules::default()),
            "Je vous propose de caler un point jeudi à 14h pouvez-vous me confirmer votre disponibilité"
        );
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
        assert_eq!(apply_fr("Ajoutez un tiret ici."), "Ajoutez un tiret ici.");
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

    // ───── Retours de Me Cuif (issue « Murmure : usage », novembre 2026) ─────

    /// Bug : « point nouveau paragraphe » écrivait « point » puis le paragraphe.
    #[test]
    fn point_nouveau_paragraphe() {
        assert_eq!(
            apply_fr("Je reste à votre disposition point nouveau paragraphe veuillez agréer"),
            "Je reste à votre disposition.\n\nVeuillez agréer"
        );
        assert_eq!(
            apply_fr("Je reste à votre disposition. Point, nouveau paragraphe. Veuillez agréer"),
            "Je reste à votre disposition.\n\nVeuillez agréer"
        );
        assert_eq!(apply_fr("C'est noté point nouvelle ligne merci"), "C'est noté.\nMerci");
    }

    #[test]
    fn points_de_suspension_singulier_ou_pluriel() {
        assert_eq!(apply_fr("Et alors points de suspension"), "Et alors…");
        assert_eq!(apply_fr("Et alors, point de suspension."), "Et alors…");
        assert_eq!(apply_fr("Il hésite trois petits points puis signe point"), "Il hésite… puis signe.");
    }

    #[test]
    fn guillemets_variantes_et_apostrophe_typographique() {
        assert_eq!(
            apply_fr("Il écrit guillemets ouvrants bon pour accord guillemets fermants point"),
            "Il écrit « bon pour accord »."
        );
        assert_eq!(apply_fr("Est-ce exact point d’interrogation"), "Est-ce exact ?");
        assert_eq!(apply_fr("Il dit ouvrir guillemets oui fin des guillemets"), "Il dit « oui »");
    }

    #[test]
    fn signes_personnels() {
        let rules = Rules { whisper_punctuation: WhisperPunctuation::Keep, ..Rules::default() };
        assert_eq!(apply_fr_with("Cabinet Durand esperluette associés", &rules), "Cabinet Durand & associés");
        assert_eq!(apply_fr_with("Écrivez à contact arobase lx point legal", &rules), "Écrivez à contact@lx point legal");
        assert_eq!(apply_fr_with("Les pièces une barre oblique deux", &rules), "Les pièces une/deux");
        let custom = Rules {
            custom: vec![CustomMark { spoken: "point virgule".into(), sign: "§".into() }],
            ..Rules::legacy()
        };
        // Signe personnel prioritaire sur la table intégrée.
        assert_eq!(apply_fr_with("Voir point virgule douze", &custom), "Voir § douze");
    }

    /// « Majuscule après une virgule dès lors qu'il y a eu une pause » : Whisper repart en majuscule.
    #[test]
    fn pas_de_majuscule_apres_une_virgule() {
        let rules = Rules::default();
        assert_eq!(
            apply_fr_with("Maître virgule. Veuillez trouver ci-joint mes conclusions point", &rules),
            "Maître, veuillez trouver ci-joint mes conclusions."
        );
        assert_eq!(
            apply_fr_with("En conséquence point-virgule. Nous sollicitons le renvoi point", &rules),
            "En conséquence ; nous sollicitons le renvoi."
        );
        // Sans signe dicté : la virgule de Whisper suivie d'une majuscule.
        let when = Rules { whisper_punctuation: WhisperPunctuation::IgnoreWhenDictated, ..Rules::default() };
        assert_eq!(apply_fr_with("Bonjour, Veuillez noter que, Il est absent.", &when), "Bonjour, veuillez noter que, il est absent.");
        // Par défaut : les virgules de Whisper partent, et leur majuscule avec.
        assert_eq!(apply_fr_with("Bonjour, Veuillez noter que, Il est absent.", &rules), "Bonjour veuillez noter que il est absent");
        // Nom propre probable : gardé.
        assert_eq!(apply_fr_with("Bonjour virgule Durand point", &rules), "Bonjour, Durand.");
        assert_eq!(apply_fr_with("Objet deux points RGPD point", &rules), "Objet : RGPD.");
        // Règle désactivée : seuls les mots-outils sont remis en minuscule (comportement 0.1.x).
        let off = Rules { lowercase_after_comma: false, ..rules };
        assert_eq!(apply_fr_with("Maître virgule. Veuillez noter point", &off), "Maître, Veuillez noter.");
    }

    #[test]
    fn majuscule_apres_retour_a_la_ligne_reglable() {
        let on = Rules::default();
        assert_eq!(apply_fr_with("Bonjour virgule à la ligne je vous écris point", &on), "Bonjour,\nJe vous écris.");
        let off = Rules { capitalize_after_newline: false, ..Rules::default() };
        assert_eq!(apply_fr_with("Bonjour virgule à la ligne je vous écris point", &off), "Bonjour,\nje vous écris.");
        // Après « point à la ligne », la majuscule reste : c'est une nouvelle phrase.
        assert_eq!(apply_fr_with("Merci point à la ligne je reste à votre disposition", &off), "Merci.\nJe reste à votre disposition");
    }

    /// « Les modèles ajoutent de la ponctuation non demandée » : seule la ponctuation dictée compte.
    #[test]
    fn ponctuation_de_whisper_ignoree_quand_on_dicte() {
        let rules = Rules::default();
        assert_eq!(
            apply_fr_with("Je vous écris. Au sujet du dossier Durand virgule que nous suivons. Point.", &rules),
            "Je vous écris au sujet du dossier Durand, que nous suivons."
        );
        // Abréviations et nombres intacts.
        assert_eq!(
            apply_fr_with("M. Durand demande 1 500,50 euros au titre de l'art. 700 point", &rules),
            "M. Durand demande 1 500,50 euros au titre de l'art. 700."
        );
        // Point d'interrogation non dicté : retiré.
        assert_eq!(apply_fr_with("Pouvez-vous confirmer ? Merci virgule cordialement", &rules), "Pouvez-vous confirmer merci, cordialement");
        // « Toujours ignorer » : même sans signe dicté.
        let always = Rules { whisper_punctuation: WhisperPunctuation::Ignore, ..Rules::default() };
        assert_eq!(apply_fr_with("Je vous écris. Au sujet du dossier.", &always), "Je vous écris au sujet du dossier");
        // « Garder » : comportement 0.1.x.
        let keep = Rules { whisper_punctuation: WhisperPunctuation::Keep, ..Rules::default() };
        assert_eq!(apply_fr_with("Je vous écris. Au sujet virgule du dossier", &keep), "Je vous écris. Au sujet, du dossier");
    }

    #[test]
    fn points_de_suspension_inventes_au_relachement() {
        let rules = Rules::default();
        assert_eq!(apply_fr_with("Je vous confirme notre accord...", &rules), "Je vous confirme notre accord");
        assert_eq!(apply_fr_with("Je vous confirme notre accord…", &rules), "Je vous confirme notre accord");
        // Dictés : gardés.
        assert_eq!(apply_fr_with("Et alors points de suspension", &rules), "Et alors…");
        // Règle « garder » : rien ne change.
        assert_eq!(apply_fr("Je vous confirme notre accord..."), "Je vous confirme notre accord...");
    }

    #[test]
    fn fichier_d_instructions_aller_retour() {
        let rules = Rules {
            whisper_punctuation: WhisperPunctuation::Ignore,
            capitalize_after_newline: false,
            lowercase_after_comma: true,
            custom: vec![
                CustomMark { spoken: "esperluette".into(), sign: "&".into() },
                CustomMark { spoken: "saut de page".into(), sign: "\n\n\n".into() },
            ],
        };
        let file = rules.to_file();
        assert!(file.contains("ponctuation de whisper = toujours ignorer"));
        assert!(file.contains("saut de page = \\n\\n\\n"));
        assert_eq!(Rules::from_file(&file).unwrap(), rules);
    }

    #[test]
    fn fichier_d_instructions_souple_et_erreurs() {
        let r = Rules::from_file("[Règles]\nMajuscule après un retour à la ligne = NON\n[signes]\nparagraphe = §\n").unwrap();
        assert!(!r.capitalize_after_newline);
        assert!(r.lowercase_after_comma, "règle absente : valeur par défaut");
        assert_eq!(r.custom, vec![CustomMark { spoken: "paragraphe".into(), sign: "§".into() }]);
        assert!(Rules::from_file("esperluette = &").unwrap_err().contains("ligne 1"));
        assert!(Rules::from_file("[règles]\nmajuscule après un retour à la ligne = peut-être").unwrap_err().contains("oui"));
        assert!(Rules::from_file("[autre]").unwrap_err().contains("section inconnue"));
        assert!(Rules::from_file("[signes]\nsans signe").unwrap_err().contains("ligne 2"));
    }
}

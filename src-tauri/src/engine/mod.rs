// SPDX-License-Identifier: AGPL-3.0-or-later
//! Moteurs de transcription : local (whisper.cpp) et serveur (API compatible OpenAI).

pub mod local;
pub mod remote;

pub use local::LocalEngine;
pub use remote::RemoteEngine;

use std::io::Cursor;

#[derive(Clone, Debug, Default)]
pub struct TranscribeOptions {
    /// Code ISO 639-1 ; `None` (ou "auto") = détection automatique.
    pub language: Option<String>,
    /// Vocabulaire / style (noms propres, jargon).
    pub initial_prompt: Option<String>,
}

/// Transcription et confiance par mot (moteur local ; vide pour le moteur serveur).
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    pub text: String,
    pub words: Vec<crate::correction::WordConfidence>,
}

impl From<String> for Transcript {
    /// Texte sans confiance par mot (moteur distant, tests).
    fn from(text: String) -> Self {
        Self { text, words: Vec::new() }
    }
}

impl TranscribeOptions {
    /// Langue effective : `None` si absente, vide ou "auto".
    pub(crate) fn language_code(&self) -> Option<&str> {
        self.language
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case("auto"))
    }

    /// Prompt initial effectif : `None` si absent ou vide.
    pub(crate) fn prompt(&self) -> Option<&str> {
        self.initial_prompt.as_deref().map(str::trim).filter(|p| !p.is_empty())
    }
}

/// Encode un signal mono 16 kHz f32 en WAV PCM 16 bits, en mémoire.
pub fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    let written = (|| -> Result<(), hound::Error> {
        let mut w = hound::WavWriter::new(&mut cursor, spec)?;
        for &s in samples {
            w.write_sample((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)?;
        }
        w.finalize()
    })();
    if let Err(e) = written {
        // Impossible en pratique (écriture en mémoire) ; on journalise sans paniquer.
        log::error!("Encodage WAV impossible : {e}");
        return Vec::new();
    }
    cursor.into_inner()
}

/// Expressions que Whisper invente sur du silence ou du bruit (sous-titres YouTube…).
/// Comparées en minuscules sur le début de chaque phrase.
const HALLUCINATED_SENTENCES: &[&str] = &[
    "sous-titres réalisés",
    "sous-titres realises",
    "sous-titrage",
    "sous-titres par",
    "sous-titres fait par",
    "sous titres réalisés",
    "merci d'avoir regardé",
    "merci d’avoir regardé",
    "merci à tous d'avoir regardé",
    "abonnez-vous",
    "n'oubliez pas de vous abonner",
    "n'hésitez pas à vous abonner",
    "thanks for watching",
    "thank you for watching",
    "subtitles by",
    "transcription by",
];

/// Fragments qui disqualifient une phrase où qu'ils apparaissent.
const HALLUCINATED_FRAGMENTS: &[&str] = &["amara.org", "radio-canada", "st' 501"];

/// Mots non vocaux que Whisper écrit entre crochets, parenthèses ou astérisques.
const NON_SPEECH_TAGS: &[&str] = &[
    "musique", "music", "rires", "rire", "applaudissements", "applause", "laughter", "silence",
    "bruit", "bruits", "inaudible", "blank_audio", "toux", "soupir", "noise",
];

/// Nettoie une transcription brute : balises non vocales, hallucinations classiques,
/// espaces superflus.
pub fn clean_transcript(raw: &str) -> String {
    let text = strip_tags(raw);
    let mut kept = String::with_capacity(text.len());
    for sentence in split_sentences(&text) {
        let lower = sentence.trim().to_lowercase();
        let core = lower.trim_start_matches(|c: char| !c.is_alphanumeric());
        let hallucinated = HALLUCINATED_SENTENCES.iter().any(|h| core.starts_with(h))
            || HALLUCINATED_FRAGMENTS.iter().any(|h| core.contains(h));
        if !hallucinated {
            kept.push_str(sentence);
        }
    }
    let collapsed = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    // Une transcription réduite à de la ponctuation (« ... », « - ») ne vaut rien.
    if !collapsed.chars().any(char::is_alphanumeric) {
        return String::new();
    }
    collapsed
}

/// Supprime `[…]` (toujours non vocal chez Whisper), `<|…|>` (jeton spécial, timestamp compris),
/// `♪`, et `(musique)` / `*rires*`.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some(inner) = rest.strip_prefix("<|")
            && let Some(end) = inner.find("|>")
            && !inner[..end].contains(char::is_whitespace)
        {
            out.push(' ');
            rest = &inner[end + 2..];
            continue;
        }
        let closing = match c {
            '[' => Some(']'),
            '(' => Some(')'),
            '*' => Some('*'),
            _ => None,
        };
        if let Some(close) = closing
            && let Some(end) = rest[1..].find(close)
        {
            let tag = rest[1..1 + end].trim().to_lowercase();
            let is_tag = c == '['
                || NON_SPEECH_TAGS.iter().any(|t| tag == *t || tag.starts_with(&format!("{t} ")));
            if is_tag {
                out.push(' ');
                rest = &rest[1 + end + close.len_utf8()..];
                continue;
            }
        }
        if c != '♪' && c != '♫' {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Découpe en phrases en conservant la ponctuation finale et les espaces.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?' | '…') {
            // Regroupe « ?! » ou « ... » avec la phrase.
            let mut end = i + c.len_utf8();
            while let Some(&(j, n)) = chars.peek() {
                if matches!(n, '.' | '!' | '?' | '…') {
                    end = j + n.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            // « Amara.org », « 2.0 » : pas une fin de phrase sans espace derrière.
            let boundary = text[end..].chars().next().is_none_or(char::is_whitespace);
            if boundary {
                out.push(&text[start..end]);
                start = end;
            }
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_en_memoire() {
        let samples = vec![0.0f32, 0.5, -0.5, 1.5];
        let wav = encode_wav(&samples);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + samples.len() * 2);
        let r = hound::WavReader::new(Cursor::new(wav)).unwrap();
        let spec = r.spec();
        assert_eq!((spec.channels, spec.sample_rate, spec.bits_per_sample), (1, 16_000, 16));
        let back: Vec<i16> = r.into_samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(back, vec![0, 16383, -16383, i16::MAX]);
    }

    #[test]
    fn nettoyage_hallucinations() {
        assert_eq!(clean_transcript(" Sous-titres réalisés par la communauté d'Amara.org"), "");
        assert_eq!(clean_transcript("Merci d'avoir regardé cette vidéo !"), "");
        assert_eq!(clean_transcript("[Musique]"), "");
        assert_eq!(clean_transcript("(musique) ♪ ♪"), "");
        assert_eq!(clean_transcript(" ..."), "");
        assert_eq!(
            clean_transcript(" Bonjour,  ceci est un test. Sous-titrage ST' 501"),
            "Bonjour, ceci est un test."
        );
        assert_eq!(
            clean_transcript("[BLANK_AUDIO] Il a dit (en riant) que c'était bon. *rires*"),
            "Il a dit (en riant) que c'était bon."
        );
        assert_eq!(clean_transcript("Version 2.0 prête ? Oui !"), "Version 2.0 prête ? Oui !");
    }

    #[test]
    fn jetons_speciaux_retires() {
        assert_eq!(clean_transcript("<|0.00|> Bonjour Maître.<|2.40|><|2.40|> Je vous écris.<|5.12|>"), "Bonjour Maître. Je vous écris.");
        assert_eq!(clean_transcript("<|fr|><|transcribe|> Bonjour"), "Bonjour");
        assert_eq!(clean_transcript("a <| b |> c"), "a <| b |> c");
    }

    #[test]
    fn options_effectives() {
        let o = TranscribeOptions { language: Some("auto".into()), initial_prompt: Some("  ".into()) };
        assert_eq!(o.language_code(), None);
        assert_eq!(o.prompt(), None);
        let o = TranscribeOptions { language: Some("fr".into()), initial_prompt: Some("Murmure".into()) };
        assert_eq!(o.language_code(), Some("fr"));
        assert_eq!(o.prompt(), Some("Murmure"));
    }
}

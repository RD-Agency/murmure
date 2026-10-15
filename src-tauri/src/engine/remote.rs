// SPDX-License-Identifier: AGPL-3.0-or-later
//! Moteur serveur : `POST /v1/audio/transcriptions` compatible OpenAI (OpenAI, Groq,
//! notre serveur faster-whisper…), authentification Bearer.

use super::{clean_transcript, encode_wav, TranscribeOptions};
use anyhow::{anyhow, bail};
use reqwest::multipart::{Form, Part};
use reqwest::StatusCode;
use serde::Deserialize;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TRANSCRIPTIONS_PATH: &str = "/audio/transcriptions";

#[derive(Clone, Debug)]
pub struct RemoteEngine {
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: String,
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

impl RemoteEngine {
    /// Envoie l'audio au serveur et renvoie le texte nettoyé.
    pub async fn transcribe(&self, samples: &[f32], opts: &TranscribeOptions) -> anyhow::Result<String> {
        let raw = self.request(samples, opts).await?;
        Ok(clean_transcript(&raw))
    }

    /// Vérifie la connexion en envoyant 1 s de silence.
    pub async fn test(&self) -> anyhow::Result<String> {
        let started = Instant::now();
        self.request(&vec![0.0; 16_000], &TranscribeOptions::default()).await?;
        Ok(format!("Connexion OK ({} ms)", started.elapsed().as_millis()))
    }

    async fn request(&self, samples: &[f32], opts: &TranscribeOptions) -> anyhow::Result<String> {
        let url = normalize_endpoint(&self.endpoint)?;
        let model = if self.model.trim().is_empty() { "whisper-large-v3-turbo" } else { self.model.trim() };

        let file = Part::bytes(encode_wav(samples))
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| anyhow!("Préparation de la requête : {e}"))?;
        let mut form = Form::new()
            .part("file", file)
            .text("model", model.to_owned())
            .text("response_format", "json");
        if let Some(lang) = opts.language_code() {
            form = form.text("language", lang.to_owned());
        }
        if let Some(prompt) = opts.prompt() {
            form = form.text("prompt", prompt.to_owned());
        }

        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("Murmure/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| anyhow!("Client HTTP : {e}"))?;
        let mut req = client.post(&url).multipart(form);
        if let Some(key) = self.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await.map_err(|e| network_error(&url, &e))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| network_error(&url, &e))?;
        if !status.is_success() {
            bail!("{}", http_error(status, &body));
        }
        serde_json::from_str::<TranscriptionResponse>(&body)
            .map(|r| r.text)
            .map_err(|_| anyhow!("Réponse inattendue du serveur (JSON sans champ « text »)"))
    }
}

/// Complète l'endpoint : une simple base (`https://x.fr`, `https://x.fr/v1`,
/// `https://api.groq.com/openai/v1`) devient `…/v1/audio/transcriptions`.
pub(crate) fn normalize_endpoint(raw: &str) -> anyhow::Result<String> {
    let mut url = raw.trim().trim_end_matches('/').to_owned();
    if url.is_empty() {
        bail!("Aucun endpoint serveur configuré");
    }
    if !url.contains("://") {
        url = format!("https://{url}");
    }
    let parsed = reqwest::Url::parse(&url).map_err(|_| anyhow!("Endpoint invalide : {raw}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("Endpoint invalide (http ou https attendu) : {raw}");
    }
    if url.ends_with(TRANSCRIPTIONS_PATH) {
        return Ok(url);
    }
    if url.ends_with("/v1") {
        return Ok(format!("{url}{TRANSCRIPTIONS_PATH}"));
    }
    Ok(format!("{url}/v1{TRANSCRIPTIONS_PATH}"))
}

fn network_error(url: &str, e: &reqwest::Error) -> anyhow::Error {
    if e.is_timeout() {
        anyhow!("Le serveur ne répond pas (délai de {} s dépassé)", TIMEOUT.as_secs())
    } else if e.is_connect() {
        anyhow!("Connexion impossible à {url} (serveur injoignable ou hors ligne)")
    } else {
        anyhow!("Erreur réseau : {e}")
    }
}

/// Message lisible à partir d'un statut HTTP et du corps d'erreur (OpenAI, Groq, FastAPI).
fn http_error(status: StatusCode, body: &str) -> String {
    let detail = extract_error_message(body);
    let base = match status.as_u16() {
        401 => "Clé API refusée".to_owned(),
        403 => "Accès refusé par le serveur".to_owned(),
        404 => "Endpoint introuvable (vérifiez l'URL)".to_owned(),
        413 => "Enregistrement trop volumineux pour le serveur".to_owned(),
        429 => "Trop de requêtes ou quota dépassé".to_owned(),
        400 | 422 => "Requête refusée par le serveur".to_owned(),
        s if s >= 500 => format!("Erreur du serveur (HTTP {s})"),
        s => format!("Réponse inattendue du serveur (HTTP {s})"),
    };
    match detail {
        Some(d) => format!("{base} : {d}"),
        None => base,
    }
}

fn extract_error_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let msg = v
        .pointer("/error/message")
        .or_else(|| v.get("detail"))
        .or_else(|| v.get("error"))
        .or_else(|| v.get("message"))?;
    let s = match msg {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let s = s.trim();
    (!s.is_empty()).then(|| s.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_endpoint() {
        let full = "https://x.fr/v1/audio/transcriptions";
        assert_eq!(normalize_endpoint("https://x.fr").unwrap(), full);
        assert_eq!(normalize_endpoint("https://x.fr/").unwrap(), full);
        assert_eq!(normalize_endpoint("https://x.fr/v1").unwrap(), full);
        assert_eq!(normalize_endpoint(" https://x.fr/v1/ ").unwrap(), full);
        assert_eq!(normalize_endpoint(full).unwrap(), full);
        assert_eq!(normalize_endpoint("x.fr").unwrap(), full);
        assert_eq!(
            normalize_endpoint("https://api.groq.com/openai/v1").unwrap(),
            "https://api.groq.com/openai/v1/audio/transcriptions"
        );
        assert_eq!(
            normalize_endpoint("http://localhost:8000").unwrap(),
            "http://localhost:8000/v1/audio/transcriptions"
        );
        assert!(normalize_endpoint("").is_err());
        assert!(normalize_endpoint("ftp://x.fr").is_err());
    }

    #[test]
    fn messages_erreur() {
        let openai = r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#;
        assert_eq!(http_error(StatusCode::UNAUTHORIZED, openai), "Clé API refusée : Incorrect API key provided");
        assert_eq!(http_error(StatusCode::NOT_FOUND, "<html>"), "Endpoint introuvable (vérifiez l'URL)");
        assert_eq!(http_error(StatusCode::UNAUTHORIZED, r#"{"detail":"Invalid token"}"#), "Clé API refusée : Invalid token");
    }

    #[tokio::test]
    async fn serveur_injoignable() {
        let e = RemoteEngine { endpoint: "http://127.0.0.1:9".into(), api_key: None, model: String::new() };
        let err = e.test().await.unwrap_err().to_string();
        assert!(err.contains("Connexion impossible") || err.contains("réseau"), "{err}");
    }

    /// Bout en bout contre un serveur réel (ex. `server/` lancé en local) :
    /// MURMURE_E2E_URL=http://127.0.0.1:8000 MURMURE_E2E_KEY=mrm_… MURMURE_TEST_WAV=t.wav \
    ///   cargo test --lib engine::remote -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn bout_en_bout_serveur() {
        let url = std::env::var("MURMURE_E2E_URL").expect("MURMURE_E2E_URL");
        let key = std::env::var("MURMURE_E2E_KEY").ok();
        let wav = std::env::var("MURMURE_TEST_WAV").expect("MURMURE_TEST_WAV");
        let mut reader = hound::WavReader::open(wav).expect("wav");
        let samples: Vec<f32> = reader.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
        let e = RemoteEngine { endpoint: url, api_key: key, model: "whisper-large-v3-turbo".into() };
        println!("test : {}", e.test().await.expect("test"));
        let opts = TranscribeOptions { language: Some("fr".into()), initial_prompt: None };
        let t0 = std::time::Instant::now();
        let text = e.transcribe(&samples, &opts).await.expect("transcribe");
        println!("texte : {text} ({} ms)", t0.elapsed().as_millis());
        assert!(text.to_lowercase().contains("dictée"));
        let bad = RemoteEngine { api_key: Some("mrm_mauvaise_cle_0000000000".into()), ..e };
        println!("mauvaise clé : {}", bad.test().await.unwrap_err());
    }
}

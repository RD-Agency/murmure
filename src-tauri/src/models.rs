//! Catalogue et téléchargement des modèles ggml de whisper.cpp (Hugging Face).
//!
//! - Dossier propre à Murmure : `<data_dir>/models/ggml-<id>.bin`.
//! - Un modèle déjà présent dans `~/.whisper/models/` (installé par un autre outil) est
//!   réutilisé tel quel : il est vu comme téléchargé, mais `delete` n'y touche jamais.
//! - Téléchargement en streaming vers `.part` puis rename ; reprise HTTP (Range) si un
//!   `.part` subsiste après une erreur réseau ; annulation via `AtomicBool`.

use anyhow::{anyhow, bail, Context};
use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

#[derive(Clone, Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub size_mb: u64,
    pub downloaded: bool,
    pub recommended: bool,
    pub path: Option<String>,
}

/// Entrée du catalogue (tailles exactes relevées sur l'API Hugging Face).
struct CatalogEntry {
    id: &'static str,
    file: &'static str,
    name: &'static str,
    description: &'static str,
    size_bytes: u64,
    recommended: bool,
}

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "large-v3-turbo-q5_0",
        file: "ggml-large-v3-turbo-q5_0.bin",
        name: "Whisper large-v3 turbo (Q5)",
        description: "Le meilleur compromis : précision de large-v3, rapide, léger en mémoire.",
        size_bytes: 574_041_195,
        recommended: true,
    },
    CatalogEntry {
        id: "large-v3-turbo-q8_0",
        file: "ggml-large-v3-turbo-q8_0.bin",
        name: "Whisper large-v3 turbo (Q8)",
        description: "Quantification plus fine, précision quasi identique au modèle complet.",
        size_bytes: 874_188_075,
        recommended: false,
    },
    CatalogEntry {
        id: "large-v3-turbo",
        file: "ggml-large-v3-turbo.bin",
        name: "Whisper large-v3 turbo (complet)",
        description: "Modèle non quantifié (F16). Précision maximale, plus lourd.",
        size_bytes: 1_624_555_275,
        recommended: false,
    },
    CatalogEntry {
        id: "small-q5_1",
        file: "ggml-small-q5_1.bin",
        name: "Whisper small (Q5)",
        description: "Pour les machines modestes : très léger, moins précis.",
        size_bytes: 190_085_487,
        recommended: false,
    },
];

fn entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

fn size_mb(bytes: u64) -> u64 {
    (bytes + 524_288) / 1_048_576
}

pub fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

/// Dossier partagé `~/.whisper/models` (whisper.cpp et autres outils).
fn external_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".whisper").join("models"))
}

/// Un fichier modèle est jugé exploitable s'il existe et n'est pas manifestement tronqué.
fn usable(path: &Path, e: &CatalogEntry) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.len() >= e.size_bytes / 10 * 9)
        .unwrap_or(false)
}

fn own_path(data_dir: &Path, e: &CatalogEntry) -> PathBuf {
    models_dir(data_dir).join(e.file)
}

fn part_path(data_dir: &Path, e: &CatalogEntry) -> PathBuf {
    models_dir(data_dir).join(format!("{}.part", e.file))
}

fn find(data_dir: &Path, e: &CatalogEntry) -> Option<PathBuf> {
    let own = own_path(data_dir, e);
    if usable(&own, e) {
        return Some(own);
    }
    external_dir().map(|d| d.join(e.file)).filter(|p| usable(p, e))
}

pub fn list(data_dir: &Path) -> Vec<ModelInfo> {
    CATALOG
        .iter()
        .map(|e| {
            let path = find(data_dir, e);
            ModelInfo {
                id: e.id.into(),
                name: e.name.into(),
                description: e.description.into(),
                size_mb: size_mb(e.size_bytes),
                downloaded: path.is_some(),
                recommended: e.recommended,
                path: path.map(|p| p.to_string_lossy().into_owned()),
            }
        })
        .collect()
}

/// Chemin du modèle s'il est présent sur disque (dossier Murmure ou `~/.whisper/models`).
pub fn model_path(data_dir: &Path, id: &str) -> Option<PathBuf> {
    entry(id).and_then(|e| find(data_dir, e))
}

/// Télécharge le modèle `id`. `on_progress(téléchargé, total)` est appelé au plus ~10 fois/s.
pub async fn download(
    data_dir: &Path,
    id: &str,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(u64, u64) + Send + 'static,
) -> anyhow::Result<PathBuf> {
    let e = entry(id).ok_or_else(|| anyhow!("Modèle inconnu : {id}"))?;
    if let Some(p) = find(data_dir, e) {
        on_progress(e.size_bytes, e.size_bytes);
        return Ok(p);
    }
    let dir = models_dir(data_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("Impossible de créer {}", dir.display()))?;
    let dest = own_path(data_dir, e);
    let part = part_path(data_dir, e);

    let result = fetch(e, &part, &cancel, &on_progress).await;
    match result {
        Ok(()) => {
            tokio::fs::rename(&part, &dest)
                .await
                .with_context(|| format!("Impossible de finaliser {}", dest.display()))?;
            log::info!("Modèle {id} téléchargé : {}", dest.display());
            Ok(dest)
        }
        Err(err) => {
            // Annulation ou fichier incohérent : on repart de zéro la prochaine fois.
            // Erreur réseau : on garde le .part pour reprendre (Range).
            if cancel.load(Ordering::Relaxed) || err.downcast_ref::<Corrupt>().is_some() {
                let _ = tokio::fs::remove_file(&part).await;
            }
            Err(err)
        }
    }
}

/// Erreur signalant un `.part` inexploitable (taille incohérente).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Corrupt(String);

async fn fetch(
    e: &CatalogEntry,
    part: &Path,
    cancel: &AtomicBool,
    on_progress: &(impl Fn(u64, u64) + Send),
) -> anyhow::Result<()> {
    let url = format!("{HF_BASE}/{}", e.file);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .user_agent(concat!("Murmure/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Client HTTP")?;

    let mut existing = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
    if existing >= e.size_bytes {
        // .part plus gros qu'attendu : inutilisable.
        let _ = tokio::fs::remove_file(part).await;
        existing = 0;
    }
    let mut req = client.get(&url);
    if existing > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={existing}-"));
    }
    let resp = req
        .send()
        .await
        .map_err(|err| anyhow!("Téléchargement impossible (réseau) : {err}"))?;
    let status = resp.status();
    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
    if !status.is_success() {
        bail!("Téléchargement refusé par Hugging Face (HTTP {})", status.as_u16());
    }
    if !resumed {
        existing = 0; // le serveur a ignoré le Range : on repart de zéro
    }
    let total = resp.content_length().map(|l| l + existing).unwrap_or(e.size_bytes);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(part)
        .await
        .with_context(|| format!("Impossible d'écrire {}", part.display()))?;

    let mut downloaded = existing;
    on_progress(downloaded, total);
    let mut last = Instant::now();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            bail!("Téléchargement annulé");
        }
        let chunk = chunk.map_err(|err| anyhow!("Téléchargement interrompu (réseau) : {err}"))?;
        file.write_all(&chunk).await.context("Écriture du modèle sur disque")?;
        downloaded += chunk.len() as u64;
        if last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            on_progress(downloaded, total);
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    if cancel.load(Ordering::Relaxed) {
        bail!("Téléchargement annulé");
    }
    if downloaded != total {
        if downloaded > total {
            return Err(Corrupt(format!("Fichier incohérent ({downloaded} octets reçus, {total} attendus)")).into());
        }
        bail!("Téléchargement incomplet ({downloaded} / {total} octets), relancez pour reprendre");
    }
    on_progress(downloaded, total);
    Ok(())
}

/// Supprime le modèle du dossier Murmure (et un éventuel `.part`).
/// Un modèle qui ne vit que dans `~/.whisper/models` n'est jamais supprimé.
pub fn delete(data_dir: &Path, id: &str) -> anyhow::Result<()> {
    let e = entry(id).ok_or_else(|| anyhow!("Modèle inconnu : {id}"))?;
    let own = own_path(data_dir, e);
    let _ = std::fs::remove_file(part_path(data_dir, e));
    if own.exists() {
        std::fs::remove_file(&own)
            .with_context(|| format!("Suppression impossible : {}", own.display()))?;
        return Ok(());
    }
    if let Some(ext) = find(data_dir, e) {
        bail!(
            "Ce modèle est partagé avec d'autres outils ({}) : Murmure ne le supprime pas.",
            ext.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_coherent() {
        let tmp = std::env::temp_dir().join(format!("murmure-models-{}", uuid::Uuid::new_v4()));
        let l = list(&tmp);
        assert_eq!(l.len(), CATALOG.len());
        assert_eq!(l.iter().filter(|m| m.recommended).count(), 1);
        let q5 = l.iter().find(|m| m.id == "large-v3-turbo-q5_0").unwrap();
        assert_eq!(q5.size_mb, 547);
        assert!(model_path(&tmp, "inconnu").is_none());
        assert_eq!(models_dir(&tmp), tmp.join("models"));
    }

    #[test]
    fn suppression_dans_notre_dossier() {
        let tmp = std::env::temp_dir().join(format!("murmure-models-{}", uuid::Uuid::new_v4()));
        let e = entry("small-q5_1").unwrap();
        std::fs::create_dir_all(models_dir(&tmp)).unwrap();
        let f = std::fs::File::create(own_path(&tmp, e)).unwrap();
        f.set_len(e.size_bytes).unwrap();
        assert_eq!(model_path(&tmp, "small-q5_1"), Some(own_path(&tmp, e)));
        delete(&tmp, "small-q5_1").unwrap();
        assert!(!own_path(&tmp, e).exists());
        std::fs::remove_dir_all(tmp).ok();
    }
}

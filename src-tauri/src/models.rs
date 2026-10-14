//! Catalogue et téléchargement des modèles ggml de whisper.cpp (Hugging Face).
//!
//! - Dossier propre à Murmure : `<data_dir>/models/ggml-<id>.bin`.
//! - Un modèle déjà présent dans `~/.whisper/models/` (installé par un autre outil) est
//!   réutilisé tel quel : il est vu comme téléchargé, mais `delete` n'y touche jamais.
//! - Téléchargement en streaming vers `.part` puis rename ; reprise HTTP (Range) si un
//!   `.part` subsiste après une erreur réseau ; annulation via `AtomicBool`.
//! - Avant : contrôle de l'espace disque. Pendant : SHA256 calculé au fil de l'eau (le début
//!   d'un `.part` repris est rehaché). Après : taille exacte + empreinte du catalogue.
//! - Erreur réseau : jusqu'à 3 nouvelles tentatives (5 s, 15 s, 45 s), avec reprise.

use anyhow::{anyhow, bail, Context};
use sha2::{Digest, Sha256};
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
    /// SHA256 du fichier (champ `lfs.oid` de l'API Hugging Face), vérifié en streaming.
    sha256: &'static str,
    recommended: bool,
}

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "large-v3-turbo-q5_0",
        file: "ggml-large-v3-turbo-q5_0.bin",
        name: "Whisper large-v3 turbo (Q5)",
        description: "Le meilleur compromis : précision de large-v3, rapide, léger en mémoire.",
        size_bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        recommended: true,
    },
    CatalogEntry {
        id: "large-v3-turbo-q8_0",
        file: "ggml-large-v3-turbo-q8_0.bin",
        name: "Whisper large-v3 turbo (Q8)",
        description: "Quantification plus fine, précision quasi identique au modèle complet.",
        size_bytes: 874_188_075,
        sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
        recommended: false,
    },
    CatalogEntry {
        id: "large-v3-turbo",
        file: "ggml-large-v3-turbo.bin",
        name: "Whisper large-v3 turbo (complet)",
        description: "Modèle non quantifié (F16). Précision maximale, plus lourd.",
        size_bytes: 1_624_555_275,
        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        recommended: false,
    },
    CatalogEntry {
        id: "small-q5_1",
        file: "ggml-small-q5_1.bin",
        name: "Whisper small (Q5)",
        description: "Pour les machines modestes : très léger, moins précis.",
        size_bytes: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
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

/// Id du modèle recommandé, utilisé quand l'id demandé est inconnu.
pub const RECOMMENDED_ID: &str = "large-v3-turbo-q5_0";

/// Délais d'attente avant chaque nouvelle tentative après une erreur réseau.
pub const RETRY_DELAYS: [Duration; 3] = [Duration::from_secs(5), Duration::from_secs(15), Duration::from_secs(45)];

/// Marge d'espace disque conservée en plus de ce qu'il reste à télécharger.
const DISK_MARGIN: u64 = 64 * 1_048_576;

/// Vrai si `id` figure au catalogue.
pub fn is_known(id: &str) -> bool {
    entry(id).is_some()
}

/// Id du catalogue correspondant à `id`, ou le modèle recommandé si `id` est inconnu.
pub fn resolve_id(id: &str) -> &'static str {
    entry(id).map(|e| e.id).unwrap_or(RECOMMENDED_ID)
}

/// Ids du catalogue, dans l'ordre d'affichage.
pub fn ids() -> impl Iterator<Item = &'static str> {
    CATALOG.iter().map(|e| e.id)
}

/// Taille exacte attendue du fichier, en octets.
pub fn expected_size(id: &str) -> Option<u64> {
    entry(id).map(|e| e.size_bytes)
}

/// Délai avant la tentative `attempt + 1` (0 = première relance), `None` quand on abandonne.
pub fn retry_delay(attempt: usize) -> Option<Duration> {
    RETRY_DELAYS.get(attempt).copied()
}

/// Erreur de téléchargement, classée pour décider d'une nouvelle tentative.
/// Toujours renvoyée enveloppée dans `anyhow::Error` (récupérable par `downcast_ref`).
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("Téléchargement annulé")]
    Cancelled,
    /// Coupure réseau, serveur indisponible : on réessaie en reprenant le `.part`.
    #[error("{0}")]
    Network(String),
    /// Fichier incohérent (taille, empreinte) : le `.part` est supprimé, on repart de zéro.
    #[error("{0}")]
    Corrupt(String),
    #[error("{0}")]
    NoSpace(String),
    #[error("{0}")]
    Fatal(String),
}

impl DownloadError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, DownloadError::Network(_) | DownloadError::Corrupt(_))
    }
}

/// Vrai si l'espace libre couvre ce qu'il reste à télécharger, marge comprise.
fn enough_space(needed: u64, available: u64) -> bool {
    available >= needed.saturating_add(DISK_MARGIN)
}

/// Vérifie l'espace disque disponible dans `dir` (création préalable du dossier requise).
fn check_space(dir: &Path, needed: u64) -> Result<(), DownloadError> {
    match fs4::available_space(dir) {
        Ok(available) if !enough_space(needed, available) => Err(DownloadError::NoSpace(format!(
            "Espace disque insuffisant : {} Mo nécessaires, {} Mo libres ({}). Libérez de la place puis réessayez.",
            size_mb(needed.saturating_add(DISK_MARGIN)),
            size_mb(available),
            dir.display()
        ))),
        Ok(_) => Ok(()),
        Err(err) => {
            log::warn!("espace disque de {} inconnu : {err}", dir.display());
            Ok(())
        }
    }
}

/// Contrôle final : taille exacte et empreinte SHA256 du catalogue.
fn verify(e: &CatalogEntry, downloaded: u64, sha256_hex: &str) -> Result<(), DownloadError> {
    if downloaded != e.size_bytes {
        return Err(DownloadError::Corrupt(format!(
            "Taille inattendue ({downloaded} octets reçus, {} attendus)",
            e.size_bytes
        )));
    }
    if !sha256_hex.eq_ignore_ascii_case(e.sha256) {
        return Err(DownloadError::Corrupt(format!(
            "Empreinte SHA256 invalide pour {} (fichier altéré pendant le transfert)",
            e.file
        )));
    }
    Ok(())
}

/// Attend `d` en surveillant l'annulation. Renvoie `false` si annulé.
async fn sleep_unless_cancelled(d: Duration, cancel: &AtomicBool) -> bool {
    let deadline = Instant::now() + d;
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200).min(deadline - Instant::now())).await;
    }
    !cancel.load(Ordering::Relaxed)
}

/// Télécharge le modèle `id`. `on_progress(téléchargé, total)` est appelé au plus ~10 fois/s.
///
/// Vérifie l'espace disque, reprend un `.part` existant, réessaie après une erreur réseau
/// ([`RETRY_DELAYS`]) et contrôle taille + SHA256 avant de publier le fichier.
/// Les erreurs sont des [`DownloadError`] enveloppées dans `anyhow::Error`.
pub async fn download(
    data_dir: &Path,
    id: &str,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(u64, u64) + Send + 'static,
) -> anyhow::Result<PathBuf> {
    let e = entry(id).ok_or_else(|| DownloadError::Fatal(format!("Modèle inconnu : {id}")))?;
    if let Some(p) = find(data_dir, e) {
        on_progress(e.size_bytes, e.size_bytes);
        return Ok(p);
    }
    let dir = models_dir(data_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|err| DownloadError::Fatal(format!("Impossible de créer {} : {err}", dir.display())))?;

    let mut attempt = 0;
    loop {
        let err = match download_once(data_dir, e, &cancel, &on_progress).await {
            Ok(p) => return Ok(p),
            Err(_) if cancel.load(Ordering::Relaxed) => return Err(DownloadError::Cancelled.into()),
            Err(err) => err,
        };
        let delay = if err.is_retryable() { retry_delay(attempt) } else { None };
        let Some(delay) = delay else {
            return Err(match err {
                DownloadError::Network(msg) if attempt > 0 => {
                    DownloadError::Network(format!("{msg} (abandon après {} tentatives)", attempt + 1))
                }
                other => other,
            }
            .into());
        };
        attempt += 1;
        log::warn!(
            "modèle {id} : {err} — nouvelle tentative {attempt}/{} dans {} s",
            RETRY_DELAYS.len(),
            delay.as_secs()
        );
        if !sleep_unless_cancelled(delay, &cancel).await {
            return Err(DownloadError::Cancelled.into());
        }
    }
}

/// Une tentative : `.part` (repris si possible) → vérification → rename.
async fn download_once(
    data_dir: &Path,
    e: &CatalogEntry,
    cancel: &AtomicBool,
    on_progress: &(impl Fn(u64, u64) + Send),
) -> Result<PathBuf, DownloadError> {
    let dest = own_path(data_dir, e);
    let part = part_path(data_dir, e);
    let existing = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0).min(e.size_bytes);
    check_space(&models_dir(data_dir), e.size_bytes - existing)?;

    match fetch(e, &part, cancel, on_progress).await {
        Ok(()) => {
            tokio::fs::rename(&part, &dest)
                .await
                .map_err(|err| DownloadError::Fatal(format!("Impossible de finaliser {} : {err}", dest.display())))?;
            log::info!("Modèle {} téléchargé et vérifié (SHA256) : {}", e.id, dest.display());
            Ok(dest)
        }
        Err(err) => {
            // Annulation ou fichier incohérent : on repart de zéro la prochaine fois.
            // Erreur réseau : on garde le .part pour reprendre (Range).
            if cancel.load(Ordering::Relaxed) || matches!(err, DownloadError::Corrupt(_) | DownloadError::Cancelled) {
                let _ = tokio::fs::remove_file(&part).await;
            }
            Err(err)
        }
    }
}

/// Ajoute au hacheur les `len` premiers octets de `path` (reprise d'un `.part`).
async fn hash_prefix(path: &Path, len: u64, hasher: &mut Sha256) -> std::io::Result<()> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await?;
    let mut reader = file.take(len);
    let mut buf = vec![0u8; 1 << 20];
    let mut read = 0u64;
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        read += n as u64;
    }
    if read != len {
        return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "fichier partiel tronqué"));
    }
    Ok(())
}

fn network(prefix: &str, err: impl std::fmt::Display) -> DownloadError {
    DownloadError::Network(format!("{prefix} : {err}"))
}

async fn fetch(
    e: &CatalogEntry,
    part: &Path,
    cancel: &AtomicBool,
    on_progress: &(impl Fn(u64, u64) + Send),
) -> Result<(), DownloadError> {
    let url = format!("{HF_BASE}/{}", e.file);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .user_agent(concat!("Murmure/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| DownloadError::Fatal(format!("Client HTTP : {err}")))?;

    let mut existing = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
    if existing >= e.size_bytes {
        // .part complet ou plus gros qu'attendu : on ne peut pas reprendre proprement.
        let _ = tokio::fs::remove_file(part).await;
        existing = 0;
    }
    // L'empreinte est calculée en streaming ; en reprise, on rehache d'abord le début déjà reçu.
    let mut hasher = Sha256::new();
    if existing > 0 && hash_prefix(part, existing, &mut hasher).await.is_err() {
        let _ = tokio::fs::remove_file(part).await;
        existing = 0;
        hasher = Sha256::new();
    }
    let mut req = client.get(&url);
    if existing > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={existing}-"));
    }
    let resp = req.send().await.map_err(|err| network("Téléchargement impossible (réseau)", err))?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        return Err(DownloadError::Corrupt("Reprise impossible, téléchargement relancé depuis le début".into()));
    }
    if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(DownloadError::Network(format!("Hugging Face indisponible (HTTP {})", status.as_u16())));
    }
    if !status.is_success() {
        return Err(DownloadError::Fatal(format!("Téléchargement refusé par Hugging Face (HTTP {})", status.as_u16())));
    }
    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
    if !resumed && existing > 0 {
        // Le serveur a ignoré le Range : on repart de zéro.
        existing = 0;
        hasher = Sha256::new();
    }
    let total = resp.content_length().map(|l| l + existing).unwrap_or(e.size_bytes);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(part)
        .await
        .map_err(|err| DownloadError::Fatal(format!("Impossible d'écrire {} : {err}", part.display())))?;

    let mut downloaded = existing;
    on_progress(downloaded, total);
    let mut last = Instant::now();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }
        let chunk = chunk.map_err(|err| network("Téléchargement interrompu (réseau)", err))?;
        file.write_all(&chunk)
            .await
            .map_err(|err| DownloadError::Fatal(format!("Écriture du modèle sur disque : {err}")))?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        if downloaded > e.size_bytes {
            return Err(DownloadError::Corrupt(format!(
                "Fichier incohérent ({downloaded} octets reçus, {} attendus)",
                e.size_bytes
            )));
        }
        if last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            on_progress(downloaded, total);
        }
    }
    let disk = |err: std::io::Error| DownloadError::Fatal(format!("Écriture du modèle sur disque : {err}"));
    file.flush().await.map_err(disk)?;
    file.sync_all().await.map_err(disk)?;
    drop(file);
    if cancel.load(Ordering::Relaxed) {
        return Err(DownloadError::Cancelled);
    }
    if downloaded < e.size_bytes {
        return Err(DownloadError::Network(format!(
            "Téléchargement incomplet ({downloaded} / {} octets)",
            e.size_bytes
        )));
    }
    verify(e, downloaded, &format!("{:x}", hasher.finalize()))?;
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

    fn fake_entry(data: &[u8]) -> CatalogEntry {
        // Empreinte calculée ici, puis figée dans une chaîne 'static pour le test.
        let hex: &'static str = Box::leak(format!("{:x}", Sha256::digest(data)).into_boxed_str());
        CatalogEntry {
            id: "test",
            file: "ggml-test.bin",
            name: "Test",
            description: "",
            size_bytes: data.len() as u64,
            sha256: hex,
            recommended: false,
        }
    }

    #[test]
    fn catalogue_empreintes_bien_formees() {
        for e in CATALOG {
            assert_eq!(e.sha256.len(), 64, "{}", e.id);
            assert!(e.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", e.id);
        }
        assert!(is_known(RECOMMENDED_ID));
        assert_eq!(entry(RECOMMENDED_ID).map(|e| e.recommended), Some(true));
    }

    #[test]
    fn resolution_des_ids() {
        assert_eq!(resolve_id("small-q5_1"), "small-q5_1");
        assert_eq!(resolve_id("inconnu"), RECOMMENDED_ID);
        assert_eq!(resolve_id(""), RECOMMENDED_ID);
        assert_eq!(expected_size("small-q5_1"), Some(190_085_487));
        assert_eq!(ids().count(), CATALOG.len());
    }

    #[test]
    fn backoff_5_15_45_puis_abandon() {
        assert_eq!(retry_delay(0), Some(Duration::from_secs(5)));
        assert_eq!(retry_delay(1), Some(Duration::from_secs(15)));
        assert_eq!(retry_delay(2), Some(Duration::from_secs(45)));
        assert_eq!(retry_delay(3), None);
        assert!(DownloadError::Network("x".into()).is_retryable());
        assert!(DownloadError::Corrupt("x".into()).is_retryable());
        assert!(!DownloadError::Cancelled.is_retryable());
        assert!(!DownloadError::NoSpace("x".into()).is_retryable());
        assert!(!DownloadError::Fatal("x".into()).is_retryable());
    }

    #[test]
    fn espace_disque() {
        let mb = 1_048_576;
        assert!(enough_space(100 * mb, 100 * mb + DISK_MARGIN));
        assert!(!enough_space(100 * mb, 100 * mb + DISK_MARGIN - 1));
        assert!(!enough_space(u64::MAX, u64::MAX - 1));
        let tmp = std::env::temp_dir();
        assert!(matches!(check_space(&tmp, u64::MAX / 2), Err(DownloadError::NoSpace(_))));
        assert!(check_space(&tmp, 0).is_ok());
    }

    #[test]
    fn verification_sha256() {
        let data = b"murmure, modele de test";
        let e = fake_entry(data);
        let good = format!("{:x}", Sha256::digest(data));
        assert!(verify(&e, data.len() as u64, &good).is_ok());
        assert!(verify(&e, data.len() as u64, &good.to_uppercase()).is_ok());
        assert!(matches!(verify(&e, data.len() as u64 - 1, &good), Err(DownloadError::Corrupt(_))));
        let bad = format!("{:x}", Sha256::digest(b"autre chose"));
        assert!(matches!(verify(&e, data.len() as u64, &bad), Err(DownloadError::Corrupt(_))));
    }

    #[tokio::test]
    async fn sha256_en_streaming_avec_reprise() {
        // Reprise : début rehaché depuis le .part, suite ajoutée au fil de l'eau.
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i * 31 % 251) as u8).collect();
        let tmp = std::env::temp_dir().join(format!("murmure-sha-{}", uuid::Uuid::new_v4()));
        let split = 1_234_567;
        std::fs::write(&tmp, &data[..split]).unwrap();
        let mut h = Sha256::new();
        hash_prefix(&tmp, split as u64, &mut h).await.unwrap();
        h.update(&data[split..]);
        let e = fake_entry(&data);
        assert!(verify(&e, data.len() as u64, &format!("{:x}", h.finalize())).is_ok());
        // Préfixe plus long que le fichier : erreur (le .part sera jeté).
        let mut h2 = Sha256::new();
        assert!(hash_prefix(&tmp, split as u64 + 1, &mut h2).await.is_err());
        std::fs::remove_file(tmp).ok();
    }

    #[tokio::test]
    async fn attente_interrompue_par_annulation() {
        let cancel = AtomicBool::new(true);
        let t = Instant::now();
        assert!(!sleep_unless_cancelled(Duration::from_secs(30), &cancel).await);
        assert!(t.elapsed() < Duration::from_secs(1));
        let go = AtomicBool::new(false);
        assert!(sleep_unless_cancelled(Duration::from_millis(10), &go).await);
    }

    #[tokio::test]
    async fn modele_inconnu_non_reessaye() {
        let tmp = std::env::temp_dir().join(format!("murmure-models-{}", uuid::Uuid::new_v4()));
        let err = download(&tmp, "inconnu", Arc::new(AtomicBool::new(false)), |_, _| {}).await.unwrap_err();
        assert!(matches!(err.downcast_ref::<DownloadError>(), Some(DownloadError::Fatal(_))));
    }
}

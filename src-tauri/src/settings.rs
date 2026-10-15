// SPDX-License-Identifier: AGPL-3.0-or-later
//! Réglages persistants de Murmure.
//!
//! - `settings.json` dans le dossier de config de l'app (écriture atomique tmp + rename) ;
//! - la clé API du moteur serveur vit dans le trousseau système, jamais dans le JSON.

use crate::correction::Replacement;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Nom du fichier de réglages dans le dossier de config.
const SETTINGS_FILE: &str = "settings.json";
/// Service et compte du trousseau pour la clé API.
const KEYRING_SERVICE: &str = "fr.rdagency.murmure";
const KEYRING_ACCOUNT: &str = "remote_api_key";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    Hybrid,
    Hold,
    Toggle,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PasteMode {
    Paste,
    Clipboard,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub engine: EngineKind,
    pub local_model: String,
    pub language: String,
    pub initial_prompt: String,
    /// « virgule », « point à la ligne »… convertis en signes (français).
    pub spoken_punctuation: bool,
    /// Textes de trois mots ou moins : ni point final ni majuscule imposée par Whisper.
    pub short_text_plain: bool,
    pub remote_endpoint: String,
    pub remote_model: String,
    pub hotkey: String,
    pub hotkey_mode: HotkeyMode,
    /// Bascule ou mains libres : silence (en ms) après lequel la dictée se termine ; 0 = jamais.
    pub auto_stop_silence_ms: u32,
    pub input_device: Option<String>,
    /// Baisse le volume de sortie du système pendant l'enregistrement.
    pub duck_volume: bool,
    pub paste_mode: PasteMode,
    pub restore_clipboard: bool,
    /// Mot de validation : prononcé en fin de dictée, il est retiré et Entrée suit le collage.
    pub submit_word_enabled: bool,
    pub submit_words: Vec<String>,
    pub sounds: bool,
    pub launch_at_login: bool,
    pub history_enabled: bool,
    /// Conserve l'audio de chaque dictée (Opus, environ 9 Mo par heure) dans l'historique.
    pub keep_audio: bool,
    /// Plafond de l'audio conservé, en Mo (5 Go au plus) : au-delà, les plus anciens sont supprimés.
    pub audio_cap_mb: u64,
    /// Corrige d'après le vocabulaire les mots que Whisper a mal reconnus (confiance faible).
    pub auto_correct: bool,
    /// Règles de remplacement appliquées au texte final.
    pub replacements: Vec<Replacement>,
    pub onboarding_done: bool,
    /// Linux Wayland : jeton de restauration du portail RemoteDesktop (collage), géré par Rust
    /// uniquement — la valeur envoyée par le front est ignorée par `save_settings`.
    pub wayland_restore_token: Option<String>,
}

/// Arrêt automatique après deux secondes de silence (bascule et mains libres).
pub const DEFAULT_AUTO_STOP_MS: u32 = 2000;

/// Mots de validation proposés par défaut.
pub fn default_submit_words() -> Vec<String> {
    vec!["terminé".into(), "envoyer".into()]
}

/// Raccourci par défaut : Option+Espace sur macOS, Ctrl+Alt+Espace ailleurs
/// (Alt+Espace ouvre le menu de fenêtre sous Windows et KRunner sous KDE).
fn default_hotkey() -> &'static str {
    if cfg!(target_os = "macos") {
        "Alt+Space"
    } else {
        "Ctrl+Alt+Space"
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: EngineKind::Local,
            local_model: "large-v3-turbo-q5_0".into(),
            language: "fr".into(),
            initial_prompt: String::new(),
            spoken_punctuation: true,
            short_text_plain: true,
            remote_endpoint: String::new(),
            remote_model: "whisper-large-v3-turbo".into(),
            hotkey: default_hotkey().into(),
            hotkey_mode: HotkeyMode::Hybrid,
            auto_stop_silence_ms: DEFAULT_AUTO_STOP_MS,
            input_device: None,
            duck_volume: false,
            paste_mode: PasteMode::Paste,
            restore_clipboard: true,
            submit_word_enabled: false,
            submit_words: default_submit_words(),
            sounds: true,
            launch_at_login: false,
            history_enabled: true,
            keep_audio: true,
            audio_cap_mb: crate::history::DEFAULT_AUDIO_CAP_MB,
            auto_correct: true,
            replacements: Vec::new(),
            onboarding_done: false,
            wayland_restore_token: None,
        }
    }
}

impl Settings {
    /// Plafond effectif de l'audio en octets (entre 100 Mo et 5 Go).
    pub fn audio_cap_bytes(&self) -> u64 {
        self.audio_cap_mb.clamp(100, crate::history::DEFAULT_AUDIO_CAP_MB) * 1024 * 1024
    }
}

fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE)
}

/// Charge les réglages. Fichier absent ou illisible → valeurs par défaut (jamais d'erreur).
pub fn load(config_dir: &Path) -> Settings {
    let path = settings_path(config_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
        Err(e) => {
            log::warn!("Réglages illisibles ({}) : {e} — valeurs par défaut", path.display());
            return Settings::default();
        }
    };
    match serde_json::from_str::<Settings>(&raw) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Réglages corrompus ({}) : {e} — valeurs par défaut", path.display());
            Settings::default()
        }
    }
}

/// Enregistre les réglages de façon atomique (fichier temporaire, fsync, puis rename).
pub fn save(config_dir: &Path, s: &Settings) -> anyhow::Result<()> {
    fs::create_dir_all(config_dir)
        .with_context(|| format!("Impossible de créer le dossier {}", config_dir.display()))?;
    let path = settings_path(config_dir);
    let tmp = config_dir.join(format!("{SETTINGS_FILE}.tmp"));
    let json = serde_json::to_vec_pretty(s).context("Sérialisation des réglages")?;
    {
        let mut f = fs::File::create(&tmp)
            .with_context(|| format!("Impossible d'écrire {}", tmp.display()))?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)
        .with_context(|| format!("Impossible de remplacer {}", path.display()))?;
    Ok(())
}

fn keyring_entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
}

/// Clé API du moteur serveur, si présente dans le trousseau (et non vide).
pub fn get_api_key() -> Option<String> {
    let entry = match keyring_entry() {
        Ok(e) => e,
        Err(e) => {
            log::warn!("Trousseau indisponible : {e}");
            return None;
        }
    };
    match entry.get_password() {
        Ok(k) if !k.trim().is_empty() => Some(k),
        Ok(_) | Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            log::warn!("Lecture de la clé API impossible : {e}");
            None
        }
    }
}

/// Enregistre la clé API dans le trousseau. Une chaîne vide supprime la clé.
pub fn set_api_key(key: &str) -> anyhow::Result<()> {
    let entry = keyring_entry().context("Trousseau système indisponible")?;
    let key = key.trim();
    if key.is_empty() {
        return match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Suppression de la clé API impossible : {e}")),
        };
    }
    entry
        .set_password(key)
        .map_err(|e| anyhow::anyhow!("Enregistrement de la clé API impossible : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmure-test-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defauts() {
        let s = Settings::default();
        assert_eq!(s.engine, EngineKind::Local);
        assert_eq!(s.local_model, "large-v3-turbo-q5_0");
        assert_eq!(s.language, "fr");
        assert_eq!(s.remote_model, "whisper-large-v3-turbo");
        assert_eq!(s.hotkey_mode, HotkeyMode::Hybrid);
        assert_eq!(s.paste_mode, PasteMode::Paste);
        assert!(s.restore_clipboard && s.sounds && s.history_enabled);
        assert!(s.keep_audio && s.auto_correct && s.replacements.is_empty());
        assert_eq!(s.audio_cap_bytes(), 5 * 1024 * 1024 * 1024);
        assert_eq!(Settings { audio_cap_mb: 1 << 40, ..Settings::default() }.audio_cap_bytes(), 5 << 30);
        assert_eq!(Settings { audio_cap_mb: 0, ..Settings::default() }.audio_cap_bytes(), 100 << 20);
        assert!(!s.launch_at_login && !s.onboarding_done);
        assert!(s.input_device.is_none());
        assert!(s.wayland_restore_token.is_none());
        assert!(s.initial_prompt.is_empty() && s.remote_endpoint.is_empty());
        assert!(s.short_text_plain);
        assert_eq!(s.auto_stop_silence_ms, 2000);
        assert!(!s.submit_word_enabled && !s.duck_volume);
        assert_eq!(s.submit_words, ["terminé", "envoyer"]);
        if cfg!(target_os = "macos") {
            assert_eq!(s.hotkey, "Alt+Space");
        } else {
            assert_eq!(s.hotkey, "Ctrl+Alt+Space");
        }
    }

    #[test]
    fn aller_retour() {
        let dir = temp_dir("roundtrip");
        let s = Settings {
            engine: EngineKind::Remote,
            remote_endpoint: "https://stt.exemple.fr".into(),
            input_device: Some("MacBook Pro Microphone".into()),
            hotkey_mode: HotkeyMode::Toggle,
            onboarding_done: true,
            ..Settings::default()
        };
        save(&dir, &s).unwrap();
        assert!(!dir.join("settings.json.tmp").exists());
        let l = load(&dir);
        assert_eq!(l.engine, EngineKind::Remote);
        assert_eq!(l.remote_endpoint, "https://stt.exemple.fr");
        assert_eq!(l.input_device.as_deref(), Some("MacBook Pro Microphone"));
        assert_eq!(l.hotkey_mode, HotkeyMode::Toggle);
        assert!(l.onboarding_done);
        // Format JSON conforme au contrat (snake_case).
        let raw = fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(raw.contains("\"engine\": \"remote\""));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn absent_corrompu_ou_partiel() {
        let dir = temp_dir("corrupt");
        assert_eq!(load(&dir).local_model, "large-v3-turbo-q5_0");
        fs::write(dir.join("settings.json"), "{ pas du json").unwrap();
        assert_eq!(load(&dir).language, "fr");
        // Champs manquants → défauts champ par champ.
        fs::write(dir.join("settings.json"), r#"{"language":"en"}"#).unwrap();
        let s = load(&dir);
        assert_eq!(s.language, "en");
        assert!(s.sounds);
        // Fichier d'une version précédente : les nouveaux réglages prennent leur valeur par défaut.
        assert!(s.short_text_plain && !s.submit_word_enabled);
        assert_eq!(s.auto_stop_silence_ms, DEFAULT_AUTO_STOP_MS);
        fs::remove_dir_all(dir).ok();
    }
}

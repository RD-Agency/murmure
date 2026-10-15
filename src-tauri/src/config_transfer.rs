// SPDX-License-Identifier: AGPL-3.0-or-later
//! Export et import des réglages dans un fichier `.json` versionné, pour équiper plusieurs postes.
//!
//! ```jsonc
//! {
//!   "format": "murmure-config",
//!   "version": 1,
//!   "app_version": "0.1.6",
//!   "exported_at": "2026-10-15T21:00:00+02:00",
//!   "settings": { "engine": "remote", "language": "fr", … },
//!   "secrets": { "remote_api_key": "…" }          // seulement si demandé explicitement
//! }
//! ```
//!
//! - Ne sont jamais exportés : le jeton du portail Wayland et l'état de l'assistant, propres au poste.
//! - La clé API (trousseau) n'est exportée qu'avec l'option explicite ; le fichier est alors créé en
//!   lecture seule pour son propriétaire (Unix).
//! - À l'import, seuls les champs présents remplacent les réglages actuels ; un champ inconnu
//!   (version plus récente) est ignoré, une valeur invalide fait échouer l'import sans rien changer.
//!
//! En ligne de commande, avant le démarrage de Tauri : `murmure --export-config fichier.json
//! [--with-secrets]` et `murmure --import-config fichier.json`.

use crate::settings::{self, Settings};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "murmure-config";
/// Version du format ; un fichier d'une version supérieure est refusé.
pub const VERSION: u32 = 1;
pub const EXPORT_ARG: &str = "--export-config";
pub const IMPORT_ARG: &str = "--import-config";
pub const WITH_SECRETS_ARG: &str = "--with-secrets";
/// Nom proposé dans la boîte de dialogue d'export.
pub const DEFAULT_FILE_NAME: &str = "murmure-reglages.json";

/// Réglages propres au poste : ni exportés, ni remplacés à l'import.
const LOCAL_ONLY: &[&str] = &["wayland_restore_token", "onboarding_done"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Secrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_api_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigFile {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub exported_at: String,
    pub settings: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Secrets>,
}

/// Résultat d'un import : réglages fusionnés, et clé API à ranger si le fichier en contient une.
#[derive(Clone, Debug)]
pub struct Imported {
    pub settings: Settings,
    pub api_key: Option<String>,
}

/// Construit le fichier d'export. `api_key` n'est inclus que s'il est fourni (option explicite).
pub fn export(s: &Settings, api_key: Option<&str>) -> ConfigFile {
    let mut map = match serde_json::to_value(s) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    for key in LOCAL_ONLY {
        map.remove(*key);
    }
    let secrets = api_key
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| Secrets { remote_api_key: Some(k.to_string()) });
    ConfigFile {
        format: FORMAT.into(),
        version: VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        exported_at: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        settings: map,
        secrets,
    }
}

/// Lit et vérifie un fichier de réglages.
pub fn parse(raw: &str) -> Result<ConfigFile, String> {
    let value: Value = serde_json::from_str(raw).map_err(|e| format!("Fichier illisible : ce n'est pas du JSON valide ({e})."))?;
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err("Ce fichier n'est pas un export de réglages de Murmure.".into());
    }
    let file: ConfigFile = serde_json::from_value(value).map_err(|e| format!("Fichier de réglages incomplet : {e}."))?;
    if file.version == 0 || file.version > VERSION {
        return Err(format!(
            "Ce fichier vient d'une version plus récente de Murmure (format {}) : mettez Murmure à jour.",
            file.version
        ));
    }
    Ok(file)
}

/// Applique le fichier par-dessus les réglages actuels.
pub fn merge(current: &Settings, file: &ConfigFile) -> Result<Imported, String> {
    let mut map = match serde_json::to_value(current) {
        Ok(Value::Object(m)) => m,
        _ => return Err("Réglages actuels illisibles.".into()),
    };
    for (key, value) in &file.settings {
        if LOCAL_ONLY.contains(&key.as_str()) || !map.contains_key(key) {
            continue;
        }
        map.insert(key.clone(), value.clone());
    }
    let settings: Settings =
        serde_json::from_value(Value::Object(map)).map_err(|e| format!("Valeur invalide dans le fichier : {e}."))?;
    crate::hotkey::validate(&settings.hotkey)?;
    let api_key = file.secrets.as_ref().and_then(|s| s.remote_api_key.clone()).filter(|k| !k.trim().is_empty());
    Ok(Imported { settings, api_key })
}

/// Écrit le fichier (atomique ; droits 600 sous Unix s'il contient un secret).
pub fn write(path: &Path, file: &ConfigFile) -> Result<(), String> {
    let mut json = serde_json::to_vec_pretty(file).map_err(|e| e.to_string())?;
    json.push(b'\n');
    let tmp = path.with_extension("json.tmp");
    let fail = |e: std::io::Error| format!("Impossible d'écrire {} : {e}", path.display());
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if file.secrets.is_some() {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(fail)?;
        f.write_all(&json).map_err(fail)?;
        f.sync_all().map_err(fail)?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(e)
    })
}

pub fn read(path: &Path) -> Result<ConfigFile, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("Impossible de lire {} : {e}", path.display()))?;
    parse(&raw)
}

// ───────────────────────────── Ligne de commande ─────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Export { path: PathBuf, with_secrets: bool },
    Import { path: PathBuf },
    Usage(String),
}

fn parse_args(args: &[String]) -> Option<Cli> {
    let value_of = |flag: &str| -> Option<Option<String>> {
        let prefix = format!("{flag}=");
        args.iter().enumerate().find_map(|(i, a)| {
            if a == flag {
                Some(args.get(i + 1).filter(|v| !v.starts_with("--")).cloned())
            } else {
                a.strip_prefix(&prefix).map(|v| Some(v.to_string()).filter(|v| !v.is_empty()))
            }
        })
    };
    if let Some(path) = value_of(EXPORT_ARG) {
        return Some(match path {
            Some(p) => Cli::Export { path: p.into(), with_secrets: args.iter().any(|a| a == WITH_SECRETS_ARG) },
            None => Cli::Usage(format!("Usage : murmure {EXPORT_ARG} fichier.json [{WITH_SECRETS_ARG}]")),
        });
    }
    value_of(IMPORT_ARG).map(|path| match path {
        Some(p) => Cli::Import { path: p.into() },
        None => Cli::Usage(format!("Usage : murmure {IMPORT_ARG} fichier.json")),
    })
}

/// `murmure --export-config` / `--import-config` : traité avant Tauri. `None` si non concerné,
/// sinon le code de sortie (0 succès, 1 échec, 2 usage).
pub fn cli(args: &[String]) -> Option<i32> {
    let cmd = parse_args(args)?;
    let Some(config_dir) = dirs::config_dir().map(|d| d.join(crate::install::IDENTIFIER)) else {
        eprintln!("Dossier de configuration introuvable.");
        return Some(1);
    };
    Some(match cmd {
        Cli::Usage(msg) => {
            eprintln!("{msg}");
            2
        }
        Cli::Export { path, with_secrets } => {
            let current = settings::load(&config_dir);
            let key = if with_secrets { settings::get_api_key() } else { None };
            let file = export(&current, key.as_deref());
            match write(&path, &file) {
                Ok(()) => {
                    println!("Réglages exportés : {}", path.display());
                    if file.secrets.is_some() {
                        println!("Le fichier contient la clé API du serveur : conservez-le en lieu sûr.");
                    } else if with_secrets {
                        println!("Aucune clé API enregistrée : rien de secret n'a été exporté.");
                    }
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        Cli::Import { path } => {
            let current = settings::load(&config_dir);
            let imported = match read(&path).and_then(|f| merge(&current, &f)) {
                Ok(i) => i,
                Err(e) => {
                    eprintln!("{e}");
                    return Some(1);
                }
            };
            if let Err(e) = settings::save(&config_dir, &imported.settings) {
                eprintln!("Impossible d'enregistrer les réglages : {e}");
                return Some(1);
            }
            println!("Réglages importés dans {}", config_dir.display());
            if let Some(key) = imported.api_key {
                match settings::set_api_key(&key) {
                    Ok(()) => println!("Clé API du serveur enregistrée dans le trousseau."),
                    Err(e) => {
                        eprintln!("Clé API non enregistrée : {e}");
                        return Some(1);
                    }
                }
            }
            println!("Si Murmure est ouvert, quittez-le et relancez-le pour appliquer les réglages.");
            0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{EngineKind, HotkeyMode};

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("murmure").chain(list.iter().copied()).map(String::from).collect()
    }

    fn sample() -> Settings {
        Settings {
            engine: EngineKind::Remote,
            remote_endpoint: "https://stt.exemple.fr/v1/audio/transcriptions".into(),
            hotkey_mode: HotkeyMode::Toggle,
            submit_word_enabled: true,
            onboarding_done: true,
            wayland_restore_token: Some("jeton".into()),
            ..Settings::default()
        }
    }

    #[test]
    fn export_sans_secret_ni_reglage_local() {
        let file = export(&sample(), None);
        let json = serde_json::to_string(&file).unwrap();
        assert_eq!(file.format, FORMAT);
        assert_eq!(file.version, VERSION);
        assert!(file.secrets.is_none() && !json.contains("secrets"));
        assert!(!json.contains("jeton") && !json.contains("wayland_restore_token") && !json.contains("onboarding_done"));
        assert_eq!(file.settings["engine"], "remote");
        assert_eq!(file.settings["submit_word_enabled"], true);
    }

    #[test]
    fn export_avec_secret_explicite() {
        let file = export(&sample(), Some(" sk-123 "));
        assert_eq!(file.secrets, Some(Secrets { remote_api_key: Some("sk-123".into()) }));
        // Clé vide : pas de bloc secrets.
        assert!(export(&sample(), Some("  ")).secrets.is_none());
    }

    #[test]
    fn aller_retour_sur_un_autre_poste() {
        let raw = serde_json::to_string_pretty(&export(&sample(), Some("sk-123"))).unwrap();
        let other = Settings { onboarding_done: false, wayland_restore_token: Some("autre".into()), ..Settings::default() };
        let imported = merge(&other, &parse(&raw).unwrap()).unwrap();
        assert_eq!(imported.settings.engine, EngineKind::Remote);
        assert_eq!(imported.settings.hotkey_mode, HotkeyMode::Toggle);
        assert!(imported.settings.submit_word_enabled);
        assert_eq!(imported.api_key.as_deref(), Some("sk-123"));
        // Propres au poste : conservés.
        assert!(!imported.settings.onboarding_done);
        assert_eq!(imported.settings.wayland_restore_token.as_deref(), Some("autre"));
    }

    #[test]
    fn import_partiel_et_champs_inconnus() {
        let raw = r#"{"format":"murmure-config","version":1,"settings":{"language":"en","reglage_futur":42,"onboarding_done":true}}"#;
        let current = Settings { sounds: false, ..Settings::default() };
        let imported = merge(&current, &parse(raw).unwrap()).unwrap();
        assert_eq!(imported.settings.language, "en");
        assert!(!imported.settings.sounds, "un champ absent garde la valeur actuelle");
        assert!(!imported.settings.onboarding_done);
        assert!(imported.api_key.is_none());
    }

    #[test]
    fn import_refuse() {
        assert!(parse("pas du json").unwrap_err().contains("JSON"));
        assert!(parse(r#"{"engine":"local"}"#).unwrap_err().contains("pas un export"));
        let future = r#"{"format":"murmure-config","version":99,"settings":{}}"#;
        assert!(parse(future).unwrap_err().contains("plus récente"));
        let bad = r#"{"format":"murmure-config","version":1,"settings":{"engine":"nuage"}}"#;
        assert!(merge(&Settings::default(), &parse(bad).unwrap()).unwrap_err().contains("Valeur invalide"));
        let bad_hotkey = r#"{"format":"murmure-config","version":1,"settings":{"hotkey":"Alt+Pomme"}}"#;
        assert!(merge(&Settings::default(), &parse(bad_hotkey).unwrap()).is_err());
    }

    #[test]
    fn ecriture_et_lecture() {
        let dir = std::env::temp_dir().join(format!("murmure-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("reglages.json");
        write(&path, &export(&sample(), Some("sk-123"))).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let back = read(&path).unwrap();
        assert_eq!(back.settings["remote_endpoint"], "https://stt.exemple.fr/v1/audio/transcriptions");
        assert!(!dir.join("reglages.json.tmp").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn arguments() {
        assert_eq!(parse_args(&args(&[])), None);
        assert_eq!(parse_args(&args(&["--hidden"])), None);
        assert_eq!(
            parse_args(&args(&["--export-config", "a.json"])),
            Some(Cli::Export { path: "a.json".into(), with_secrets: false })
        );
        assert_eq!(
            parse_args(&args(&["--export-config=a.json", "--with-secrets"])),
            Some(Cli::Export { path: "a.json".into(), with_secrets: true })
        );
        assert_eq!(parse_args(&args(&["--import-config", "b.json"])), Some(Cli::Import { path: "b.json".into() }));
        assert!(matches!(parse_args(&args(&["--import-config"])), Some(Cli::Usage(_))));
        assert!(matches!(parse_args(&args(&["--export-config", "--with-secrets"])), Some(Cli::Usage(_))));
    }
}

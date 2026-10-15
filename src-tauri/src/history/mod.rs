// SPDX-License-Identifier: AGPL-3.0-or-later
//! Historique local des dictées : base SQLite `history.sqlite3` dans le dossier de données.
//!
//! - Schéma versionné (`PRAGMA user_version`), migrations appliquées à l'ouverture.
//! - L'ancien `history.json` est importé sans perte au premier lancement, puis renommé en
//!   `history.json.migrated` (conservé comme sauvegarde).
//! - Recherche plein texte FTS5 (sans accents ni casse) sur le texte courant de chaque dictée.
//! - Audio facultatif : `audio/<id>.ogg` (Opus), plafonné ; au-delà, les audios les plus anciens
//!   sont supprimés, le texte est conservé.
//! - Corrections manuelles : paire avant/après et termes corrigés (diff mot à mot), d'où les
//!   propositions de vocabulaire.

pub mod audio;

use crate::correction::{self, AutoCorrection, TermChange};
use crate::settings::{EngineKind, Settings};
use anyhow::Context;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

const DB_FILE: &str = "history.sqlite3";
const LEGACY_FILE: &str = "history.json";
const AUDIO_DIR: &str = "audio";
/// Nombre de dictées renvoyées par `recent` (accueil, statistiques).
pub const RECENT_LIMIT: usize = 500;
/// Plafond par défaut de l'audio conservé : 5 Go.
pub const DEFAULT_AUDIO_CAP_MB: u64 = 5 * 1024;
/// Un terme corrigé au moins ce nombre de fois est proposé pour le vocabulaire.
pub const SUGGEST_AFTER: u32 = 2;

/// Migrations du schéma, dans l'ordre. L'indice + 1 est la version (`user_version`).
const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE dictations (
        seq              INTEGER PRIMARY KEY,         -- rowid stable (VACUUM), clé de l'index FTS
        id               TEXT NOT NULL UNIQUE,
        created_at       TEXT NOT NULL,               -- RFC 3339, heure locale
        created_ms       INTEGER NOT NULL,            -- époque Unix en ms, tri et filtre par date
        duration_ms      INTEGER NOT NULL,
        latency_ms       INTEGER NOT NULL,
        engine           TEXT NOT NULL,               -- local | remote
        model            TEXT NOT NULL,
        language         TEXT NOT NULL DEFAULT '',    -- code ISO 639-1, '' = auto ou inconnu
        raw_text         TEXT NOT NULL,               -- sortie du modèle, avant corrections et ponctuation
        final_text       TEXT NOT NULL,               -- texte collé
        corrected_text   TEXT,                        -- correction manuelle, NULL sinon
        corrected_at     TEXT,
        auto_corrections TEXT NOT NULL DEFAULT '[]',  -- JSON : [{from, to, probability}]
        audio_file       TEXT,                        -- nom du fichier dans audio/, NULL sans audio
        audio_bytes      INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX dictations_created ON dictations(created_ms);
    CREATE INDEX dictations_audio ON dictations(created_ms) WHERE audio_file IS NOT NULL;

    CREATE VIRTUAL TABLE dictations_fts USING fts5(text, tokenize = 'unicode61 remove_diacritics 2');
    CREATE TRIGGER dictations_ai AFTER INSERT ON dictations BEGIN
        INSERT INTO dictations_fts(rowid, text) VALUES (new.seq, coalesce(new.corrected_text, new.final_text));
    END;
    CREATE TRIGGER dictations_ad AFTER DELETE ON dictations BEGIN
        DELETE FROM dictations_fts WHERE rowid = old.seq;
    END;
    CREATE TRIGGER dictations_au AFTER UPDATE OF final_text, corrected_text ON dictations BEGIN
        UPDATE dictations_fts SET text = coalesce(new.corrected_text, new.final_text) WHERE rowid = new.seq;
    END;

    CREATE TABLE corrections (
        id           INTEGER PRIMARY KEY,
        dictation_id TEXT NOT NULL REFERENCES dictations(id) ON DELETE CASCADE,
        created_at   TEXT NOT NULL,
        before_text  TEXT NOT NULL,
        after_text   TEXT NOT NULL
    );
    CREATE INDEX corrections_dictation ON corrections(dictation_id);
    CREATE TABLE correction_terms (
        correction_id INTEGER NOT NULL REFERENCES corrections(id) ON DELETE CASCADE,
        wrong         TEXT NOT NULL,
        right         TEXT NOT NULL
    );
    CREATE INDEX correction_terms_correction ON correction_terms(correction_id);
    CREATE INDEX correction_terms_right ON correction_terms(right);

    -- Propositions de vocabulaire écartées par l'utilisateur.
    CREATE TABLE dismissed_suggestions (
        term         TEXT PRIMARY KEY,
        dismissed_at TEXT NOT NULL
    );
    "#,
];

/// Dictée telle qu'affichée et exportée.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub id: String,
    /// Texte courant : la correction manuelle si elle existe, sinon le texte collé.
    pub text: String,
    /// RFC 3339
    pub created_at: String,
    pub duration_ms: u64,
    pub latency_ms: u64,
    pub engine: EngineKind,
    pub model: String,
    pub language: String,
    pub raw_text: String,
    pub final_text: String,
    pub corrected_text: Option<String>,
    pub corrected_at: Option<String>,
    pub auto_corrections: Vec<AutoCorrection>,
    pub has_audio: bool,
    pub audio_bytes: u64,
}

/// Dictée à enregistrer, produite par le pipeline.
#[derive(Clone, Debug)]
pub struct NewDictation {
    pub raw_text: String,
    pub final_text: String,
    pub duration_ms: u64,
    pub latency_ms: u64,
    pub engine: EngineKind,
    pub model: String,
    pub language: String,
    pub auto_corrections: Vec<AutoCorrection>,
}

impl HistoryEntry {
    /// Entrée sans audio, non enregistrée (événement `dictation-result`, historique désactivé).
    pub fn new(d: &NewDictation) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            text: d.final_text.clone(),
            created_at: chrono::Local::now().to_rfc3339(),
            duration_ms: d.duration_ms,
            latency_ms: d.latency_ms,
            engine: d.engine,
            model: d.model.clone(),
            language: d.language.clone(),
            raw_text: d.raw_text.clone(),
            final_text: d.final_text.clone(),
            corrected_text: None,
            corrected_at: None,
            auto_corrections: d.auto_corrections.clone(),
            has_audio: false,
            audio_bytes: 0,
        }
    }
}

/// Recherche : texte (FTS5, préfixes), intervalle de dates locales `AAAA-MM-JJ` inclusives.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct HistoryQuery {
    pub text: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    pub total: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AudioUsage {
    pub bytes: u64,
    pub files: u64,
    pub cap_bytes: u64,
    /// Durée totale des dictées dont l'audio est conservé.
    pub duration_ms: u64,
}

/// Terme corrigé plusieurs fois, proposé pour le vocabulaire.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct VocabularySuggestion {
    pub term: String,
    pub count: u32,
    /// Formes erronées rencontrées (« affirmation »…), pour créer une règle de remplacement.
    pub variants: Vec<String>,
}

/// Résultat d'une correction manuelle.
#[derive(Clone, Debug, Serialize)]
pub struct CorrectionResult {
    pub entry: HistoryEntry,
    pub changes: Vec<TermChange>,
}

/// Format d'export de l'historique.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Json,
    /// Dossier : audio Ogg Opus et texte (corrigé s'il existe), `metadata.csv` et `metadata.jsonl`.
    Corpus,
}

pub struct HistoryDb {
    conn: Mutex<Connection>,
    dir: PathBuf,
}

/// Entrée de l'ancien `history.json` (v0.1.x).
#[derive(Deserialize)]
struct LegacyEntry {
    id: String,
    text: String,
    created_at: String,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    latency_ms: u64,
    engine: EngineKind,
    #[serde(default)]
    model: String,
}

fn engine_str(e: EngineKind) -> &'static str {
    match e {
        EngineKind::Local => "local",
        EngineKind::Remote => "remote",
    }
}

fn parse_engine(s: &str) -> EngineKind {
    if s == "remote" { EngineKind::Remote } else { EngineKind::Local }
}

fn rfc3339_to_ms(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).map(|d| d.timestamp_millis()).unwrap_or(0)
}

/// Début de la journée locale `AAAA-MM-JJ` en ms ; `next_day` : début du lendemain (borne exclusive).
fn local_day_ms(day: &str, next_day: bool) -> Option<i64> {
    use chrono::TimeZone;
    let mut d = chrono::NaiveDate::parse_from_str(day.trim(), "%Y-%m-%d").ok()?;
    if next_day {
        d = d.succ_opt()?;
    }
    let dt = d.and_hms_opt(0, 0, 0)?;
    chrono::Local.from_local_datetime(&dt).earliest().map(|t| t.timestamp_millis())
}

/// Requête FTS5 : chaque mot devient un préfixe entre guillemets, tous requis.
pub fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

const SELECT_COLUMNS: &str = "id, created_at, duration_ms, latency_ms, engine, model, language, raw_text, final_text, \
     corrected_text, corrected_at, auto_corrections, audio_file, audio_bytes";

fn row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    let final_text: String = r.get(8)?;
    let corrected_text: Option<String> = r.get(9)?;
    let auto: String = r.get(11)?;
    let audio_file: Option<String> = r.get(12)?;
    Ok(HistoryEntry {
        id: r.get(0)?,
        text: corrected_text.clone().unwrap_or_else(|| final_text.clone()),
        created_at: r.get(1)?,
        duration_ms: r.get::<_, i64>(2)?.max(0) as u64,
        latency_ms: r.get::<_, i64>(3)?.max(0) as u64,
        engine: parse_engine(&r.get::<_, String>(4)?),
        model: r.get(5)?,
        language: r.get(6)?,
        raw_text: r.get(7)?,
        final_text,
        corrected_text,
        corrected_at: r.get(10)?,
        auto_corrections: serde_json::from_str(&auto).unwrap_or_default(),
        has_audio: audio_file.is_some(),
        audio_bytes: r.get::<_, i64>(13)?.max(0) as u64,
    })
}

impl HistoryDb {
    /// Ouvre (ou crée) la base, applique les migrations et importe l'ancien `history.json`.
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir).with_context(|| format!("création de {}", data_dir.display()))?;
        let path = data_dir.join(DB_FILE);
        let conn = Connection::open(&path).with_context(|| format!("ouverture de {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            // Doit précéder la création de la première table pour prendre effet.
            conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
        }
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             -- Un texte supprimé est effacé du fichier, pas seulement délié.
             PRAGMA secure_delete = ON;",
        )?;
        let db = Self { conn: Mutex::new(conn), dir: data_dir.to_path_buf() };
        db.migrate()?;
        if let Err(e) = db.import_legacy() {
            log::warn!("import de l'ancien historique : {e:#}");
        }
        Ok(db)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.lock();
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?.max(0) as usize;
        if version > MIGRATIONS.len() {
            anyhow::bail!(
                "historique créé par une version plus récente de Murmure (schéma {version}) : mettez Murmure à jour"
            );
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql).with_context(|| format!("migration du schéma vers la version {}", i + 1))?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
            log::info!("historique : schéma en version {}", i + 1);
        }
        Ok(())
    }

    /// Importe `history.json` (v0.1.x) puis le renomme. Un fichier illisible est mis de côté
    /// (`history.json.illisible`), jamais supprimé.
    fn import_legacy(&self) -> anyhow::Result<()> {
        let legacy = self.dir.join(LEGACY_FILE);
        let bytes = match std::fs::read(&legacy) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let entries: Vec<LegacyEntry> = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("ancien historique illisible ({e}), mis de côté");
                std::fs::rename(&legacy, self.dir.join(format!("{LEGACY_FILE}.illisible")))?;
                return Ok(());
            }
        };
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let mut imported = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO dictations
                 (id, created_at, created_ms, duration_ms, latency_ms, engine, model, raw_text, final_text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            )?;
            for e in &entries {
                imported += stmt.execute(params![
                    e.id,
                    e.created_at,
                    rfc3339_to_ms(&e.created_at),
                    e.duration_ms as i64,
                    e.latency_ms as i64,
                    engine_str(e.engine),
                    e.model,
                    e.text,
                ])?;
            }
        }
        tx.commit()?;
        drop(conn);
        std::fs::rename(&legacy, self.dir.join(format!("{LEGACY_FILE}.migrated")))?;
        log::info!("historique : {imported} dictées importées depuis {LEGACY_FILE}");
        Ok(())
    }

    fn audio_dir(&self) -> PathBuf {
        self.dir.join(AUDIO_DIR)
    }

    /// Enregistre une dictée, avec son audio encodé si fourni, puis applique le plafond audio.
    pub fn insert(&self, d: &NewDictation, audio_ogg: Option<&[u8]>, audio_cap_bytes: u64) -> anyhow::Result<HistoryEntry> {
        let mut entry = HistoryEntry::new(d);
        let mut audio_file = None;
        if let Some(bytes) = audio_ogg.filter(|b| !b.is_empty() && (b.len() as u64) <= audio_cap_bytes) {
            let name = format!("{}.ogg", entry.id);
            let dir = self.audio_dir();
            let written = std::fs::create_dir_all(&dir).and_then(|_| {
                let tmp = dir.join(format!("{name}.tmp"));
                std::fs::write(&tmp, bytes)?;
                std::fs::rename(&tmp, dir.join(&name))
            });
            match written {
                Ok(()) => {
                    audio_file = Some(name);
                    entry.has_audio = true;
                    entry.audio_bytes = bytes.len() as u64;
                }
                Err(e) => log::warn!("audio de la dictée non enregistré : {e}"),
            }
        }
        let inserted = self.conn.lock().execute(
            "INSERT INTO dictations (id, created_at, created_ms, duration_ms, latency_ms, engine, model, language,
                                     raw_text, final_text, auto_corrections, audio_file, audio_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                entry.id,
                entry.created_at,
                rfc3339_to_ms(&entry.created_at),
                entry.duration_ms as i64,
                entry.latency_ms as i64,
                engine_str(entry.engine),
                entry.model,
                entry.language,
                entry.raw_text,
                entry.final_text,
                serde_json::to_string(&entry.auto_corrections)?,
                audio_file,
                entry.audio_bytes as i64,
            ],
        );
        if let Err(e) = inserted {
            if let Some(name) = &audio_file {
                let _ = std::fs::remove_file(self.audio_dir().join(name));
            }
            return Err(e.into());
        }
        if audio_file.is_some() {
            self.enforce_audio_cap(audio_cap_bytes)?;
            // L'audio de cette dictée a pu être retiré si le plafond est minuscule.
            if let Some(e) = self.get(&entry.id)? {
                entry = e;
            }
        }
        Ok(entry)
    }

    pub fn get(&self, id: &str) -> anyhow::Result<Option<HistoryEntry>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(&format!("SELECT {SELECT_COLUMNS} FROM dictations WHERE id = ?1"), [id], row_to_entry)
            .optional()?)
    }

    /// Les `limit` dictées les plus récentes.
    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<HistoryEntry>> {
        Ok(self.search(&HistoryQuery { limit: Some(limit as u32), ..Default::default() })?.entries)
    }

    /// Recherche plein texte et par dates, du plus récent au plus ancien.
    pub fn search(&self, q: &HistoryQuery) -> anyhow::Result<HistoryPage> {
        let fts = fts_query(&q.text);
        let from = q.from.as_deref().and_then(|d| local_day_ms(d, false));
        let to = q.to.as_deref().and_then(|d| local_day_ms(d, true));
        let filter = "(?1 IS NULL OR seq IN (SELECT rowid FROM dictations_fts WHERE dictations_fts MATCH ?1))
                      AND (?2 IS NULL OR created_ms >= ?2) AND (?3 IS NULL OR created_ms < ?3)";
        let conn = self.conn.lock();
        let total: i64 =
            conn.query_row(&format!("SELECT count(*) FROM dictations WHERE {filter}"), params![fts, from, to], |r| r.get(0))?;
        let limit = i64::from(q.limit.unwrap_or(100).clamp(1, 10_000));
        let offset = i64::from(q.offset.unwrap_or(0));
        let mut stmt = conn.prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM dictations WHERE {filter} ORDER BY created_ms DESC, seq DESC LIMIT ?4 OFFSET ?5"
        ))?;
        let entries = stmt
            .query_map(params![fts, from, to, limit, offset], row_to_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(HistoryPage { entries, total: total.max(0) as u64 })
    }

    /// Toutes les dictées correspondant à la recherche (export), sans pagination.
    fn search_all(&self, q: &HistoryQuery) -> anyhow::Result<Vec<HistoryEntry>> {
        let q = HistoryQuery { limit: Some(u32::MAX), offset: None, ..q.clone() };
        let fts = fts_query(&q.text);
        let from = q.from.as_deref().and_then(|d| local_day_ms(d, false));
        let to = q.to.as_deref().and_then(|d| local_day_ms(d, true));
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM dictations
             WHERE (?1 IS NULL OR seq IN (SELECT rowid FROM dictations_fts WHERE dictations_fts MATCH ?1))
               AND (?2 IS NULL OR created_ms >= ?2) AND (?3 IS NULL OR created_ms < ?3)
             ORDER BY created_ms DESC, seq DESC"
        ))?;
        let entries = stmt.query_map(params![fts, from, to], row_to_entry)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
    }

    fn remove_audio_files(&self, files: &[String]) {
        for f in files {
            if let Err(e) = std::fs::remove_file(self.audio_dir().join(f))
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("suppression de l'audio {f} : {e}");
            }
        }
    }

    /// Rend au système les pages libérées et vide le journal WAL (texte effacé du disque).
    fn reclaim(&self) {
        let conn = self.conn.lock();
        if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum; PRAGMA wal_checkpoint(TRUNCATE);") {
            log::debug!("compactage de l'historique : {e}");
        }
    }

    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        let file: Option<String> = {
            let conn = self.conn.lock();
            let file = conn
                .query_row("SELECT audio_file FROM dictations WHERE id = ?1", [id], |r| r.get(0))
                .optional()?
                .flatten();
            conn.execute("DELETE FROM dictations WHERE id = ?1", [id])?;
            file
        };
        self.remove_audio_files(&file.into_iter().collect::<Vec<_>>());
        self.reclaim();
        Ok(())
    }

    /// Efface tout l'historique : textes, corrections, audio.
    pub fn clear(&self) -> anyhow::Result<()> {
        {
            let conn = self.conn.lock();
            conn.execute_batch(
                "DELETE FROM dictations;
                 DELETE FROM dismissed_suggestions;
                 INSERT INTO dictations_fts(dictations_fts) VALUES ('optimize');",
            )?;
        }
        if let Err(e) = std::fs::remove_dir_all(self.audio_dir())
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("suppression du dossier audio : {e}");
        }
        self.reclaim();
        Ok(())
    }

    // ─────────────────────────────── Audio ───────────────────────────────

    pub fn audio_usage(&self, cap_bytes: u64) -> anyhow::Result<AudioUsage> {
        let conn = self.conn.lock();
        let (bytes, files, duration): (i64, i64, i64) = conn.query_row(
            "SELECT coalesce(sum(audio_bytes), 0), count(*), coalesce(sum(duration_ms), 0)
             FROM dictations WHERE audio_file IS NOT NULL",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(AudioUsage { bytes: bytes as u64, files: files as u64, cap_bytes, duration_ms: duration as u64 })
    }

    /// Supprime les audios les plus anciens tant que le total dépasse `cap_bytes`. Renvoie le
    /// nombre d'octets libérés. Le texte des dictées est conservé.
    pub fn enforce_audio_cap(&self, cap_bytes: u64) -> anyhow::Result<u64> {
        let removed: Vec<(String, i64)> = {
            let mut conn = self.conn.lock();
            let total: i64 = conn.query_row(
                "SELECT coalesce(sum(audio_bytes), 0) FROM dictations WHERE audio_file IS NOT NULL",
                [],
                |r| r.get(0),
            )?;
            if total as u64 <= cap_bytes {
                return Ok(0);
            }
            let mut excess = total as u64 - cap_bytes;
            let tx = conn.transaction()?;
            let mut removed = Vec::new();
            {
                let mut stmt = tx.prepare(
                    "SELECT seq, audio_file, audio_bytes FROM dictations
                     WHERE audio_file IS NOT NULL ORDER BY created_ms ASC, seq ASC",
                )?;
                let mut rows = stmt.query([])?;
                let mut clear = tx.prepare("UPDATE dictations SET audio_file = NULL, audio_bytes = 0 WHERE seq = ?1")?;
                while let Some(r) = rows.next()? {
                    if excess == 0 {
                        break;
                    }
                    let (seq, file, bytes): (i64, String, i64) = (r.get(0)?, r.get(1)?, r.get(2)?);
                    clear.execute([seq])?;
                    excess = excess.saturating_sub(bytes.max(0) as u64);
                    removed.push((file, bytes));
                }
            }
            tx.commit()?;
            removed
        };
        let freed = removed.iter().map(|(_, b)| b.max(&0)).sum::<i64>() as u64;
        log::info!("audio de l'historique : {} fichiers anciens supprimés ({freed} octets)", removed.len());
        self.remove_audio_files(&removed.into_iter().map(|(f, _)| f).collect::<Vec<_>>());
        Ok(freed)
    }

    /// Supprime tous les audios ; les textes restent.
    pub fn delete_all_audio(&self) -> anyhow::Result<()> {
        self.conn.lock().execute("UPDATE dictations SET audio_file = NULL, audio_bytes = 0 WHERE audio_file IS NOT NULL", [])?;
        if let Err(e) = std::fs::remove_dir_all(self.audio_dir())
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(e.into());
        }
        Ok(())
    }

    /// Audio d'une dictée décodé en WAV (écoute dans l'interface).
    pub fn audio_wav(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        let file: Option<String> = self
            .conn
            .lock()
            .query_row("SELECT audio_file FROM dictations WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .flatten();
        let file = file.ok_or_else(|| anyhow::anyhow!("Aucun audio pour cette dictée."))?;
        let bytes = std::fs::read(self.audio_dir().join(&file)).context("Fichier audio introuvable")?;
        let samples = audio::decode_ogg_opus(&bytes)?;
        Ok(crate::engine::encode_wav(&samples))
    }

    // ───────────────────────────── Corrections ─────────────────────────────

    /// Remplace le texte d'une dictée par `text`, enregistre la paire avant/après et les termes
    /// corrigés.
    pub fn correct(&self, id: &str, text: &str) -> anyhow::Result<CorrectionResult> {
        let text = text.trim();
        if text.is_empty() {
            anyhow::bail!("Le texte ne peut pas être vide : supprimez plutôt la dictée.");
        }
        let entry = self.get(id)?.ok_or_else(|| anyhow::anyhow!("Dictée introuvable."))?;
        if entry.text == text {
            return Ok(CorrectionResult { entry, changes: Vec::new() });
        }
        let changes = correction::word_diff(&entry.text, text);
        let now = chrono::Local::now().to_rfc3339();
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO corrections (dictation_id, created_at, before_text, after_text) VALUES (?1, ?2, ?3, ?4)",
                params![id, now, entry.text, text],
            )?;
            let correction_id = tx.last_insert_rowid();
            for c in &changes {
                tx.execute(
                    "INSERT INTO correction_terms (correction_id, wrong, right) VALUES (?1, ?2, ?3)",
                    params![correction_id, c.wrong, c.right],
                )?;
            }
            // Revenir au texte d'origine efface la correction.
            let corrected = (text != entry.final_text).then_some(text);
            tx.execute(
                "UPDATE dictations SET corrected_text = ?2, corrected_at = ?3 WHERE id = ?1",
                params![id, corrected, corrected.map(|_| now.as_str())],
            )?;
            tx.commit()?;
        }
        let entry = self.get(id)?.ok_or_else(|| anyhow::anyhow!("Dictée introuvable."))?;
        Ok(CorrectionResult { entry, changes })
    }

    /// Termes corrigés au moins [`SUGGEST_AFTER`] fois, absents du vocabulaire et des règles, non
    /// écartés : proposés pour le vocabulaire (jamais ajoutés sans action de l'utilisateur).
    pub fn suggestions(
        &self,
        vocabulary: &str,
        rules: &[correction::Replacement],
    ) -> anyhow::Result<Vec<VocabularySuggestion>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT t.right, count(DISTINCT t.correction_id) AS n, group_concat(DISTINCT t.wrong)
             FROM correction_terms t
             WHERE t.right NOT IN (SELECT term FROM dismissed_suggestions)
             GROUP BY t.right
             HAVING n >= ?1
             ORDER BY n DESC, max(t.correction_id) DESC
             LIMIT 50",
        )?;
        let rows = stmt.query_map([SUGGEST_AFTER], |r| {
            Ok(VocabularySuggestion {
                term: r.get(0)?,
                count: r.get::<_, i64>(1)? as u32,
                variants: r
                    .get::<_, Option<String>>(2)?
                    .unwrap_or_default()
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        })?;
        let mut out = Vec::new();
        for s in rows {
            let s = s?;
            let ruled = rules.iter().any(|r| r.to.trim().eq_ignore_ascii_case(s.term.trim()));
            if correction::is_vocabulary_candidate(&s.term) && !correction::vocabulary_contains(vocabulary, &s.term) && !ruled {
                out.push(s);
            }
        }
        Ok(out)
    }

    pub fn dismiss_suggestion(&self, term: &str) -> anyhow::Result<()> {
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO dismissed_suggestions (term, dismissed_at) VALUES (?1, ?2)",
            params![term, chrono::Local::now().to_rfc3339()],
        )?;
        Ok(())
    }

    // ─────────────────────────────── Export ───────────────────────────────

    /// Exporte les dictées correspondant à `q` dans `dest` (fichier ou dossier selon le format).
    /// Renvoie le nombre de dictées exportées.
    pub fn export(&self, q: &HistoryQuery, format: ExportFormat, dest: &Path) -> anyhow::Result<usize> {
        let entries = self.search_all(q)?;
        match format {
            ExportFormat::Json => {
                std::fs::write(dest, serde_json::to_vec_pretty(&entries)?)?;
                Ok(entries.len())
            }
            ExportFormat::Csv => {
                std::fs::write(dest, to_csv(&entries))?;
                Ok(entries.len())
            }
            ExportFormat::Corpus => self.export_corpus(&entries, dest),
        }
    }

    fn export_corpus(&self, entries: &[HistoryEntry], dest: &Path) -> anyhow::Result<usize> {
        let audio_dest = dest.join("audio");
        std::fs::create_dir_all(&audio_dest)?;
        let mut csv = String::from("file_name,transcription\n");
        let mut jsonl = String::new();
        let mut n = 0;
        let with_audio: Vec<&HistoryEntry> = entries.iter().filter(|e| e.has_audio).collect();
        let mut files = Vec::with_capacity(with_audio.len());
        {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare("SELECT audio_file FROM dictations WHERE id = ?1")?;
            for e in &with_audio {
                files.push(stmt.query_row([&e.id], |r| r.get::<_, Option<String>>(0)).optional()?.flatten());
            }
        }
        for (e, file) in with_audio.into_iter().zip(files) {
            let Some(file) = file else { continue };
            let src = self.audio_dir().join(&file);
            if std::fs::copy(&src, audio_dest.join(&file)).is_err() {
                continue;
            }
            let rel = format!("audio/{file}");
            csv.push_str(&format!("{},{}\n", csv_field(&rel, ','), csv_field(&e.text, ',')));
            jsonl.push_str(&serde_json::to_string(&serde_json::json!({
                "audio": rel,
                "text": e.text,
                "corrected": e.corrected_text.is_some(),
                "raw_text": e.raw_text,
                "language": e.language,
                "duration_ms": e.duration_ms,
                "model": e.model,
                "created_at": e.created_at,
            }))?);
            jsonl.push('\n');
            n += 1;
        }
        std::fs::write(dest.join("metadata.csv"), csv)?;
        std::fs::write(dest.join("metadata.jsonl"), jsonl)?;
        Ok(n)
    }
}

// ─────────────────────────── Intégration à l'app ───────────────────────────

/// Historique géré par Tauri. Une base impossible à ouvrir n'empêche pas de dicter : l'erreur est
/// renvoyée par les commandes de l'historique.
pub struct HistoryState {
    db: Result<Arc<HistoryDb>, String>,
}

impl HistoryState {
    pub fn open(data_dir: &Path) -> Self {
        let db = HistoryDb::open(data_dir).map(Arc::new).map_err(|e| {
            log::error!("historique indisponible : {e:#}");
            format!("Historique indisponible : {e:#}")
        });
        Self { db }
    }

    pub fn db(&self) -> Result<Arc<HistoryDb>, String> {
        self.db.clone()
    }
}

/// Enregistre une dictée en tâche de fond (encodage Opus si l'audio est conservé), puis émet
/// `dictation-result`. Historique désactivé : rien n'est écrit, l'événement part aussitôt.
pub fn record(app: &AppHandle, settings: &Settings, d: NewDictation, samples: Arc<Vec<f32>>) {
    let emit = |app: &AppHandle, entry: &HistoryEntry| {
        if let Err(e) = app.emit("dictation-result", entry) {
            log::warn!("émission dictation-result : {e}");
        }
    };
    let db = match app.try_state::<HistoryState>().map(|s| s.db()) {
        Some(Ok(db)) if settings.history_enabled => db,
        _ => {
            emit(app, &HistoryEntry::new(&d));
            return;
        }
    };
    let (keep_audio, cap) = (settings.keep_audio, settings.audio_cap_bytes());
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let audio = keep_audio
            .then(|| audio::encode_ogg_opus(&samples))
            .and_then(|r| r.map_err(|e| log::warn!("encodage de l'audio : {e:#}")).ok());
        let entry = match db.insert(&d, audio.as_deref(), cap) {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!("historique : {e:#}");
                HistoryEntry::new(&d)
            }
        };
        emit(&app, &entry);
    });
}

fn csv_field(s: &str, sep: char) -> String {
    if s.contains([sep, '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// CSV pour les tableurs : séparateur point-virgule et BOM UTF-8 (Excel en français).
pub fn to_csv(entries: &[HistoryEntry]) -> String {
    let mut out = String::from("\u{feff}date;duree_ms;latence_ms;moteur;modele;langue;texte;texte_brut;texte_colle;corrige;audio\n");
    for e in entries {
        let fields = [
            e.created_at.clone(),
            e.duration_ms.to_string(),
            e.latency_ms.to_string(),
            engine_str(e.engine).to_string(),
            e.model.clone(),
            e.language.clone(),
            e.text.clone(),
            e.raw_text.clone(),
            e.final_text.clone(),
            if e.corrected_text.is_some() { "oui" } else { "non" }.to_string(),
            if e.has_audio { "oui" } else { "non" }.to_string(),
        ];
        out.push_str(&fields.iter().map(|f| csv_field(f, ';')).collect::<Vec<_>>().join(";"));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::correction::Replacement;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmure-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn dictation(text: &str) -> NewDictation {
        NewDictation {
            raw_text: text.to_lowercase(),
            final_text: text.to_string(),
            duration_ms: 1000,
            latency_ms: 200,
            engine: EngineKind::Local,
            model: "large-v3-turbo-q5_0".into(),
            language: "fr".into(),
            auto_corrections: Vec::new(),
        }
    }

    const MB: u64 = 1024 * 1024;

    #[test]
    fn schema_versionne_et_reouverture() {
        let dir = temp_dir();
        {
            let db = HistoryDb::open(&dir).unwrap();
            db.insert(&dictation("Bonjour"), None, MB).unwrap();
        }
        let db = HistoryDb::open(&dir).unwrap();
        let conn = db.conn.lock();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
        let av: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(av, 2, "auto_vacuum incrémental");
        drop(conn);
        assert_eq!(db.recent(10).unwrap().len(), 1);
        // Schéma plus récent que l'application : refus explicite, pas de corruption.
        db.conn.lock().pragma_update(None, "user_version", 99).unwrap();
        drop(db);
        let err = HistoryDb::open(&dir).err().unwrap().to_string();
        assert!(err.contains("plus récente"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_de_l_ancien_historique_sans_perte() {
        let dir = temp_dir();
        let legacy = r#"[
          {"id":"b","text":"Deuxième dictée, plus récente.","created_at":"2026-10-14T10:00:00+02:00",
           "duration_ms":2100,"latency_ms":310,"engine":"remote","model":"whisper-large-v3-turbo"},
          {"id":"a","text":"Première dictée : l'infirmation du jugement.","created_at":"2026-10-13T09:30:00+02:00",
           "duration_ms":1500,"latency_ms":280,"engine":"local","model":"large-v3-turbo-q5_0"}
        ]"#;
        std::fs::write(dir.join(LEGACY_FILE), legacy).unwrap();
        let db = HistoryDb::open(&dir).unwrap();
        let all = db.recent(10).unwrap();
        assert_eq!(all.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(all[0].engine, EngineKind::Remote);
        assert_eq!(all[0].duration_ms, 2100);
        assert_eq!(all[1].text, "Première dictée : l'infirmation du jugement.");
        assert_eq!(all[1].raw_text, all[1].final_text);
        assert!(!dir.join(LEGACY_FILE).exists());
        assert!(dir.join(format!("{LEGACY_FILE}.migrated")).exists(), "sauvegarde conservée");
        // Recherche plein texte sur les entrées importées.
        let hits = db.search(&HistoryQuery { text: "infirmation".into(), ..Default::default() }).unwrap();
        assert_eq!(hits.total, 1);
        drop(db);
        // Réouverture : rien n'est réimporté.
        let db = HistoryDb::open(&dir).unwrap();
        assert_eq!(db.recent(10).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ancien_historique_illisible_mis_de_cote() {
        let dir = temp_dir();
        std::fs::write(dir.join(LEGACY_FILE), b"{pas du json").unwrap();
        let db = HistoryDb::open(&dir).unwrap();
        assert!(db.recent(10).unwrap().is_empty());
        assert_eq!(std::fs::read(dir.join(format!("{LEGACY_FILE}.illisible"))).unwrap(), b"{pas du json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recherche_plein_texte_et_dates() {
        let dir = temp_dir();
        let db = HistoryDb::open(&dir).unwrap();
        for t in [
            "Nous sollicitons l'infirmation du jugement entrepris.",
            "Mise en demeure de régler les loyers impayés.",
            "Conclusions récapitulatives avant l'audience de mise en état.",
        ] {
            db.insert(&dictation(t), None, MB).unwrap();
        }
        let q = |text: &str| db.search(&HistoryQuery { text: text.into(), ..Default::default() }).unwrap();
        // Sans accents ni casse, par préfixe, tous les mots requis.
        assert_eq!(q("RECAPITUL").total, 1);
        assert_eq!(q("mise").total, 2);
        assert_eq!(q("mise état").total, 1);
        assert_eq!(q("demeure loyers").entries[0].text, "Mise en demeure de régler les loyers impayés.");
        assert_eq!(q("introuvable").total, 0);
        // Caractères spéciaux de FTS5 neutralisés.
        assert_eq!(q("\"l'infirmation\" (").total, 1);
        assert_eq!(q("jugement OR loyers").total, 0, "OR est un mot cherché, pas un opérateur");
        assert_eq!(q("  ").total, 3);
        // Dates : aujourd'hui inclus, hier exclu.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let yesterday = (chrono::Local::now() - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
        let by_date = |from: &str, to: &str| {
            db.search(&HistoryQuery { from: Some(from.into()), to: Some(to.into()), ..Default::default() }).unwrap().total
        };
        assert_eq!(by_date(&today, &today), 3);
        assert_eq!(by_date(&yesterday, &yesterday), 0);
        // Pagination.
        let page = db.search(&HistoryQuery { limit: Some(2), offset: Some(2), ..Default::default() }).unwrap();
        assert_eq!((page.total, page.entries.len()), (3, 1));
        // Le texte corrigé est indexé, l'ancien ne l'est plus.
        let id = q("loyers").entries[0].id.clone();
        db.correct(&id, "Mise en demeure de régler les charges impayées.").unwrap();
        assert_eq!(q("loyers").total, 0);
        assert_eq!(q("charges").total, 1);
        // Suppression : l'index suit.
        db.delete(&id).unwrap();
        assert_eq!(q("charges").total, 0);
        db.clear().unwrap();
        assert_eq!(q("").total, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_de_l_audio_au_plafond() {
        let dir = temp_dir();
        let db = HistoryDb::open(&dir).unwrap();
        let blob = vec![7u8; 400 * 1024];
        let cap = MB; // 1 Mo : deux fichiers de 400 Ko tiennent, pas trois.
        let ids: Vec<String> = (0..5)
            .map(|i| {
                std::thread::sleep(std::time::Duration::from_millis(3));
                db.insert(&dictation(&format!("dictée {i}")), Some(&blob), cap).unwrap().id
            })
            .collect();
        let usage = db.audio_usage(cap).unwrap();
        assert_eq!(usage.files, 2);
        assert!(usage.bytes <= cap);
        // Les plus anciens ont perdu leur audio, pas leur texte.
        for (i, id) in ids.iter().enumerate() {
            let e = db.get(id).unwrap().unwrap();
            assert_eq!(e.text, format!("dictée {i}"));
            assert_eq!(e.has_audio, i >= 3, "dictée {i}");
            assert_eq!(dir.join(AUDIO_DIR).join(format!("{id}.ogg")).exists(), i >= 3);
        }
        // Plafond abaissé : nouvelle rotation.
        assert_eq!(db.enforce_audio_cap(500 * 1024).unwrap(), 400 * 1024);
        assert_eq!(db.audio_usage(cap).unwrap().files, 1);
        assert!(db.get(&ids[4]).unwrap().unwrap().has_audio);
        // Bouton « Supprimer les audios ».
        db.delete_all_audio().unwrap();
        assert_eq!(db.audio_usage(cap).unwrap(), AudioUsage { bytes: 0, files: 0, cap_bytes: cap, duration_ms: 0 });
        assert_eq!(db.recent(10).unwrap().len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plafond_de_5_go_par_defaut() {
        // La rotation se fait sur la somme des tailles enregistrées : simulée sans écrire 5 Go.
        let dir = temp_dir();
        let db = HistoryDb::open(&dir).unwrap();
        let cap = DEFAULT_AUDIO_CAP_MB * MB;
        assert_eq!(cap, 5 * 1024 * 1024 * 1024);
        for i in 0..6 {
            let e = db.insert(&dictation(&format!("d{i}")), Some(b"OggS"), cap).unwrap();
            // Chaque fichier « pèse » 1 Go dans la base.
            db.conn.lock().execute("UPDATE dictations SET audio_bytes = ?2 WHERE id = ?1", params![e.id, 1i64 << 30]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        db.enforce_audio_cap(cap).unwrap();
        let usage = db.audio_usage(cap).unwrap();
        assert_eq!((usage.files, usage.bytes), (5, 5 << 30));
        let oldest = db.search(&HistoryQuery { text: "d0".into(), ..Default::default() }).unwrap();
        assert!(!oldest.entries[0].has_audio);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audio_reel_reecoute_en_wav() {
        let dir = temp_dir();
        let db = HistoryDb::open(&dir).unwrap();
        let samples: Vec<f32> = (0..16_000).map(|i| (i as f32 * 0.07).sin() * 0.3).collect();
        let ogg = audio::encode_ogg_opus(&samples).unwrap();
        let e = db.insert(&dictation("avec audio"), Some(&ogg), MB).unwrap();
        assert!(e.has_audio && e.audio_bytes > 0);
        let wav = db.audio_wav(&e.id).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(wav.len(), 44 + samples.len() * 2);
        db.delete(&e.id).unwrap();
        assert!(!dir.join(AUDIO_DIR).join(format!("{}.ogg", e.id)).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrections_et_propositions_de_vocabulaire() {
        let dir = temp_dir();
        let db = HistoryDb::open(&dir).unwrap();
        let a = db.insert(&dictation("Nous sollicitons l'affirmation du jugement."), None, MB).unwrap();
        let b = db.insert(&dictation("Confirmation ou affirmation de la décision."), None, MB).unwrap();
        let c = db.insert(&dictation("Le dossier Batimax est clos."), None, MB).unwrap();

        let r = db.correct(&a.id, "Nous sollicitons l'infirmation du jugement.").unwrap();
        assert_eq!(r.changes, vec![TermChange { wrong: "affirmation".into(), right: "infirmation".into() }]);
        assert_eq!(r.entry.text, "Nous sollicitons l'infirmation du jugement.");
        assert_eq!(r.entry.final_text, "Nous sollicitons l'affirmation du jugement.");
        assert!(r.entry.corrected_at.is_some());
        // Une seule correction : rien n'est proposé.
        assert!(db.suggestions("", &[]).unwrap().is_empty());

        db.correct(&b.id, "Confirmation ou infirmation de la décision.").unwrap();
        db.correct(&c.id, "Le dossier Batimex est clos.").unwrap();
        let s = db.suggestions("", &[]).unwrap();
        assert_eq!(s, vec![VocabularySuggestion { term: "infirmation".into(), count: 2, variants: vec!["affirmation".into()] }]);

        // Déjà dans le vocabulaire, ou cible d'une règle : plus proposé.
        assert!(db.suggestions("Murmure, Infirmation", &[]).unwrap().is_empty());
        let rule = Replacement { from: "affirmation".into(), to: "infirmation".into() };
        assert!(db.suggestions("", &[rule]).unwrap().is_empty());
        // Écarté : plus proposé.
        db.dismiss_suggestion("infirmation").unwrap();
        assert!(db.suggestions("", &[]).unwrap().is_empty());

        // La paire avant/après est conservée.
        let n: i64 = db.conn.lock().query_row("SELECT count(*) FROM corrections", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        // Retour au texte d'origine : plus de correction affichée.
        let back = db.correct(&a.id, "Nous sollicitons l'affirmation du jugement.").unwrap();
        assert!(back.entry.corrected_text.is_none());
        assert!(db.correct(&a.id, "   ").is_err());
        // Suppression d'une dictée : ses corrections partent avec elle.
        db.delete(&c.id).unwrap();
        let n: i64 = db.conn.lock().query_row("SELECT count(*) FROM correction_terms", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exports_csv_json_corpus() {
        let dir = temp_dir();
        let db = HistoryDb::open(&dir).unwrap();
        let ogg = audio::encode_ogg_opus(&vec![0.05f32; 8000]).unwrap();
        let a = db.insert(&dictation("Texte; avec \"guillemets\""), Some(&ogg), MB).unwrap();
        db.insert(&dictation("Sans audio"), None, MB).unwrap();
        db.correct(&a.id, "Texte; corrigé \"guillemets\"").unwrap();
        let out = temp_dir();
        let all = HistoryQuery::default();
        assert_eq!(db.export(&all, ExportFormat::Csv, &out.join("h.csv")).unwrap(), 2);
        let csv = std::fs::read_to_string(out.join("h.csv")).unwrap();
        assert!(csv.starts_with("\u{feff}date;"));
        assert!(csv.contains("\"Texte; corrigé \"\"guillemets\"\"\""));
        assert_eq!(db.export(&all, ExportFormat::Json, &out.join("h.json")).unwrap(), 2);
        let json: Vec<HistoryEntry> = serde_json::from_slice(&std::fs::read(out.join("h.json")).unwrap()).unwrap();
        assert_eq!(json.len(), 2);
        let corpus = out.join("corpus");
        assert_eq!(db.export(&all, ExportFormat::Corpus, &corpus).unwrap(), 1);
        assert!(corpus.join("audio").join(format!("{}.ogg", a.id)).exists());
        let meta = std::fs::read_to_string(corpus.join("metadata.jsonl")).unwrap();
        assert!(meta.contains("\"corrected\":true") && meta.contains("corrigé"));
        // Export filtré par la recherche.
        let q = HistoryQuery { text: "sans".into(), ..Default::default() };
        assert_eq!(db.export(&q, ExportFormat::Json, &out.join("f.json")).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&out);
    }
}

# Murmure — architecture et contrat interne

Murmure est une app de dictée vocale façon Wispr Flow : on maintient un raccourci, on parle,
on relâche, le texte apparaît dans l'application active. Deux moteurs :

- **Local** : Whisper large-v3-turbo via whisper.cpp (`whisper-rs`), Metal sur macOS, CUDA en option (`--features cuda`).
- **Serveur** : n'importe quel endpoint compatible OpenAI `POST /v1/audio/transcriptions`
  (notre serveur `server/` à base de faster-whisper, ou OpenAI / Groq), authentifié par clé API (Bearer).

Cibles : macOS (Apple Silicon d'abord), Linux KDE (X11 et Wayland), Windows.

## Arborescence

```
src/                    Front React + TS + Tailwind v4
  main.tsx              Fenêtre principale (index.html)
  overlay.tsx           Pastille flottante (overlay.html)
  lib/api.ts            Wrappers typés invoke/listen (SEULE porte vers Rust)
  lib/types.ts          Types partagés (miroir des structs Rust ci-dessous)
src-tauri/src/
  lib.rs                Builder Tauri, plugins, enregistrement des commandes, setup   [agent système]
  state.rs              AppState partagé (Mutex) + machine d'états de la dictée      [agent système]
  pipeline.rs           start/stop/cancel → audio → moteur → injection → historique  [agent système]
  hotkey.rs             Raccourci global (hold / toggle / hybride)                   [agent système]
  inject.rs             Collage dans l'app active (presse-papiers + Cmd/Ctrl+V)      [agent système]
  overlay.rs            Positionnement / affichage de la fenêtre overlay             [agent système]
  tray.rs               Icône de barre de menus                                      [agent système]
  permissions.rs        Micro / Accessibilité (macOS)                                [agent système]
  history.rs            Historique JSON local                                        [agent système]
  settings.rs           Réglages JSON + clé API dans le trousseau                    [agent audio/moteurs]
  audio.rs              Capture micro cpal → mono 16 kHz f32, niveau RMS             [agent audio/moteurs]
  models.rs             Catalogue + téléchargement des modèles ggml                  [agent audio/moteurs]
  engine/mod.rs         enum Engine + TranscribeOptions                              [agent audio/moteurs]
  engine/local.rs       whisper-rs, contexte gardé en mémoire                        [agent audio/moteurs]
  engine/remote.rs      Client HTTP multipart compatible OpenAI                      [agent audio/moteurs]
server/                 Serveur FastAPI + faster-whisper, Docker CUDA/CPU            [agent serveur]
```

## Réglages (`settings.json` dans le dossier de config de l'app)

```jsonc
{
  "engine": "local",                       // "local" | "remote"
  "local_model": "large-v3-turbo-q5_0",    // id du catalogue models.rs
  "language": "fr",                        // "auto" | code ISO 639-1
  "initial_prompt": "",                    // vocabulaire / style (noms propres, jargon)
  "remote_endpoint": "",                   // ex. https://stt.exemple.fr/v1/audio/transcriptions
  "remote_model": "whisper-large-v3-turbo",
  "hotkey": "Alt+Space",                   // format accelerator Tauri ; défaut Linux/Windows : "Ctrl+Alt+Space"
  "hotkey_mode": "hybrid",                 // "hybrid" (tap = mains libres, maintien = push-to-talk) | "hold" | "toggle"
  "input_device": null,                    // null = micro par défaut, sinon nom du périphérique
  "paste_mode": "paste",                   // "paste" (presse-papiers + Cmd/Ctrl+V) | "clipboard" (copie seule)
  "restore_clipboard": true,
  "sounds": true,
  "launch_at_login": false,
  "history_enabled": true,
  "onboarding_done": false
}
```

La clé API n'est **jamais** dans `settings.json` : trousseau système (`keyring`, service `fr.rdagency.murmure`, compte `remote_api_key`).

## Commandes Tauri (front → Rust)

Noms snake_case côté Rust, arguments camelCase côté JS (convention Tauri). Erreurs : `Result<T, String>`.

| Commande | Arguments | Retour |
|---|---|---|
| `get_settings` | — | `Settings` |
| `save_settings` | `settings: Settings` | `()` — réapplique raccourci, autostart, recharge moteur si besoin |
| `set_api_key` | `key: string` (vide = suppression) | `()` |
| `has_api_key` | — | `bool` |
| `test_remote` | `endpoint: string, apiKey: string \| null, model: string` | `string` (message de succès + latence) |
| `list_input_devices` | — | `AudioDevice[]` |
| `list_models` | — | `ModelInfo[]` |
| `download_model` | `id: string` | `()` (progression par événement) |
| `cancel_download` | `id: string` | `()` |
| `delete_model` | `id: string` | `()` |
| `start_dictation` / `stop_dictation` / `cancel_dictation` / `toggle_dictation` | — | `()` |
| `get_history` | — | `HistoryEntry[]` (plus récent d'abord) |
| `delete_history_entry` | `id: string` | `()` |
| `clear_history` | — | `()` |
| `copy_text` | `text: string` | `()` |
| `check_permissions` | — | `Permissions` |
| `request_microphone` | — | `Permissions` |
| `open_permission_settings` | `kind: "microphone" \| "accessibility"` | `()` |
| `get_platform` | — | `"macos" \| "linux" \| "windows"` |

## Événements (Rust → front, `emit` global)

| Événement | Payload |
|---|---|
| `dictation-state` | `{ state: "idle" \| "recording" \| "transcribing" \| "error", message?: string, hands_free?: bool }` |
| `audio-level` | `number` 0..1, ~30 Hz pendant l'enregistrement |
| `dictation-result` | `HistoryEntry` |
| `model-download-progress` | `{ id, downloaded, total, done: bool, error?: string }` |
| `settings-changed` | `Settings` |
| `navigate` | `"home" \| "history" \| "settings"` (depuis le tray) |

## Types

```ts
type AudioDevice   = { name: string; is_default: boolean };
type ModelInfo     = { id: string; name: string; description: string; size_mb: number;
                       downloaded: boolean; recommended: boolean; path: string | null };
type HistoryEntry  = { id: string; text: string; created_at: string /* RFC3339 */;
                       duration_ms: number; latency_ms: number; engine: "local" | "remote";
                       model: string };
type Permissions   = { microphone: "granted" | "denied" | "undetermined" | "unknown";
                       accessibility: "granted" | "denied" | "unknown" };
```

## API Rust entre modules (à respecter à la lettre)

```rust
// settings.rs
#[derive(Clone, Serialize, Deserialize)] pub struct Settings { /* champs ci-dessus, #[serde(default)] */ }
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)] #[serde(rename_all="snake_case")] pub enum EngineKind { Local, Remote }
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)] #[serde(rename_all="snake_case")] pub enum HotkeyMode { Hybrid, Hold, Toggle }
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)] #[serde(rename_all="snake_case")] pub enum PasteMode { Paste, Clipboard }
pub fn load(config_dir: &Path) -> Settings;                      // défauts si absent/corrompu
pub fn save(config_dir: &Path, s: &Settings) -> anyhow::Result<()>;
pub fn get_api_key() -> Option<String>;
pub fn set_api_key(key: &str) -> anyhow::Result<()>;            // "" = suppression

// audio.rs
pub fn list_input_devices() -> Vec<AudioDevice>;
pub struct Recorder { .. }                                       // Send
impl Recorder {
    /// Démarre la capture. `on_level` est appelé ~30 fois/s avec un niveau 0..1 (hors thread temps réel).
    pub fn start(device: Option<&str>, on_level: impl Fn(f32) + Send + 'static) -> anyhow::Result<Recorder>;
    /// Arrête et renvoie l'audio mono 16 kHz f32.
    pub fn stop(self) -> anyhow::Result<Vec<f32>>;
}

// models.rs
pub fn models_dir(data_dir: &Path) -> PathBuf;
pub fn list(data_dir: &Path) -> Vec<ModelInfo>;
pub fn model_path(data_dir: &Path, id: &str) -> Option<PathBuf>; // Some si présent sur disque
pub async fn download(data_dir: &Path, id: &str, cancel: Arc<AtomicBool>,
                      on_progress: impl Fn(u64, u64) + Send + 'static) -> anyhow::Result<PathBuf>;
pub fn delete(data_dir: &Path, id: &str) -> anyhow::Result<()>;

// engine/mod.rs
pub struct TranscribeOptions { pub language: Option<String> /* None = auto */, pub initial_prompt: Option<String> }
pub struct LocalEngine { .. }   // engine/local.rs
impl LocalEngine {
    pub fn load(model_path: &Path) -> anyhow::Result<LocalEngine>;   // lourd : appeler hors thread UI
    pub fn model_path(&self) -> &Path;
    pub fn transcribe(&self, samples: &[f32], opts: &TranscribeOptions) -> anyhow::Result<String>;
}
pub struct RemoteEngine { pub endpoint: String, pub api_key: Option<String>, pub model: String }
impl RemoteEngine {
    pub async fn transcribe(&self, samples: &[f32], opts: &TranscribeOptions) -> anyhow::Result<String>;
    pub async fn test(&self) -> anyhow::Result<String>;            // envoie 1 s de silence, renvoie un message lisible
}
pub fn encode_wav(samples: &[f32]) -> Vec<u8>;                     // PCM 16 bits mono 16 kHz
```

## Flux de dictée

```
raccourci pressé ─► state=recording, overlay visible, son "start", Recorder::start
raccourci relâché ─► (hybride : appui < 300 ms → mains libres, on attend un 2e appui)
                  ─► Recorder::stop ─► state=transcribing
                  ─► LocalEngine/RemoteEngine::transcribe (thread bloquant / async)
                  ─► texte nettoyé (trim, espaces) ─► inject::paste(texte) ─► historique
                  ─► dictation-result, state=idle, overlay masqué
Échap pendant l'enregistrement ─► cancel_dictation
Enregistrement < 250 ms ou silence ─► abandon silencieux
```

## Principes UI

Neutre, calme, typographie système (SF Pro / Inter), une seule couleur d'accent sobre, clair et sombre
automatiques. La fenêtre principale sert surtout à régler ; l'usage quotidien passe par le raccourci et la pastille.
Tout le texte de l'interface est en français.

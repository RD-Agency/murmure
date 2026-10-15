# Architecture de Murmure

Ce document décrit l'organisation du code et les contrats entre ses parties. Il s'adresse à qui veut
modifier Murmure. Pour l'utiliser, voir le [README](README.md).

## Vue d'ensemble

Murmure est une application [Tauri 2](https://tauri.app). Le cœur est en Rust (`src-tauri/`) : capture du
micro, raccourci global, transcription, collage, mises à jour. L'interface est en React et TypeScript
(`src/`) et ne fait que régler et afficher : elle ne parle à Rust que par les commandes et les événements
décrits plus bas.

Deux fenêtres partagent le même front : la fenêtre principale (`index.html`, `src/main.tsx`) et la pastille
flottante qui s'affiche pendant une dictée (`overlay.html`, `src/overlay.tsx`). Au quotidien, la fenêtre
principale reste fermée ; l'application vit dans la barre de menus.

Deux moteurs de transcription :

- **Local** : Whisper via whisper.cpp (crate `whisper-rs`). Le modèle reste chargé en mémoire entre deux
  dictées. Accélération Metal sur macOS ; sous Linux et Windows, CUDA (`--features cuda`) ou Vulkan
  (`--features vulkan`), exclusifs l'un de l'autre. L'accélérateur est détecté à l'exécution
  (`engine::local::acceleration`, premier périphérique ggml de type GPU, même règle que whisper.cpp) :
  recherche en faisceau de largeur 5 sur GPU, décodage glouton sur processeur. La flash attention est
  coupée avec Vulkan : whisper.cpp ne remplit pas correctement le cache KV pour ce backend, ce qui répète
  des mots et tronque des phrases (mesuré sur RTX 5060 et lavapipe).
  Au-delà de 30 s d'audio, les timestamps sont décodés (jamais affichés) : sans eux, whisper.cpp avance
  sa fenêtre de 30 s pile et perd les mots à cheval sur la jonction. En deçà, ils restent coupés, car ils
  ne changent pas le texte et déclenchent parfois une seconde passe inutile.
- **Serveur** : tout service compatible avec `POST /v1/audio/transcriptions` d'OpenAI, authentifié par
  une clé Bearer. Le serveur de `server/` (FastAPI et faster-whisper) en est un, documenté dans
  [server/README.md](server/README.md).

## Arborescence

```
src/                      Front React, TypeScript, Tailwind CSS 4
  main.tsx                fenêtre principale
  overlay.tsx             pastille flottante
  lib/api.ts              appels typés à Rust : seul point de contact avec le backend
  lib/types.ts            types partagés, miroir des structures Rust
  lib/mock.ts             faux backend du mode démo dans le navigateur
  views/, components/     écrans (accueil, historique, réglages, assistant) et composants
src-tauri/src/
  lib.rs                  construction de l'app Tauri, greffons, commandes, démarrage
  state.rs                état partagé (AppState) et machine d'états de la dictée
  pipeline.rs             démarrer, arrêter, annuler : audio, moteur, collage, historique
  postprocess.rs          mise en forme finale : mot de validation, ponctuation, textes courts
  silence.rs              détection du silence pour l'arrêt automatique
  ducking.rs              baisse du volume de sortie pendant l'enregistrement
  audio_file.rs           fichiers audio : décodage (Symphonia, opus-decoder), découpage sur silence
  file_transcription.rs   transcription d'un fichier : commandes, progression, annulation
  config_transfer.rs      export et import des réglages (.json), options --export-config et --import-config
  hotkey.rs               raccourci global : maintien, bascule, hybride
  inject.rs               collage dans l'application active
  wayland.rs              détection d'une session Wayland, état des portails, conversions
  portal.rs               portails XDG GlobalShortcuts et RemoteDesktop (Linux)
  overlay.rs              position et affichage de la pastille
  tray.rs                 icône et menu de la barre de menus
  permissions.rs          micro et Accessibilité (macOS)
  history.rs              historique local (history.json)
  settings.rs             réglages (settings.json) et clé API dans le trousseau
  audio.rs                capture du micro (cpal), conversion en mono 16 kHz, niveau sonore
  models.rs               catalogue des modèles, téléchargement vérifié
  install.rs              installation automatique du modèle, option --install-model
  updater.rs              mises à jour automatiques
  punctuation.rs          ponctuation dictée en français
  engine/mod.rs           types communs aux moteurs
  engine/local.rs         moteur local (whisper-rs)
  engine/remote.rs        client HTTP compatible OpenAI
server/                   serveur de transcription (Python, FastAPI, faster-whisper, Docker)
tests/wayland-kde/        test d'intégration du portail des raccourcis sous KDE Wayland (Docker)
.github/workflows/        CI, release et workflow manuel sur le runner GPU
.github/scripts/          publication : page de téléchargement, liens latest/, manifestes
```

## Déroulement d'une dictée

```
raccourci pressé   ─► état recording, pastille visible, son de départ, Recorder::start
raccourci relâché  ─► mode hybride : appui de moins de 300 ms → mains libres, on attend un second appui
                   ─► Recorder::stop ─► état transcribing
                   ─► LocalEngine::transcribe (thread bloquant) ou RemoteEngine::transcribe (async)
                   ─► postprocess::finalize : espaces, mot de validation, ponctuation dictée, textes courts
                   ─► inject::deliver (collage, puis Entrée si mot de validation) ─► historique
                   ─► événement dictation-result, état idle, pastille masquée
Échap pendant l'enregistrement ─► cancel_dictation (X11, macOS, Windows)
silence prolongé en bascule ou mains libres ─► Stop, comme un second appui
enregistrement de moins de 250 ms, ou silence ─► abandon sans message
```

**Fonction de niveau du `Recorder`.** La closure passée à `Recorder::start` vit dans le thread audio
jusqu'à la fin de la capture. Elle porte deux choses en plus du niveau envoyé à la pastille :

- `AutoStop` (`pipeline.rs`), qui nourrit `silence::SilenceDetector`. Le détecteur ne compte le silence
  qu'après de la parole, avec un seuil adapté au bruit de fond (plancher suivi au minimum, parole à
  12 dB au-dessus, hystérésis de 4 dB). Quand `auto_stop_silence_ms` est écoulé, et seulement si la dictée
  est en mains libres (`Dictation::hands_free`, vrai en bascule et après un appui bref en hybride), il
  envoie `Command::Stop`, une seule fois.
- `ducking::Duck`, si `duck_volume` est vrai. Le volume baisse 250 ms après le début (le son de départ
  reste audible) et revient quand la closure est détruite : fin normale, annulation (`Recorder` abandonné)
  ou micro en échec. Le volume d'origine est aussi écrit dans `volume-restore.json`, que
  `ducking::recover` relit au démarrage après un arrêt brutal. Le volume n'est rétabli que s'il n'a pas
  été modifié entre-temps.

**Mise en forme (`postprocess.rs`).** Sur le texte de Whisper aux espaces normalisés :

1. Mot de validation (`submit_word_enabled`, `submit_words`) : comparé aux derniers mots, sans casse,
   accents ni ponctuation autour, une lettre d'écart admise à partir de cinq lettres. Il est retiré avec
   la virgule ou les deux-points qui le précèdent, et `submit` passe à vrai.
2. Ponctuation dictée (`punctuation::apply_fr`).
3. Textes courts (`short_text_plain`) : si le résultat compte trois mots ou moins, la règle est
   rejouée sur le texte de Whisper sans son point final (un seul point, pas « … ») et sans la majuscule
   de tête, sauf sigle, casse mixte, lettre seule ou mot écrit ainsi dans le vocabulaire. Comme la
   ponctuation dictée passe ensuite, « merci point » donne toujours « Merci. ».

`inject::deliver(…, submit)` appuie sur Entrée 150 ms après le collage, par la même voie que Ctrl+V :
enigo (`Key::Return`), portail RemoteDesktop (`XK_Return`), `ydotool` (code 28) ou `wtype -k Return`.
Rien n'est envoyé en mode copie seule ou si le collage a échoué. Un mot de validation dit seul envoie
uniquement Entrée, sans entrée d'historique.

## Réglages

Fichier `settings.json` dans le dossier de configuration de l'application (identifiant
`fr.rdagency.murmure`). Tous les champs ont une valeur par défaut : un fichier absent ou corrompu donne les
réglages par défaut.

```jsonc
{
  "engine": "local",                       // "local" | "remote"
  "local_model": "large-v3-turbo-q5_0",    // identifiant du catalogue de models.rs
  "language": "fr",                        // "auto" ou code ISO 639-1
  "initial_prompt": "",                    // vocabulaire : noms propres, termes techniques
  "spoken_punctuation": true,              // ponctuation dictée (langue fr ou auto)
  "short_text_plain": true,                // trois mots ou moins : ni point final ni majuscule imposée
  "remote_endpoint": "",                   // ex. https://stt.exemple.fr/v1/audio/transcriptions
  "remote_model": "whisper-large-v3-turbo",
  "hotkey": "Alt+Space",                   // format accelerator de Tauri ; "Ctrl+Alt+Space" hors macOS
  "hotkey_mode": "hybrid",                 // "hybrid" | "hold" | "toggle"
  "auto_stop_silence_ms": 2000,            // bascule ou mains libres : fin après ce silence ; 0 = jamais
  "input_device": null,                    // null = micro par défaut, sinon nom du périphérique
  "duck_volume": false,                    // baisse le volume de sortie pendant l'enregistrement
  "paste_mode": "paste",                   // "paste" (presse-papiers puis Cmd/Ctrl+V) | "clipboard" (copie seule)
  "restore_clipboard": true,
  "submit_word_enabled": false,            // mot de validation : retiré, puis Entrée après le collage
  "submit_words": ["terminé", "envoyer"],
  "sounds": true,
  "launch_at_login": false,
  "history_enabled": true,
  "onboarding_done": false,
  "wayland_restore_token": null            // jeton du portail RemoteDesktop, géré par Rust
}
```

La clé API n'est jamais écrite dans `settings.json`. Elle est rangée dans le trousseau du système (crate
`keyring`, service `fr.rdagency.murmure`, compte `remote_api_key`).

### Export et import (`config_transfer.rs`)

```jsonc
{
  "format": "murmure-config",
  "version": 1,                            // un fichier de version supérieure est refusé
  "app_version": "0.1.6",
  "exported_at": "2026-10-15T21:00:00+02:00",
  "settings": { … },                       // Settings sans onboarding_done ni wayland_restore_token
  "secrets": { "remote_api_key": "…" }     // seulement sur demande explicite
}
```

À l'import, chaque champ présent remplace la valeur actuelle, les champs absents ou inconnus sont ignorés,
`onboarding_done` et `wayland_restore_token` restent ceux du poste. Une valeur invalide (énumération,
raccourci) fait échouer l'import sans rien modifier. Dans l'interface, les réglages importés passent par le
même chemin que `save_settings` (raccourci, lancement au démarrage, moteur). Un fichier qui contient un
secret est écrit avec les droits 600 sous Unix.

`murmure --export-config fichier.json [--with-secrets]` et `murmure --import-config fichier.json` sont
traités avant Tauri, comme `--install-model`. L'import en ligne de commande écrit `settings.json` et le
trousseau : une instance déjà ouverte garde ses réglages en mémoire jusqu'au prochain lancement. Codes de
sortie : 0 succès, 1 échec, 2 usage.

## Commandes (front vers Rust)

Les noms sont en snake_case côté Rust, les arguments en camelCase côté JavaScript (convention de Tauri).
Toutes renvoient `Result<T, String>` : le message d'erreur est affiché tel quel, il doit donc être rédigé
en français pour l'utilisateur.

| Commande | Arguments | Retour |
|---|---|---|
| `get_settings` | — | `Settings` |
| `save_settings` | `settings` | `()` ; réapplique le raccourci et le lancement au démarrage, recharge le moteur si besoin |
| `set_api_key` | `key` (vide = suppression) | `()` |
| `has_api_key` | — | `bool` |
| `test_remote` | `endpoint`, `apiKey`, `model` | message de succès avec la latence |
| `list_input_devices` | — | `AudioDevice[]` |
| `list_models` | — | `ModelInfo[]` |
| `download_model` | `id` | `()` ; progression par événement ; rejoint un téléchargement déjà en cours |
| `get_downloads` | — | `DownloadStatus[]`, y compris ceux lancés par Rust au démarrage |
| `cancel_download`, `delete_model` | `id` | `()` |
| `start_dictation`, `stop_dictation`, `cancel_dictation`, `toggle_dictation` | — | `()` |
| `get_history` | — | `HistoryEntry[]`, du plus récent au plus ancien |
| `delete_history_entry` | `id` | `()` |
| `clear_history` | — | `()` |
| `copy_text` | `text` | `()` |
| `check_permissions`, `request_microphone` | — | `Permissions` |
| `open_permission_settings` | `kind` : `"microphone"` ou `"accessibility"` | `()` |
| `get_platform` | — | `"macos"`, `"linux"` ou `"windows"` |
| `get_acceleration` | — | par exemple « Metal · Apple M2 », « Vulkan · NVIDIA GeForce RTX 5050 », « Processeur » |
| `check_update` | — | `UpdateInfo` ou `null` (à jour, manifeste absent, ou build de développement) ; erreur si le réseau est injoignable |
| `install_update` | — | télécharge, installe et relance ; refusé pendant une dictée |
| `get_wayland_status` | — | `WaylandStatus` (`wayland: false` hors session Wayland) |
| `wayland_authorize` | — | `WaylandStatus` ; déclenche les demandes de portail (raccourci, clavier) |
| `wayland_configure_shortcut` | — | `()` ; ouvre le réglage du raccourci du bureau (`ConfigureShortcuts`) |
| `wayland_refresh` | — | `WaylandStatus` ; relit le raccourci attribué et l'outil de collage de secours |

## Événements (Rust vers front)

| Événement | Contenu |
|---|---|
| `dictation-state` | `{ state: "idle" \| "recording" \| "transcribing" \| "error", message?, hands_free? }` |
| `audio-level` | niveau de 0 à 1, environ 30 fois par seconde pendant l'enregistrement |
| `dictation-result` | `HistoryEntry` |
| `model-download-progress` | `{ id, downloaded, total, done, error? }` |
| `settings-changed` | `Settings` |
| `navigate` | `"home"`, `"history"` ou `"settings"`, depuis le menu de la barre de menus |
| `update-available` | `UpdateInfo` |
| `update-progress` | `{ downloaded, total }` en octets (`total` vaut 0 s'il est inconnu), environ 10 fois par seconde |
| `wayland-status` | `WaylandStatus`, à chaque changement d'état des portails |
| `file-transcription-progress` | `{ stage: "decoding" \| "transcribing", fraction, done, total }` ; `fraction` de 0 à 1, segments faits et total |

## Types partagés

```ts
type AudioDevice    = { name: string; is_default: boolean };
type ModelInfo      = { id: string; name: string; description: string; size_mb: number;
                        downloaded: boolean; recommended: boolean; path: string | null };
type HistoryEntry   = { id: string; text: string; created_at: string /* RFC 3339 */;
                        duration_ms: number; latency_ms: number; engine: "local" | "remote"; model: string };
type DownloadStatus = { id: string; downloaded: number; total: number };
type Permissions    = { microphone: "granted" | "denied" | "undetermined" | "unknown";
                        accessibility: "granted" | "denied" | "unknown" };
type UpdateInfo     = { version: string; notes: string | null };
type PortalState    = "inactive" | "pending" | "active" | "denied" | "unavailable";
type WaylandStatus  = { wayland: boolean; shortcut: PortalState; shortcut_trigger: string | null;
                        shortcut_configurable: boolean; shortcut_error: string | null;
                        keyboard: PortalState; keyboard_error: string | null;
                        paste_fallback: "ydotool" | "wtype" | null };
type FileTranscript = { text: string; file_name: string; duration_ms: number; latency_ms: number };
```

`src/lib/types.ts` doit rester aligné sur les structures Rust : un champ ajouté d'un côté s'ajoute de
l'autre, et dans `src/lib/mock.ts`.

## Interfaces entre modules Rust

```rust
// settings.rs
pub fn load(config_dir: &Path) -> Settings;                     // valeurs par défaut si absent ou corrompu
pub fn save(config_dir: &Path, s: &Settings) -> anyhow::Result<()>;
pub fn get_api_key() -> Option<String>;
pub fn set_api_key(key: &str) -> anyhow::Result<()>;            // "" = suppression

// audio.rs
pub fn list_input_devices() -> Vec<AudioDevice>;
impl Recorder {
    /// Démarre la capture ; `on_level` est appelé environ 30 fois par seconde, hors du thread temps réel.
    pub fn start(device: Option<&str>, on_level: impl Fn(f32) + Send + 'static) -> anyhow::Result<Recorder>;
    /// Arrête et renvoie l'audio en mono 16 kHz, f32.
    pub fn stop(self) -> anyhow::Result<Vec<f32>>;
}

// models.rs
pub const RECOMMENDED_ID: &str;                                  // "large-v3-turbo-q5_0"
pub fn list(data_dir: &Path) -> Vec<ModelInfo>;
pub fn model_path(data_dir: &Path, id: &str) -> Option<PathBuf>; // Some si le fichier est présent
pub fn resolve_id(id: &str) -> &'static str;                     // identifiant inconnu → recommandé
pub async fn download(data_dir: &Path, id: &str, cancel: Arc<AtomicBool>,
                      on_progress: impl Fn(u64, u64) + Send + 'static) -> anyhow::Result<PathBuf>;
pub fn delete(data_dir: &Path, id: &str) -> anyhow::Result<()>;

// engine/
pub struct TranscribeOptions { pub language: Option<String> /* None = auto */, pub initial_prompt: Option<String> }
impl LocalEngine {
    pub fn load(model_path: &Path) -> anyhow::Result<LocalEngine>;   // lent : hors du thread de l'interface
    pub fn transcribe(&self, samples: &[f32], opts: &TranscribeOptions) -> anyhow::Result<String>;
}
impl RemoteEngine {
    pub async fn transcribe(&self, samples: &[f32], opts: &TranscribeOptions) -> anyhow::Result<String>;
    pub async fn test(&self) -> anyhow::Result<String>;            // envoie une seconde de silence
}
pub fn encode_wav(samples: &[f32]) -> Vec<u8>;                     // PCM 16 bits, mono, 16 kHz
```

`models::download` vérifie l'espace disque avant de commencer, reprend un fichier `.part` interrompu
(requête `Range`), calcule le SHA-256 au fil de l'eau et contrôle taille et empreinte à la fin. Après une
erreur réseau, il réessaie trois fois (après 5, 15 puis 45 secondes). Ses erreurs sont des
`models::DownloadError` (`Cancelled`, `Network`, `Corrupt`, `NoSpace`, `Fatal`) dans un `anyhow::Error`.

## Installation du modèle (`install.rs`)

Le modèle s'installe sans intervention : au démarrage, quand les réglages passent au moteur local ou
changent de modèle, et lors d'une dictée sans modèle. Rien n'est téléchargé si le modèle est déjà présent,
dans le dossier de Murmure ou dans `~/.whisper/models`.

Un seul téléchargement par modèle à la fois (`state::Downloads`) : un clic dans l'interface rejoint celui
qui est en cours. Une dictée pendant l'installation affiche « Installation du modèle en cours (42 %) —
réessayez dans un instant ». À la fin, si le modèle choisi manque, Murmure sélectionne celui qui vient
d'être installé, puis précharge le moteur. La fenêtre principale ne s'ouvre d'elle-même qu'en cas d'échec.

`murmure --install-model [id]` (ou `--install-model=id`) est traité avant le démarrage de Tauri, sans
fenêtre. La progression s'affiche sur la sortie standard. Codes de sortie : 0 succès, 1 échec, 2 identifiant
inconnu. La variable `MURMURE_DATA_DIR` remplace le dossier de données (modèles, historique), pour l'app
comme pour cette commande.

## Mises à jour (`updater.rs`)

Le manifeste est `https://crgenerator.s3.eu-west-par.io.cloud.ovh.net/murmure/latest.json`, au format de
Tauri 2, publié par la release. La clé publique minisign est dans `tauri.conf.json`
(`plugins.updater.pubkey`) ; la clé privée n'existe que dans les secrets de la CI.

La vérification a lieu dix secondes après le démarrage puis toutes les heures, jamais pendant une dictée,
et jamais en build de développement. Une mise à jour trouvée est téléchargée en silence et sa signature
vérifiée. Elle est installée sur disque (macOS, AppImage) ou gardée en mémoire (Windows). Le menu de la
barre de menus propose alors de redémarrer.

Quand l'application est inactive depuis deux minutes (aucune dictée, aucun téléchargement, fenêtre
principale fermée), Murmure pose le marqueur `relaunch-hidden` et redémarre ; au lancement suivant, le
marqueur est consommé et l'application démarre sans fenêtre. Avec les paquets .deb et .rpm, l'installation
demande les droits administrateur : la mise à jour est seulement proposée, avec un bouton.

Un manifeste absent (404) ou un réseau coupé ne produit qu'un message de niveau debug.

## Variantes Linux

**NVIDIA** (`--features cuda`, paquets `…-linux-x64-cuda`) : `libcudart`, `libcublas` et `libcublasLt`
(CUDA 12) sont livrées dans `/usr/lib/murmure/cuda`, trouvées grâce au rpath posé par `build.rs`. Seul le
pilote NVIDIA est requis. Au build, `MURMURE_CUDA_LIB_DIR` impose le dossier des bibliothèques CUDA utilisé pour l'édition de liens,
utile quand `/usr/local/cuda` pointe vers une autre version que celle du compilateur.
Ces bibliothèques sont propriétaires : voir [LICENSE-EXCEPTION-CUDA.md](LICENSE-EXCEPTION-CUDA.md).

**Vulkan** (`--features vulkan`, paquets `…-linux-x64-vulkan`, pas d'AppImage) : rien n'est embarqué. Le
paquet dépend de `libvulkan1` (.deb) ou de `libvulkan.so.1()(64bit)` (.rpm). Sans pilote Vulkan,
l'initialisation échoue proprement (`ggml_backend_vk_reg` renvoie un pointeur nul) ; un pilote purement
logiciel comme lavapipe est ignoré par ggml. Dans les deux cas, la transcription se fait sur le processeur,
avec un avertissement dans le journal.

Chaque variante a son manifeste (`cuda/latest.json`, `vulkan/latest.json`), pour qu'une installation GPU
ne bascule jamais vers la variante standard.

## Wayland (`wayland.rs`, `portal.rs`)

Sous Wayland, le greffon `tauri-plugin-global-shortcut` (X11) ne reçoit aucune touche et enigo ou xdotool
n'atteignent pas les fenêtres natives. Murmure passe donc par les portails XDG, avec la crate `ashpd`
(D-Bus en Rust pur via `zbus`, sur le runtime tokio de Tauri).

**Détection.** Session Wayland si `XDG_SESSION_TYPE=wayland`, ou si `WAYLAND_DISPLAY` est défini sans
`XDG_SESSION_TYPE=x11`. Sous X11, macOS et Windows, rien ne change.

**Identifiant d'application.** Une application hors bac à sable n'a pas d'identifiant pour le portail,
sauf si le bureau l'a lancée dans une unité `app-Murmure@….service`. Sans identifiant, KDE crée un
composant `token_…` à chaque lancement et redemande le raccourci. Murmure appelle donc
`org.freedesktop.host.portal.Registry.Register("Murmure")` (xdg-desktop-portal 1.19 ou plus récent), ce qui
exige un fichier `Murmure.desktop` : il est fourni par les paquets, et créé dans
`~/.local/share/applications` s'il manque (AppImage). Un échec n'est pas bloquant.

**Raccourci** (`org.freedesktop.portal.GlobalShortcuts`, KDE Plasma 5.27 et GNOME 48 ou plus récents).
Murmure crée une session et lie un raccourci `dictation`, dont le déclencheur proposé vient du réglage
(`wayland::accelerator_to_portal` : `Ctrl+Alt+Space` devient `CTRL+ALT+space`). KDE n'affiche sa boîte de
dialogue que pour un raccourci nouveau. Les signaux `Activated` et `Deactivated` sont transmis à
`hotkey::on_key_event` : les modes hybride, maintien et bascule se comportent comme sous X11. Le raccourci
réellement attribué est relu (réponse de `BindShortcuts`, `ListShortcuts` au retour du focus, signal
`ShortcutsChanged`). Dans ce mode, `hotkey::apply` n'enregistre rien côté X11, sinon les fenêtres XWayland
recevraient un double appui.

**Collage** (`org.freedesktop.portal.RemoteDesktop`). Session clavier seul, `persist_mode` à 2 (jusqu'à
révocation) ; le `restore_token`, à usage unique, est renouvelé à chaque session et stocké dans
`wayland_restore_token`, si bien que la boîte de dialogue n'apparaît qu'une fois. Ctrl+V est envoyé par
`NotifyKeyboardKeysym` (`Control_L`, `v`), indépendamment de la disposition du clavier. L'ordre de repli
est fixé par `inject::paste_plan`, testé : portail, puis `ydotool` si le socket de `ydotoold` répond, puis
`wtype` (refusé par KWin), et enfin copie seule avec une notification « Texte copié ».

**Presse-papiers.** `arboard` avec `wayland-data-control` (`ext-data-control-v1` et
`zwlr-data-control-v1`, pour KWin et wlroots), avec repli X11 par XWayland ailleurs, par exemple sous
GNOME. Un thread de fond sert le contenu tant que Murmure tourne.

**Moment des demandes.** Au démarrage, si l'assistant est terminé, le raccourci est rebranché (sans boîte
de dialogue s'il est déjà accordé) et la session clavier restaurée si un jeton existe. Sinon, les demandes
partent du bouton « Autoriser Murmure » (assistant et Réglages › Raccourci), via `wayland_authorize`.

**Repli.** Sans portail des raccourcis, ou après un refus, l'utilisateur lie `murmure --toggle` à un
raccourci du bureau (procédure affichée dans Réglages › Raccourci). Ce mode est une bascule.

## Transcription de fichiers (`audio_file.rs`, `file_transcription.rs`)

Bouton « Transcrire un fichier… » de l'accueil. Un seul fichier à la fois (`FileJob`, géré par Tauri).

1. **Décodage** en Rust pur, sans garder le signal d'origine : Symphonia (WAV PCM et ADPCM, MP3, AAC et
   ALAC en M4A/MP4, Ogg Vorbis, FLAC), mixage en mono et rééchantillonnage par blocs vers 16 kHz
   (`StreamResampler`, rubato, latence compensée à la fin). Opus, absent de Symphonia, est démultiplexé
   par Symphonia (Ogg) et décodé par `opus-decoder` directement en mono 16 kHz. Au-delà de trois heures,
   le fichier est refusé (environ 700 Mo de signal en mémoire). `opus-decoder` reprend l'arithmétique de
   la référence C : ses débordements volontaires feraient paniquer le profil de développement, d'où
   `overflow-checks = false` pour ce paquet dans `Cargo.toml`.
2. **Découpage** (`split_on_silence`) : segments d'environ 20 s, coupés au point le plus calme (trames de
   30 ms) entre 15 et 25 s, toujours sous la fenêtre de 30 s de Whisper. Ce découpage est propre aux
   fichiers et indépendant de celui de la dictée.
3. **Transcription** segment par segment avec le moteur choisi, la langue et le vocabulaire des réglages ;
   les segments silencieux sont sautés. L'annulation est vérifiée entre deux segments.
4. **Texte** : segments joints, puis `postprocess::finalize` avec la ponctuation dictée (sans règle des
   textes courts ni mot de validation). Le texte n'est ni collé ni ajouté à l'historique ; l'accueil
   l'affiche avec un bouton « Copier le texte ».

Les fichiers de `src-tauri/testdata/` (une seconde de sinus dans chaque format) servent aux tests de
décodage. Test de bout en bout sur un vrai fichier :
`MURMURE_TEST_AUDIO=dictee.m4a cargo test --release --lib file_transcription -- --ignored --nocapture`.

## Baisse du volume (`ducking.rs`)

Volume du périphérique de sortie par défaut, ramené à 30 % de sa valeur pendant l'enregistrement.

- **Linux** : `wpctl get-volume` / `set-volume @DEFAULT_AUDIO_SINK@` (PipeWire), sinon
  `pactl get-sink-volume` / `set-sink-volume @DEFAULT_SINK@` (PulseAudio, ou PipeWire avec
  `pipewire-pulse`). Sans l'un des deux, rien ne se passe. `pactl` applique le même volume à tous les
  canaux : une balance gauche-droite réglée à la main est perdue.
- **macOS** : CoreAudio, propriété `kAudioHardwareServiceDeviceProperty_VirtualMainVolume` (celle de la
  barre de menus), à défaut `kAudioDevicePropertyVolumeScalar` de l'élément principal ou des canaux 1 et
  2. Certains périphériques (sortie HDMI, interfaces audio sans volume matériel) n'en exposent aucune :
  le réglage est alors sans effet.
- **Windows** : `IAudioEndpointVolume` (Core Audio) du périphérique de rendu par défaut, rôle console.
- Un son déjà coupé n'est pas touché.

## Ponctuation dictée (`punctuation.rs`)

Active si `spoken_punctuation` est vrai et que la langue est `fr` ou `auto`. Sont convertis : « virgule »,
« point », « point à la ligne », « à la ligne », « nouveau paragraphe », « deux points », « point-virgule »,
« point d'interrogation », « point d'exclamation », « points de suspension », « ouvrez / fermez les
guillemets », « ouvrez / fermez la parenthèse ». La typographie française et les majuscules sont
recomposées.

« Point » seul n'est converti qu'en fin de phrase ou quand Whisper l'a isolé entre deux signes, jamais
après un déterminant (« caler un point jeudi », « le point de vue ») ; même règle pour « virgule » et
« deux points ». Avec la langue `fr`, un exemple de dictée ponctuée (`PROMPT_FR`) est ajouté au prompt
initial de Whisper : sans lui, « deux points » devient « 2. » et « à la ligne » est mal reconnu.

## Interface

Interface sobre, typographie du système, une seule couleur d'accent, thèmes clair et sombre automatiques.
Tous les textes sont en français.

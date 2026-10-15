// SPDX-License-Identifier: AGPL-3.0-or-later
// Miroir exact du contrat Rust (ARCHITECTURE.md). Ne pas diverger.

export type EngineKind = "local" | "remote";
export type HotkeyMode = "hybrid" | "hold" | "toggle";
export type PasteMode = "paste" | "clipboard";
export type Platform = "macos" | "linux" | "windows";
export type View = "home" | "history" | "settings";

export interface Settings {
  engine: EngineKind;
  local_model: string;
  /** "auto" ou code ISO 639-1 */
  language: string;
  initial_prompt: string;
  /** « virgule », « point à la ligne »… convertis en signes (français) */
  spoken_punctuation: boolean;
  remote_endpoint: string;
  remote_model: string;
  /** Format accelerator Tauri, ex. "Alt+Space" */
  hotkey: string;
  hotkey_mode: HotkeyMode;
  /** null = micro par défaut */
  input_device: string | null;
  paste_mode: PasteMode;
  restore_clipboard: boolean;
  sounds: boolean;
  launch_at_login: boolean;
  history_enabled: boolean;
  onboarding_done: boolean;
  /** Linux Wayland : jeton du portail RemoteDesktop, géré par Rust (ignoré à l'enregistrement). */
  wayland_restore_token: string | null;
}

export interface AudioDevice {
  name: string;
  is_default: boolean;
}

export interface ModelInfo {
  id: string;
  name: string;
  description: string;
  size_mb: number;
  downloaded: boolean;
  recommended: boolean;
  path: string | null;
}

export interface HistoryEntry {
  id: string;
  text: string;
  /** RFC3339 */
  created_at: string;
  duration_ms: number;
  latency_ms: number;
  engine: EngineKind;
  model: string;
}

export interface Permissions {
  microphone: "granted" | "denied" | "undetermined" | "unknown";
  accessibility: "granted" | "denied" | "unknown";
}

export type PermissionKind = "microphone" | "accessibility";

export type DictationState = "idle" | "recording" | "transcribing" | "error";

export interface DictationStatePayload {
  state: DictationState;
  message?: string;
  hands_free?: boolean;
}

/** Téléchargement en cours (commande `get_downloads`). */
export interface DownloadStatus {
  id: string;
  downloaded: number;
  total: number;
}

export interface DownloadProgress {
  id: string;
  downloaded: number;
  total: number;
  done: boolean;
  error?: string;
}

/** Mise à jour disponible (événement `update-available`, retour de `check_update`). */
export interface UpdateInfo {
  version: string;
  notes: string | null;
}

/** Progression du téléchargement de la mise à jour (octets cumulés ; total = 0 si inconnu). */
export interface UpdateProgress {
  downloaded: number;
  total: number;
}

/** État d'une autorisation de portail XDG (Wayland). */
export type PortalState = "inactive" | "pending" | "active" | "denied" | "unavailable";

/** Linux Wayland : raccourci global (GlobalShortcuts) et collage (RemoteDesktop) via les portails. */
export interface WaylandStatus {
  wayland: boolean;
  shortcut: PortalState;
  /** Raccourci réellement attribué par le bureau (ex. « Ctrl+Alt+Espace »). */
  shortcut_trigger: string | null;
  /** Le bureau sait ouvrir son réglage du raccourci (ConfigureShortcuts). */
  shortcut_configurable: boolean;
  shortcut_error: string | null;
  keyboard: PortalState;
  keyboard_error: string | null;
  /** Outil de collage de secours détecté. */
  paste_fallback: "ydotool" | "wtype" | null;
}

/** Événements Rust → front et leur payload. */
export interface EventMap {
  "dictation-state": DictationStatePayload;
  "audio-level": number;
  "dictation-result": HistoryEntry;
  "model-download-progress": DownloadProgress;
  "settings-changed": Settings;
  navigate: View;
  "update-available": UpdateInfo;
  /** Mise à jour téléchargée et en place : appliquée au prochain redémarrage (automatique). */
  "update-ready": UpdateInfo;
  "update-progress": UpdateProgress;
  "wayland-status": WaylandStatus;
}

export type EventName = keyof EventMap;

/** Commandes front → Rust : arguments (camelCase côté JS) et retour. */
export interface CommandMap {
  get_settings: { args: undefined; ret: Settings };
  save_settings: { args: { settings: Settings }; ret: void };
  set_api_key: { args: { key: string }; ret: void };
  has_api_key: { args: undefined; ret: boolean };
  test_remote: { args: { endpoint: string; apiKey: string | null; model: string }; ret: string };
  list_input_devices: { args: undefined; ret: AudioDevice[] };
  list_models: { args: undefined; ret: ModelInfo[] };
  download_model: { args: { id: string }; ret: void };
  cancel_download: { args: { id: string }; ret: void };
  get_downloads: { args: undefined; ret: DownloadStatus[] };
  delete_model: { args: { id: string }; ret: void };
  start_dictation: { args: undefined; ret: void };
  stop_dictation: { args: undefined; ret: void };
  cancel_dictation: { args: undefined; ret: void };
  toggle_dictation: { args: undefined; ret: void };
  get_history: { args: undefined; ret: HistoryEntry[] };
  delete_history_entry: { args: { id: string }; ret: void };
  clear_history: { args: undefined; ret: void };
  copy_text: { args: { text: string }; ret: void };
  check_permissions: { args: undefined; ret: Permissions };
  request_microphone: { args: undefined; ret: Permissions };
  open_permission_settings: { args: { kind: PermissionKind }; ret: void };
  get_platform: { args: undefined; ret: Platform };
  get_acceleration: { args: undefined; ret: string };
  check_update: { args: undefined; ret: UpdateInfo | null };
  install_update: { args: undefined; ret: void };
  get_wayland_status: { args: undefined; ret: WaylandStatus };
  wayland_authorize: { args: undefined; ret: WaylandStatus };
  wayland_configure_shortcut: { args: undefined; ret: void };
  wayland_refresh: { args: undefined; ret: WaylandStatus };
}

export type CommandName = keyof CommandMap;

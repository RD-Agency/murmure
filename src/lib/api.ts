// SPDX-License-Identifier: AGPL-3.0-or-later
// Seule porte vers Rust : wrappers typés autour de invoke/listen.
// Hors Tauri (navigateur), bascule automatiquement sur un mock (src/lib/mock.ts).

import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  CommandMap,
  CommandName,
  EventMap,
  EventName,
  PermissionKind,
  Settings,
} from "./types";

export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
export const isMock = !isTauri;

type MockModule = typeof import("./mock");
let mockModule: Promise<MockModule> | null = null;
export function loadMock(): Promise<MockModule> {
  return (mockModule ??= import("./mock"));
}

type ArgsOf<K extends CommandName> = CommandMap[K]["args"] extends undefined
  ? []
  : [CommandMap[K]["args"]];

async function call<K extends CommandName>(
  cmd: K,
  ...args: ArgsOf<K>
): Promise<CommandMap[K]["ret"]> {
  const payload = args[0] as Record<string, unknown> | undefined;
  if (isTauri) return invoke<CommandMap[K]["ret"]>(cmd, payload);
  const m = await loadMock();
  return m.mockInvoke(cmd, payload) as Promise<CommandMap[K]["ret"]>;
}

/** Abonnement à un événement Rust. Renvoie une fonction de désabonnement synchrone. */
export function on<E extends EventName>(
  event: E,
  handler: (payload: EventMap[E]) => void,
): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  const ready: Promise<() => void> = isTauri
    ? listen<EventMap[E]>(event, (e) => handler(e.payload))
    : loadMock().then((m) => m.mockListen(event, handler));
  ready.then((fn) => {
    if (disposed) fn();
    else unlisten = fn;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/** Message lisible depuis une erreur `Result<T, String>` côté Rust. */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Erreur inconnue";
}

export const api = {
  getSettings: () => call("get_settings"),
  saveSettings: (settings: Settings) => call("save_settings", { settings }),
  setApiKey: (key: string) => call("set_api_key", { key }),
  hasApiKey: () => call("has_api_key"),
  testRemote: (endpoint: string, apiKey: string | null, model: string) =>
    call("test_remote", { endpoint, apiKey, model }),
  listInputDevices: () => call("list_input_devices"),
  listModels: () => call("list_models"),
  downloadModel: (id: string) => call("download_model", { id }),
  cancelDownload: (id: string) => call("cancel_download", { id }),
  getDownloads: () => call("get_downloads"),
  deleteModel: (id: string) => call("delete_model", { id }),
  startDictation: () => call("start_dictation"),
  stopDictation: () => call("stop_dictation"),
  cancelDictation: () => call("cancel_dictation"),
  toggleDictation: () => call("toggle_dictation"),
  getHistory: () => call("get_history"),
  deleteHistoryEntry: (id: string) => call("delete_history_entry", { id }),
  clearHistory: () => call("clear_history"),
  copyText: (text: string) => call("copy_text", { text }),
  checkPermissions: () => call("check_permissions"),
  requestMicrophone: () => call("request_microphone"),
  openPermissionSettings: (kind: PermissionKind) =>
    call("open_permission_settings", { kind }),
  getPlatform: () => call("get_platform"),
  /** Accélération du moteur local (« Vulkan · NVIDIA GeForce RTX 5050 », « Processeur »…). */
  getAcceleration: () => call("get_acceleration"),
  checkUpdate: () => call("check_update"),
  /** Télécharge (événements `update-progress`), installe puis relance l'app. */
  installUpdate: () => call("install_update"),
  getWaylandStatus: () => call("get_wayland_status"),
  /** Wayland : demandes de portail (raccourci global + contrôle du clavier). */
  waylandAuthorize: () => call("wayland_authorize"),
  waylandConfigureShortcut: () => call("wayland_configure_shortcut"),
  waylandRefresh: () => call("wayland_refresh"),
  /** Exporte les réglages (boîte d'enregistrement) ; la clé API seulement si demandé. */
  exportConfig: (includeSecrets: boolean) => call("export_config", { includeSecrets }),
  importConfig: () => call("import_config"),
  pickAudioFile: () => call("pick_audio_file"),
  /** Progression par l'événement `file-transcription-progress`. */
  transcribeFile: (path: string) => call("transcribe_file", { path }),
  cancelFileTranscription: () => call("cancel_file_transcription"),
  /** Version de l'app en cours d'exécution. */
  getVersion: async (): Promise<string> => (isTauri ? getVersion() : (await loadMock()).MOCK_VERSION),
};

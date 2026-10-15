// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";
import { api, errorMessage, on } from "./lib/api";
import type {
  DownloadProgress,
  DownloadStatus,
  FileProgress,
  FileTranscript,
  HistoryEntry,
  ModelInfo,
  Platform,
  Settings,
  UpdateInfo,
  UpdateProgress,
  View,
  WaylandStatus,
} from "./lib/types";

export type SaveStatus = "idle" | "saving" | "saved" | "error";

/** idle : rien vérifié · checking · uptodate · available · installing · error */
export type UpdateStatus = "idle" | "checking" | "uptodate" | "available" | "installing" | "ready" | "error";

export interface UpdateSlice {
  status: UpdateStatus;
  available: UpdateInfo | null;
  progress: UpdateProgress | null;
  error: string | null;
}

/** Transcription d'un fichier audio, gardée ici pour survivre à un changement d'écran. */
export interface FileJobSlice {
  status: "idle" | "running" | "done" | "error";
  name: string | null;
  progress: FileProgress | null;
  result: FileTranscript | null;
  error: string | null;
}

const FILE_JOB_IDLE: FileJobSlice = { status: "idle", name: null, progress: null, result: null, error: null };

interface AppState {
  ready: boolean;
  platform: Platform;
  settings: Settings | null;
  saveStatus: SaveStatus;
  saveError: string | null;
  view: View;
  models: ModelInfo[];
  downloads: Record<string, DownloadProgress>;
  downloadErrors: Record<string, string>;
  history: HistoryEntry[];
  hasApiKey: boolean;
  update: UpdateSlice;
  /** Linux Wayland : état des portails (null hors Tauri/avant init). */
  wayland: WaylandStatus | null;
  fileJob: FileJobSlice;

  init: () => Promise<void>;
  setView: (v: View) => void;
  /** Modifie les réglages localement puis sauvegarde (debounce en ms pour les champs texte). */
  updateSettings: (patch: Partial<Settings>, debounceMs?: number) => void;
  flushSettings: () => Promise<void>;
  refreshModels: () => Promise<void>;
  refreshHistory: () => Promise<void>;
  refreshApiKey: () => Promise<void>;
  downloadModel: (id: string) => Promise<void>;
  cancelDownload: (id: string) => Promise<void>;
  deleteModel: (id: string) => Promise<void>;
  deleteHistoryEntry: (id: string) => Promise<void>;
  clearHistory: () => Promise<void>;
  checkUpdate: () => Promise<void>;
  installUpdate: () => Promise<void>;
  /** Choisit un fichier audio puis le transcrit. */
  transcribeFile: () => Promise<void>;
  cancelFileTranscription: () => Promise<void>;
  dismissFileJob: () => void;
  /** Importe un fichier de réglages ; renvoie false si l'utilisateur a annulé. */
  importConfig: () => Promise<boolean>;
}

let saveTimer: ReturnType<typeof setTimeout> | null = null;
let savedTimer: ReturnType<typeof setTimeout> | null = null;
let pendingLocal = false;
let initialized = false;

export const useApp = create<AppState>((set, get) => ({
  ready: false,
  platform: "macos",
  settings: null,
  saveStatus: "idle",
  saveError: null,
  view: "home",
  models: [],
  downloads: {},
  downloadErrors: {},
  history: [],
  hasApiKey: false,
  update: { status: "idle", available: null, progress: null, error: null },
  wayland: null,
  fileJob: FILE_JOB_IDLE,

  init: async () => {
    if (initialized) return;
    initialized = true;

    on("settings-changed", (s) => {
      // Ne pas écraser une modification locale en attente de sauvegarde.
      if (!pendingLocal) set({ settings: s });
    });
    on("navigate", (v) => set({ view: v }));
    on("wayland-status", (wayland) => set({ wayland }));
    on("file-transcription-progress", (progress) =>
      set((st) => (st.fileJob.status === "running" ? { fileJob: { ...st.fileJob, progress } } : st)),
    );
    on("update-available", (info) =>
      set((st) =>
        st.update.status === "installing"
          ? { update: { ...st.update, available: info } }
          : { update: { status: "available", available: info, progress: null, error: null } },
      ),
    );
    on("update-ready", (info) => set({ update: { status: "ready", available: info, progress: null, error: null } }));
    on("update-progress", (progress) =>
      // Aussi déclenché depuis le tray : on passe en « installing » à la première progression.
      set((st) => ({ update: { ...st.update, status: "installing", progress, error: null } })),
    );
    on("dictation-result", (entry) => {
      if (get().settings?.history_enabled === false) return;
      set((st) => ({ history: [entry, ...st.history.filter((h) => h.id !== entry.id)] }));
    });
    on("model-download-progress", (p) => {
      set((st) => {
        const downloads = { ...st.downloads };
        const downloadErrors = { ...st.downloadErrors };
        if (p.done) delete downloads[p.id];
        else downloads[p.id] = p;
        if (p.done && p.error && !/annul/i.test(p.error)) downloadErrors[p.id] = p.error;
        else delete downloadErrors[p.id];
        return { downloads, downloadErrors };
      });
      if (p.done) {
        void get()
          .refreshModels()
          .then(() => {
            // Premier modèle téléchargé : le sélectionner si le modèle courant est absent.
            const { settings, models } = get();
            if (p.error || !settings) return;
            const current = models.find((m) => m.id === settings.local_model);
            if (!current?.downloaded && models.find((m) => m.id === p.id)?.downloaded) {
              get().updateSettings({ local_model: p.id });
            }
          });
      }
    });

    void api
      .getWaylandStatus()
      .then((wayland) => set({ wayland }))
      .catch(() => undefined);
    const [platform, settings, models, history, hasApiKey, running] = await Promise.all([
      api.getPlatform().catch(() => "macos" as Platform),
      api.getSettings(),
      api.listModels().catch(() => [] as ModelInfo[]),
      api.getHistory().catch(() => [] as HistoryEntry[]),
      api.hasApiKey().catch(() => false),
      // Téléchargements déjà lancés par Rust (auto-installation au démarrage).
      api.getDownloads().catch(() => [] as DownloadStatus[]),
    ]);
    document.documentElement.dataset.platform = platform;
    set((st) => {
      const downloads = { ...st.downloads };
      for (const d of running) {
        // Un événement de progression a pu arriver pendant l'init : on garde le plus avancé.
        if (!downloads[d.id] || downloads[d.id].downloaded < d.downloaded) downloads[d.id] = { ...d, done: false };
      }
      return { platform, settings, models, history, hasApiKey, downloads, ready: true };
    });
  },

  setView: (view) => set({ view }),

  updateSettings: (patch, debounceMs = 0) => {
    const current = get().settings;
    if (!current) return;
    set({ settings: { ...current, ...patch } });
    pendingLocal = true;
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void get().flushSettings(), debounceMs);
  },

  flushSettings: async () => {
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    const s = get().settings;
    if (!s || !pendingLocal) return;
    pendingLocal = false;
    set({ saveStatus: "saving", saveError: null });
    try {
      await api.saveSettings(s);
      if (savedTimer) clearTimeout(savedTimer);
      set({ saveStatus: "saved" });
      savedTimer = setTimeout(() => set({ saveStatus: "idle" }), 1800);
    } catch (e) {
      set({ saveStatus: "error", saveError: errorMessage(e) });
    }
  },

  refreshModels: async () => {
    try {
      set({ models: await api.listModels() });
    } catch {
      /* garde la liste précédente */
    }
  },

  refreshHistory: async () => {
    try {
      set({ history: await api.getHistory() });
    } catch {
      /* ignore */
    }
  },

  refreshApiKey: async () => {
    try {
      set({ hasApiKey: await api.hasApiKey() });
    } catch {
      /* ignore */
    }
  },

  downloadModel: async (id) => {
    const model = get().models.find((m) => m.id === id);
    const total = (model?.size_mb ?? 0) * 1024 * 1024;
    set((st) => {
      const downloadErrors = { ...st.downloadErrors };
      delete downloadErrors[id];
      // Téléchargement déjà en cours (auto-installation) : on le rejoint sans remettre la jauge à zéro.
      const current = st.downloads[id] ?? { id, downloaded: 0, total, done: false };
      return { downloads: { ...st.downloads, [id]: current }, downloadErrors };
    });
    try {
      // Rust peut ne rendre la main qu'à la fin du téléchargement : la progression arrive par événement.
      await api.downloadModel(id);
      await get().refreshModels();
      if (get().models.find((m) => m.id === id)?.downloaded) {
        set((st) => {
          const downloads = { ...st.downloads };
          delete downloads[id];
          return { downloads };
        });
      }
    } catch (e) {
      const msg = errorMessage(e);
      set((st) => {
        const downloads = { ...st.downloads };
        delete downloads[id];
        const downloadErrors = { ...st.downloadErrors };
        if (!/annul|cancel/i.test(msg)) downloadErrors[id] = msg;
        return { downloads, downloadErrors };
      });
    }
  },

  cancelDownload: async (id) => {
    await api.cancelDownload(id).catch(() => undefined);
    set((st) => {
      const downloads = { ...st.downloads };
      delete downloads[id];
      return { downloads };
    });
  },

  deleteModel: async (id) => {
    await api.deleteModel(id);
    await get().refreshModels();
  },

  deleteHistoryEntry: async (id) => {
    set((st) => ({ history: st.history.filter((h) => h.id !== id) }));
    await api.deleteHistoryEntry(id).catch(() => get().refreshHistory());
  },

  clearHistory: async () => {
    set({ history: [] });
    await api.clearHistory().catch(() => get().refreshHistory());
  },

  checkUpdate: async () => {
    if (get().update.status === "installing" || get().update.status === "ready") return;
    set((st) => ({ update: { ...st.update, status: "checking", error: null } }));
    try {
      const info = await api.checkUpdate();
      set((st) =>
        st.update.status === "ready"
          ? st
          : { update: { status: info ? "available" : "uptodate", available: info, progress: null, error: null } },
      );
    } catch (e) {
      set((st) => ({ update: { ...st.update, status: "error", error: errorMessage(e) } }));
    }
  },

  transcribeFile: async () => {
    if (get().fileJob.status === "running") return;
    let path: string | null;
    try {
      path = await api.pickAudioFile();
    } catch (e) {
      set({ fileJob: { ...FILE_JOB_IDLE, status: "error", error: errorMessage(e) } });
      return;
    }
    if (!path) return;
    const name = path.split(/[\\/]/).pop() ?? path;
    set({ fileJob: { ...FILE_JOB_IDLE, status: "running", name } });
    try {
      const result = await api.transcribeFile(path);
      set({ fileJob: { status: "done", name, progress: null, result, error: null } });
    } catch (e) {
      const msg = errorMessage(e);
      set({ fileJob: /annul/i.test(msg) ? FILE_JOB_IDLE : { ...FILE_JOB_IDLE, status: "error", name, error: msg } });
    }
  },

  cancelFileTranscription: async () => {
    await api.cancelFileTranscription().catch(() => undefined);
  },

  dismissFileJob: () => {
    if (get().fileJob.status !== "running") set({ fileJob: FILE_JOB_IDLE });
  },

  importConfig: async () => {
    // Une modification locale en attente serait écrasée par l'import : on l'enregistre d'abord.
    await get().flushSettings();
    const settings = await api.importConfig();
    if (!settings) return false;
    set({ settings });
    await get().refreshApiKey();
    return true;
  },

  installUpdate: async () => {
    if (get().update.status === "installing") return;
    set((st) => ({ update: { ...st.update, status: "installing", progress: null, error: null } }));
    try {
      // En cas de succès, Rust relance l'app : cette promesse ne se résout normalement pas.
      await api.installUpdate();
    } catch (e) {
      set((st) => ({
        update: {
          ...st.update,
          status: st.update.status === "ready" ? "ready" : st.update.available ? "available" : "error",
          progress: null,
          error: errorMessage(e),
        },
      }));
    }
  },
}));

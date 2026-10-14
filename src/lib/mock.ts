// Mode démo : simule le backend Rust dans un navigateur (pnpm dev sans Tauri).
// Données persistées dans localStorage (`?reset` pour repartir de zéro),
// événements relayés entre onglets via BroadcastChannel (fenêtre principale ↔ overlay.html).

import type {
  AudioDevice,
  CommandMap,
  CommandName,
  EventMap,
  EventName,
  HistoryEntry,
  ModelInfo,
  Permissions,
  Platform,
  Settings,
} from "./types";

const STORAGE_KEY = "murmure-mock-v1";

const DEFAULT_SETTINGS: Settings = {
  engine: "local",
  local_model: "large-v3-turbo-q5_0",
  language: "fr",
  initial_prompt: "",
  remote_endpoint: "",
  remote_model: "whisper-large-v3-turbo",
  hotkey: "Alt+Space",
  hotkey_mode: "hybrid",
  input_device: null,
  paste_mode: "paste",
  restore_clipboard: true,
  sounds: true,
  launch_at_login: false,
  history_enabled: true,
  onboarding_done: false,
};

const CATALOG: Omit<ModelInfo, "downloaded" | "path">[] = [
  {
    id: "large-v3-turbo-q5_0",
    name: "Whisper large-v3-turbo (q5)",
    description: "Le meilleur compromis : précision proche du modèle complet, trois fois plus léger.",
    size_mb: 574,
    recommended: true,
  },
  {
    id: "large-v3-turbo-q8_0",
    name: "Whisper large-v3-turbo (q8)",
    description: "Quantification plus fine, un peu plus précis et plus lourd.",
    size_mb: 874,
    recommended: false,
  },
  {
    id: "large-v3-turbo",
    name: "Whisper large-v3-turbo",
    description: "Modèle complet en pleine précision. Pour les machines très bien dotées.",
    size_mb: 1624,
    recommended: false,
  },
  {
    id: "small",
    name: "Whisper small",
    description: "Rapide et compact, moins précis sur les noms propres.",
    size_mb: 488,
    recommended: false,
  },
  {
    id: "base",
    name: "Whisper base",
    description: "Très léger, pour les machines modestes.",
    size_mb: 148,
    recommended: false,
  },
];

const PHRASES = [
  "Bonjour Claire, je te confirme notre rendez-vous de jeudi à 14 h dans les locaux de Lyon.",
  "Penser à relancer le cabinet Morel pour la signature de l'avenant avant la fin du mois.",
  "Le déploiement de la version 2.3 est prévu lundi matin, la base de données sera migrée la veille.",
  "Merci pour votre retour. Je vous envoie le devis révisé d'ici demain midi.",
  "Ajouter au compte rendu : le budget communication est validé, la refonte du site démarre en novembre.",
  "Peux-tu vérifier que les tests passent sur la branche principale avant de fusionner ?",
  "Réserver une salle pour l'atelier de jeudi, prévoir un vidéoprojecteur et du café.",
  "La réunion avec l'équipe produit est décalée à 16 h, même lien de visio.",
  "Note pour plus tard : comparer les offres d'hébergement souverain avant le comité de pilotage.",
  "Je suis d'accord sur le principe, mais il faudrait chiffrer précisément la phase deux.",
  "Rappel : le dossier de subvention doit être déposé sur la plateforme avant le 30.",
  "Très bonne présentation hier, les clients ont apprécié la démonstration en direct.",
];

const DEVICES: AudioDevice[] = [
  { name: "Micro MacBook Pro", is_default: true },
  { name: "AirPods Pro", is_default: false },
  { name: "Shure MV7", is_default: false },
];

interface MockData {
  settings: Settings;
  downloaded: string[];
  history: HistoryEntry[];
  apiKey: string;
  permissions: Permissions;
}

function seedHistory(): HistoryEntry[] {
  const now = Date.now();
  const offsets = [4, 38, 95, 170, 26 * 60, 27 * 60, 50 * 60, 3 * 24 * 60, 4 * 24 * 60, 9 * 24 * 60];
  return offsets.map((min, i) => {
    const text = PHRASES[i % PHRASES.length];
    const words = text.split(/\s+/).length;
    return {
      id: `seed-${i}`,
      text,
      created_at: new Date(now - min * 60_000).toISOString(),
      duration_ms: Math.round(words * 380 + 600),
      latency_ms: i % 4 === 3 ? 640 + i * 11 : 280 + ((i * 47) % 160),
      engine: i % 4 === 3 ? "remote" : "local",
      model: i % 4 === 3 ? "whisper-large-v3-turbo" : "large-v3-turbo-q5_0",
    };
  });
}

function freshData(): MockData {
  return {
    settings: { ...DEFAULT_SETTINGS },
    downloaded: [],
    history: seedHistory(),
    apiKey: "",
    permissions: { microphone: "undetermined", accessibility: "denied" },
  };
}

function loadData(): MockData {
  try {
    if (new URLSearchParams(location.search).has("reset")) {
      localStorage.removeItem(STORAGE_KEY);
    }
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<MockData>;
      const base = freshData();
      return {
        ...base,
        ...parsed,
        settings: { ...base.settings, ...(parsed.settings ?? {}) },
      };
    }
  } catch {
    /* stockage indisponible : données fraîches */
  }
  return freshData();
}

const data = loadData();
function persist() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(data));
  } catch {
    /* ignore */
  }
}

// ---------- Bus d'événements ----------

type Handler = (payload: unknown) => void;
const handlers = new Map<string, Set<Handler>>();
let channel: BroadcastChannel | null = null;
try {
  channel = new BroadcastChannel("murmure-mock");
  channel.onmessage = (e: MessageEvent<{ event: string; payload: unknown }>) => {
    dispatchLocal(e.data.event, e.data.payload);
  };
} catch {
  channel = null;
}

function dispatchLocal(event: string, payload: unknown) {
  handlers.get(event)?.forEach((h) => h(payload));
}

export function mockEmit<E extends EventName>(event: E, payload: EventMap[E]) {
  dispatchLocal(event, payload);
  channel?.postMessage({ event, payload });
}

export function mockListen<E extends EventName>(
  event: E,
  handler: (payload: EventMap[E]) => void,
): () => void {
  const set = handlers.get(event) ?? new Set<Handler>();
  handlers.set(event, set);
  // Enveloppe unique : un même handler (ex. setState) peut être abonné plusieurs fois.
  const h: Handler = (payload) => handler(payload as EventMap[E]);
  set.add(h);
  return () => set.delete(h);
}

// ---------- Utilitaires ----------

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function detectPlatform(): Platform {
  const forced = new URLSearchParams(location.search).get("platform");
  if (forced === "macos" || forced === "linux" || forced === "windows") return forced;
  const ua = navigator.userAgent;
  if (/Mac/i.test(ua)) return "macos";
  if (/Win/i.test(ua)) return "windows";
  return "linux";
}

function models(): ModelInfo[] {
  return CATALOG.map((m) => {
    const downloaded = data.downloaded.includes(m.id);
    return {
      ...m,
      downloaded,
      path: downloaded ? `~/Library/Application Support/fr.rdagency.murmure/models/ggml-${m.id}.bin` : null,
    };
  });
}

const downloads = new Map<string, { cancelled: boolean }>();

async function simulateDownload(id: string) {
  const model = CATALOG.find((m) => m.id === id);
  if (!model) throw "Modèle inconnu";
  if (downloads.has(id)) return;
  const token = { cancelled: false };
  downloads.set(id, token);
  const total = model.size_mb * 1024 * 1024;
  let downloaded = 0;
  const durationMs = 5000 + model.size_mb * 2;
  const stepMs = 120;
  (async () => {
    while (downloaded < total) {
      await sleep(stepMs);
      if (token.cancelled) {
        downloads.delete(id);
        mockEmit("model-download-progress", { id, downloaded, total, done: true, error: "Téléchargement annulé" });
        return;
      }
      const jitter = 0.6 + Math.random() * 0.8;
      downloaded = Math.min(total, downloaded + (total / (durationMs / stepMs)) * jitter);
      mockEmit("model-download-progress", { id, downloaded: Math.round(downloaded), total, done: false });
    }
    downloads.delete(id);
    if (!data.downloaded.includes(id)) data.downloaded.push(id);
    persist();
    mockEmit("model-download-progress", { id, downloaded: total, total, done: true });
  })();
}

// ---------- Dictée simulée ----------

let dictation: {
  startedAt: number;
  timer: ReturnType<typeof setInterval>;
  handsFree: boolean;
} | null = null;
let busy = false;

function startDictation(handsFree = false) {
  if (dictation || busy) return;
  let level = 0.1;
  let t = 0;
  const timer = setInterval(() => {
    t += 1;
    // Enveloppe de parole : syllabes + pauses.
    const syllable = Math.max(0, Math.sin(t / 2.1)) * Math.max(0, Math.sin(t / 9));
    const target = 0.08 + syllable * (0.55 + Math.random() * 0.4);
    level += (target - level) * 0.5;
    mockEmit("audio-level", Math.min(1, Math.max(0, level)));
  }, 33);
  dictation = { startedAt: Date.now(), timer, handsFree };
  mockEmit("dictation-state", { state: "recording", hands_free: handsFree });
}

function setHandsFree() {
  if (!dictation) return;
  dictation.handsFree = true;
  mockEmit("dictation-state", { state: "recording", hands_free: true });
}

function cancelDictation() {
  if (!dictation) return;
  clearInterval(dictation.timer);
  dictation = null;
  mockEmit("dictation-state", { state: "idle" });
}

async function stopDictation() {
  if (!dictation) return;
  const d = dictation;
  clearInterval(d.timer);
  dictation = null;
  busy = true;
  const duration = Date.now() - d.startedAt;
  mockEmit("dictation-state", { state: "transcribing" });
  const latency = 260 + Math.round(Math.random() * 380);
  await sleep(latency + 400);
  busy = false;
  if (duration < 250) {
    mockEmit("dictation-state", { state: "idle" });
    return;
  }
  const s = data.settings;
  if (s.engine === "local" && !data.downloaded.includes(s.local_model)) {
    mockEmit("dictation-state", { state: "error", message: "Aucun modèle téléchargé" });
    await sleep(2200);
    mockEmit("dictation-state", { state: "idle" });
    return;
  }
  const text = PHRASES[Math.floor(Math.random() * PHRASES.length)];
  const entry: HistoryEntry = {
    id: `mock-${Date.now()}`,
    text,
    created_at: new Date().toISOString(),
    duration_ms: duration,
    latency_ms: latency,
    engine: s.engine,
    model: s.engine === "local" ? s.local_model : s.remote_model,
  };
  if (s.history_enabled) {
    data.history.unshift(entry);
    persist();
  }
  pasteIntoFocused(text);
  mockEmit("dictation-result", entry);
  mockEmit("dictation-state", { state: "idle" });
}

/** Simule le collage dans « l'application active » : le champ qui a le focus. */
function pasteIntoFocused(text: string) {
  if (data.settings.paste_mode !== "paste") return;
  const el = document.activeElement;
  if (el instanceof HTMLTextAreaElement || (el instanceof HTMLInputElement && el.type === "text")) {
    const start = el.selectionStart ?? el.value.length;
    const end = el.selectionEnd ?? el.value.length;
    const before = el.value.slice(0, start);
    const sep = before && !/\s$/.test(before) ? " " : "";
    el.setRangeText(sep + text, start, end, "end");
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }
}

// ---------- Raccourci simulé (dans la page uniquement) ----------

const MOD_KEYS = new Set(["Alt", "Option", "Shift", "Control", "Ctrl", "CommandOrControl", "CmdOrCtrl", "Command", "Cmd", "Super", "Meta"]);

function matchesHotkey(e: KeyboardEvent, accel: string): boolean {
  const parts = accel.split("+");
  const key = parts.find((p) => !MOD_KEYS.has(p));
  if (!key) return false;
  const mac = detectPlatform() === "macos";
  const want = { alt: false, shift: false, ctrl: false, meta: false };
  for (const p of parts) {
    if (p === "Alt" || p === "Option") want.alt = true;
    else if (p === "Shift") want.shift = true;
    else if (p === "Control" || p === "Ctrl") want.ctrl = true;
    else if (p === "Command" || p === "Cmd" || p === "Super" || p === "Meta") want.meta = true;
    else if (p === "CommandOrControl" || p === "CmdOrCtrl") {
      if (mac) want.meta = true;
      else want.ctrl = true;
    }
  }
  if (e.altKey !== want.alt || e.shiftKey !== want.shift || e.ctrlKey !== want.ctrl || e.metaKey !== want.meta) {
    return false;
  }
  return codeToKey(e.code) === key;
}

function codeToKey(code: string): string {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  return code;
}

let hotkeyInstalled = false;
/** Reproduit le comportement du raccourci global dans la page (mode démo). */
export function installMockHotkey() {
  if (hotkeyInstalled) return;
  hotkeyInstalled = true;
  let pressedAt = 0;
  let holding = false;
  let mainKey = "";
  window.addEventListener(
    "keydown",
    (e) => {
      if ((e.target as HTMLElement | null)?.closest?.("[data-hotkey-recorder]")) return;
      if (e.key === "Escape" && dictation) {
        e.preventDefault();
        cancelDictation();
        return;
      }
      if (e.repeat) {
        if (holding) e.preventDefault();
        return;
      }
      if (!matchesHotkey(e, data.settings.hotkey)) return;
      e.preventDefault();
      const mode = data.settings.hotkey_mode;
      if (dictation) {
        // 2e appui : arrêt (mains libres / bascule).
        void stopDictation();
        return;
      }
      mainKey = e.code;
      pressedAt = Date.now();
      holding = true;
      startDictation(mode === "toggle");
    },
    true,
  );
  window.addEventListener(
    "keyup",
    (e) => {
      if (!holding || e.code !== mainKey) return;
      holding = false;
      e.preventDefault();
      const mode = data.settings.hotkey_mode;
      if (mode === "toggle") return;
      if (mode === "hybrid" && Date.now() - pressedAt < 300) {
        setHandsFree();
        return;
      }
      void stopDictation();
    },
    true,
  );
}

// ---------- Commandes ----------

type Impl = {
  [K in CommandName]: (args: CommandMap[K]["args"]) => Promise<CommandMap[K]["ret"]> | CommandMap[K]["ret"];
};

const impl: Impl = {
  get_settings: () => structuredClone(data.settings),
  save_settings: async ({ settings }) => {
    await sleep(120);
    data.settings = structuredClone(settings);
    persist();
    mockEmit("settings-changed", structuredClone(settings));
  },
  set_api_key: async ({ key }) => {
    await sleep(80);
    data.apiKey = key;
    persist();
  },
  has_api_key: () => data.apiKey.length > 0,
  test_remote: async ({ endpoint, apiKey }) => {
    await sleep(700 + Math.random() * 500);
    let url: URL;
    try {
      url = new URL(endpoint);
    } catch {
      throw "Adresse invalide : indiquez une URL complète (https://…)";
    }
    if (!/^https?:$/.test(url.protocol)) throw "Seuls http:// et https:// sont acceptés";
    if (url.hostname.includes("erreur")) throw "Connexion refusée par le serveur (502)";
    if (!apiKey && !data.apiKey) throw "Le serveur exige une clé API (401)";
    return `Connexion réussie · ${Math.round(180 + Math.random() * 240)} ms`;
  },
  list_input_devices: () => DEVICES,
  list_models: () => models(),
  download_model: ({ id }) => simulateDownload(id),
  cancel_download: ({ id }) => {
    const t = downloads.get(id);
    if (t) t.cancelled = true;
  },
  delete_model: async ({ id }) => {
    await sleep(150);
    data.downloaded = data.downloaded.filter((d) => d !== id);
    persist();
  },
  start_dictation: () => startDictation(false),
  stop_dictation: () => stopDictation(),
  cancel_dictation: () => cancelDictation(),
  toggle_dictation: () => (dictation ? stopDictation() : startDictation(true)),
  get_history: () => structuredClone(data.history),
  delete_history_entry: ({ id }) => {
    data.history = data.history.filter((h) => h.id !== id);
    persist();
  },
  clear_history: () => {
    data.history = [];
    persist();
  },
  copy_text: ({ text }) => {
    // Sans attendre : l'API presse-papiers du navigateur peut rester en suspens sans focus.
    navigator.clipboard?.writeText(text).catch(() => undefined);
  },
  check_permissions: () => {
    const p = { ...data.permissions };
    if (detectPlatform() !== "macos") p.accessibility = "unknown";
    return p;
  },
  request_microphone: async () => {
    await sleep(600);
    data.permissions.microphone = "granted";
    persist();
    return impl.check_permissions(undefined) as Permissions;
  },
  open_permission_settings: ({ kind }) => {
    // Simule l'utilisateur qui coche la case dans Réglages Système.
    setTimeout(() => {
      data.permissions[kind] = "granted";
      persist();
    }, 1800);
  },
  get_platform: () => detectPlatform(),
};

export async function mockInvoke(cmd: CommandName, args: unknown): Promise<unknown> {
  const fn = impl[cmd] as (a: unknown) => unknown;
  // Petite latence réaliste.
  await sleep(20);
  return fn(args);
}

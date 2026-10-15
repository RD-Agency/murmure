// SPDX-License-Identifier: AGPL-3.0-or-later
// Mode démo : simule le backend Rust dans un navigateur (pnpm dev sans Tauri).
// Données persistées dans localStorage (`?reset` pour repartir de zéro),
// événements relayés entre onglets via BroadcastChannel (fenêtre principale ↔ overlay.html).

import type {
  AudioDevice,
  AudioUsage,
  CorrectionResult,
  CommandMap,
  CommandName,
  DownloadStatus,
  EventMap,
  EventName,
  HistoryEntry,
  HistoryPage,
  HistoryQuery,
  ModelInfo,
  Permissions,
  Platform,
  Settings,
  TermChange,
  VocabularySuggestion,
  WaylandStatus,
} from "./types";

const STORAGE_KEY = "murmure-mock-v2";

const DEFAULT_SETTINGS: Settings = {
  engine: "local",
  local_model: "large-v3-turbo-q5_0",
  language: "fr",
  initial_prompt: "",
  spoken_punctuation: true,
  short_text_plain: true,
  remote_endpoint: "",
  remote_model: "whisper-large-v3-turbo",
  hotkey: "Alt+Space",
  hotkey_mode: "hybrid",
  auto_stop_silence_ms: 2000,
  input_device: null,
  duck_volume: false,
  paste_mode: "paste",
  restore_clipboard: true,
  submit_word_enabled: false,
  submit_words: ["terminé", "envoyer"],
  sounds: true,
  launch_at_login: false,
  history_enabled: true,
  keep_audio: true,
  audio_cap_mb: 5120,
  auto_correct: true,
  replacements: [],
  onboarding_done: false,
  wayland_restore_token: null,
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

/** Dictées juridiques de démonstration : [texte collé, texte corrigé ou null]. */
const LEGAL: [string, string | null][] = [
  [
    "Nous sollicitons en conséquence l'affirmation du jugement entrepris en toutes ses dispositions.",
    "Nous sollicitons en conséquence l'infirmation du jugement entrepris en toutes ses dispositions.",
  ],
  [
    "L'employeur n'a pas respecté la procédure prévue aux articles L 12 32 2 et suivants du code du travail.",
    "L'employeur n'a pas respecté la procédure prévue aux articles L. 1232-2 et suivants du code du travail.",
  ],
  [
    "La cour d'appel a prononcé l'affirmation partielle de la décision rendue en première instance.",
    "La cour d'appel a prononcé l'infirmation partielle de la décision rendue en première instance.",
  ],
  ["Le dossier Lefebvre contre la société Batimex est renvoyé à l'audience de mise en état du 12 novembre.", null],
];

const DEVICES: AudioDevice[] = [
  { name: "Micro MacBook Pro", is_default: true },
  { name: "AirPods Pro", is_default: false },
  { name: "Shure MV7", is_default: false },
];

interface MockCorrection {
  entryId: string;
  changes: TermChange[];
}

interface MockData {
  settings: Settings;
  downloaded: string[];
  history: HistoryEntry[];
  corrections: MockCorrection[];
  dismissed: string[];
  apiKey: string;
  permissions: Permissions;
}

/** Opus à 20 kbit/s : 2 500 octets par seconde. */
const OPUS_BYTES_PER_MS = 2.5;

function makeEntry(
  id: string,
  text: string,
  createdAt: number,
  opts: Partial<HistoryEntry> & { engine?: HistoryEntry["engine"] } = {},
): HistoryEntry {
  const words = text.split(/\s+/).length;
  const duration = opts.duration_ms ?? Math.round(words * 380 + 600);
  const engine = opts.engine ?? "local";
  const hasAudio = opts.has_audio ?? true;
  return {
    id,
    text,
    created_at: new Date(createdAt).toISOString(),
    duration_ms: duration,
    latency_ms: opts.latency_ms ?? 300,
    engine,
    model: engine === "remote" ? "whisper-large-v3-turbo" : "large-v3-turbo-q5_0",
    language: "fr",
    raw_text: opts.raw_text ?? text.toLowerCase().replace(/[.,;:!?]/g, ""),
    final_text: opts.final_text ?? text,
    corrected_text: opts.corrected_text ?? null,
    corrected_at: opts.corrected_at ?? null,
    auto_corrections: opts.auto_corrections ?? [],
    has_audio: hasAudio,
    audio_bytes: hasAudio ? Math.round(duration * OPUS_BYTES_PER_MS) : 0,
  };
}

function seedHistory(): { history: HistoryEntry[]; corrections: MockCorrection[] } {
  const now = Date.now();
  const history: HistoryEntry[] = [];
  const corrections: MockCorrection[] = [];
  LEGAL.forEach(([final, corrected], i) => {
    const at = now - (12 + i * 41) * 60_000;
    const id = `seed-legal-${i}`;
    history.push(
      makeEntry(id, corrected ?? final, at, {
        final_text: final,
        corrected_text: corrected,
        corrected_at: corrected ? new Date(at + 90_000).toISOString() : null,
        latency_ms: 290 + i * 23,
        auto_corrections:
          i === 3 ? [{ from: "Batimax", to: "Batimex", probability: 0.31 }] : [],
      }),
    );
    if (corrected) corrections.push({ entryId: id, changes: diffWords(final, corrected) });
  });
  const offsets = [4, 95, 170, 26 * 60, 27 * 60, 50 * 60, 3 * 24 * 60, 4 * 24 * 60, 9 * 24 * 60];
  offsets.forEach((min, i) => {
    history.push(
      makeEntry(`seed-${i}`, PHRASES[i % PHRASES.length], now - min * 60_000, {
        engine: i % 4 === 3 ? "remote" : "local",
        latency_ms: i % 4 === 3 ? 640 + i * 11 : 280 + ((i * 47) % 160),
        has_audio: i < 7,
      }),
    );
  });
  history.sort((a, b) => b.created_at.localeCompare(a.created_at));
  return { history, corrections };
}

/** Diff mot à mot simplifié (préfixe et suffixe communs), comme `correction::word_diff`. */
function diffWords(before: string, after: string): TermChange[] {
  const a = before.split(/\s+/);
  const b = after.split(/\s+/);
  let p = 0;
  while (p < a.length && p < b.length && a[p] === b[p]) p++;
  let q = 0;
  while (q < a.length - p && q < b.length - p && a[a.length - 1 - q] === b[b.length - 1 - q]) q++;
  const wrong = a.slice(p, a.length - q).join(" ");
  const right = b.slice(p, b.length - q).join(" ");
  if (!wrong || !right || wrong.split(" ").length > 6 || right.split(" ").length > 6) return [];
  const strip = (s: string) => s.replace(/^[ldjmnst]['’]/i, "").replace(/[.,;:!?]+$/, "");
  return [{ wrong: strip(wrong), right: strip(right) }];
}

function normalize(s: string): string {
  return s.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLowerCase();
}

function searchHistory(q: HistoryQuery): HistoryPage {
  const terms = normalize(q.text).split(/\s+/).filter(Boolean);
  const from = q.from ? new Date(`${q.from}T00:00:00`).getTime() : null;
  const to = q.to ? new Date(`${q.to}T00:00:00`).getTime() + 86_400_000 : null;
  const all = data.history.filter((h) => {
    const t = new Date(h.created_at).getTime();
    if (from !== null && t < from) return false;
    if (to !== null && t >= to) return false;
    const words = normalize(h.text).split(/[^\p{L}\p{N}]+/u);
    return terms.every((term) => words.some((w) => w.startsWith(term.replace(/[^\p{L}\p{N}]/gu, ""))));
  });
  const offset = q.offset ?? 0;
  return { entries: structuredClone(all.slice(offset, offset + (q.limit ?? 100))), total: all.length };
}

function suggestions(): VocabularySuggestion[] {
  const counts = new Map<string, { count: number; variants: Set<string> }>();
  for (const c of data.corrections) {
    for (const ch of c.changes) {
      const cur = counts.get(ch.right) ?? { count: 0, variants: new Set<string>() };
      cur.count++;
      cur.variants.add(ch.wrong);
      counts.set(ch.right, cur);
    }
  }
  const vocab = data.settings.initial_prompt.toLowerCase();
  return [...counts.entries()]
    .filter(([term, v]) => v.count >= 2 && !data.dismissed.includes(term) && !vocab.includes(term.toLowerCase()))
    .filter(([term]) => !data.settings.replacements.some((r) => r.to.toLowerCase() === term.toLowerCase()))
    .map(([term, v]) => ({ term, count: v.count, variants: [...v.variants] }));
}

function audioUsage(): AudioUsage {
  const withAudio = data.history.filter((h) => h.has_audio);
  return {
    bytes: withAudio.reduce((n, h) => n + h.audio_bytes, 0),
    files: withAudio.length,
    cap_bytes: Math.min(Math.max(data.settings.audio_cap_mb, 100), 5120) * 1024 * 1024,
    duration_ms: withAudio.reduce((n, h) => n + h.duration_ms, 0),
  };
}

/** WAV de démonstration : une voix synthétique très douce, de la durée de la dictée. */
function demoWav(durationMs: number): ArrayBuffer {
  const rate = 16_000;
  const n = Math.round((Math.min(durationMs, 20_000) / 1000) * rate);
  const buf = new ArrayBuffer(44 + n * 2);
  const v = new DataView(buf);
  const str = (o: number, s: string) => [...s].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  str(0, "RIFF");
  v.setUint32(4, 36 + n * 2, true);
  str(8, "WAVEfmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  str(36, "data");
  v.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) {
    const t = i / rate;
    const env = Math.max(0, Math.sin(t * 5)) ** 2;
    const f0 = 150 + 20 * Math.sin(t * 2);
    const s = (Math.sin(2 * Math.PI * f0 * t) + 0.4 * Math.sin(4 * Math.PI * f0 * t)) * env * 0.12;
    v.setInt16(44 + i * 2, Math.round(s * 32767), true);
  }
  return buf;
}

function freshData(): MockData {
  const seed = seedHistory();
  return {
    settings: { ...DEFAULT_SETTINGS },
    downloaded: [],
    history: seed.history,
    corrections: seed.corrections,
    dismissed: [],
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

// Registre des téléchargements, comme `state.rs` : un seul par modèle, progression consultable.
const downloads = new Map<string, { cancelled: boolean; downloaded: number; total: number }>();

const RECOMMENDED_ID = CATALOG.find((m) => m.recommended)?.id ?? "large-v3-turbo-q5_0";
const resolveId = (id: string) => (CATALOG.some((m) => m.id === id) ? id : RECOMMENDED_ID);

function simulateDownload(id: string) {
  const model = CATALOG.find((m) => m.id === id);
  if (!model) throw "Modèle inconnu";
  // Déjà en cours (ex. auto-installation) : on le rejoint.
  if (downloads.has(id)) return;
  const total = model.size_mb * 1024 * 1024;
  const token = { cancelled: false, downloaded: 0, total };
  downloads.set(id, token);
  const durationMs = 5000 + model.size_mb * 2;
  const stepMs = 120;
  (async () => {
    while (token.downloaded < total) {
      await sleep(stepMs);
      if (token.cancelled) {
        downloads.delete(id);
        mockEmit("model-download-progress", { id, downloaded: token.downloaded, total, done: true, error: "Téléchargement annulé" });
        return;
      }
      const jitter = 0.6 + Math.random() * 0.8;
      token.downloaded = Math.round(Math.min(total, token.downloaded + (total / (durationMs / stepMs)) * jitter));
      mockEmit("model-download-progress", { id, downloaded: token.downloaded, total, done: false });
    }
    downloads.delete(id);
    if (!data.downloaded.includes(id)) data.downloaded.push(id);
    persist();
    mockEmit("model-download-progress", { id, downloaded: total, total, done: true });
  })();
}

/** Comme `install::ensure_installed` : moteur local sans modèle → téléchargement automatique. */
function ensureInstalled() {
  const s = data.settings;
  if (s.engine !== "local") return;
  const id = resolveId(s.local_model);
  if (!data.downloaded.includes(id)) simulateDownload(id);
}

let autoInstallDone = false;

function downloadStatuses(): DownloadStatus[] {
  // Premier appel (démarrage de la fenêtre principale) : simule l'auto-installation de Rust.
  if (!autoInstallDone) {
    autoInstallDone = true;
    ensureInstalled();
  }
  return [...downloads.entries()].map(([id, d]) => ({ id, downloaded: d.downloaded, total: d.total }));
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
  const s = data.settings;
  if (s.engine === "local" && !data.downloaded.includes(s.local_model)) {
    // Comme `pipeline::preflight` : installation automatique, message avec la progression réelle.
    const id = resolveId(s.local_model);
    const d = downloads.get(id);
    const message = d
      ? `Installation du modèle en cours (${Math.min(99, Math.floor((d.downloaded / Math.max(1, d.total)) * 100))} %) — réessayez dans un instant`
      : "Installation du modèle…";
    if (!d) simulateDownload(id);
    mockEmit("dictation-state", { state: "error", message });
    setTimeout(() => mockEmit("dictation-state", { state: "idle" }), 2500);
    return;
  }
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
  const text = PHRASES[Math.floor(Math.random() * PHRASES.length)];
  const entry: HistoryEntry = {
    ...makeEntry(`mock-${Date.now()}`, text, Date.now(), {
      duration_ms: duration,
      latency_ms: latency,
      engine: s.engine,
      has_audio: s.history_enabled && s.keep_audio,
    }),
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

// ---------- Mises à jour simulées ----------
// `?update` : une version 0.2.0 est disponible ; `?update=ready` : déjà téléchargée et en place ;
// `?update=error` : serveur injoignable.

export const MOCK_VERSION = "0.1.0";
const updateParam = new URLSearchParams(location.search).get("update");
const MOCK_UPDATE =
  updateParam === null || updateParam === "error"
    ? null
    : { version: "0.2.0", notes: "Corrections et améliorations de la stabilité." };

if (MOCK_UPDATE) {
  // Comme `updater::spawn_background` : annonce silencieuse peu après le démarrage.
  setTimeout(() => mockEmit(updateParam === "ready" ? "update-ready" : "update-available", MOCK_UPDATE), 1500);
}

async function checkUpdate() {
  await sleep(700);
  if (updateParam === "error") throw "Impossible de joindre le serveur de mises à jour. Vérifiez votre connexion.";
  if (MOCK_UPDATE) mockEmit("update-available", MOCK_UPDATE);
  return MOCK_UPDATE;
}

let installingUpdate = false;
async function installUpdate() {
  if (!MOCK_UPDATE) throw "Aucune mise à jour disponible.";
  if (installingUpdate) throw "Installation déjà en cours.";
  installingUpdate = true;
  const total = 18 * 1024 * 1024;
  for (let downloaded = 0; downloaded < total; ) {
    await sleep(100);
    downloaded = Math.min(total, downloaded + total / 30);
    mockEmit("update-progress", { downloaded, total });
  }
  await sleep(600);
  // « Relance » : rechargement sans le paramètre ?update.
  location.href = location.pathname;
}

// ---------- Wayland (`?wayland` simule une session KDE Wayland, `?wayland=deny` un refus) ----------

const waylandParam = new URLSearchParams(location.search).get("wayland");
let wayland: WaylandStatus = {
  wayland: waylandParam !== null,
  shortcut: "inactive",
  shortcut_trigger: null,
  shortcut_configurable: true,
  shortcut_error: null,
  keyboard: "inactive",
  keyboard_error: null,
  paste_fallback: null,
};

async function waylandAuthorize(): Promise<WaylandStatus> {
  if (!wayland.wayland) return wayland;
  wayland = { ...wayland, shortcut: "pending", shortcut_error: null };
  mockEmit("wayland-status", wayland);
  await sleep(1200);
  wayland =
    waylandParam === "deny"
      ? { ...wayland, shortcut: "denied", shortcut_error: "Raccourci non accordé : Demande refusée." }
      : { ...wayland, shortcut: "active", shortcut_trigger: "Ctrl+Alt+Espace" };
  wayland = { ...wayland, keyboard: "pending", keyboard_error: null };
  mockEmit("wayland-status", wayland);
  await sleep(1000);
  wayland =
    waylandParam === "deny"
      ? { ...wayland, keyboard: "denied", keyboard_error: "Contrôle du clavier non accordé : Demande refusée." }
      : { ...wayland, keyboard: "active" };
  mockEmit("wayland-status", wayland);
  return wayland;
}

// ---------- Réglages et fichiers ----------

function downloadText(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function exportConfig(includeSecrets: boolean): string {
  const settings: Partial<Settings> = structuredClone(data.settings);
  delete settings.onboarding_done;
  delete settings.wayland_restore_token;
  const file = {
    format: "murmure-config",
    version: 1,
    app_version: MOCK_VERSION,
    exported_at: new Date().toISOString(),
    settings,
    ...(includeSecrets && data.apiKey ? { secrets: { remote_api_key: data.apiKey } } : {}),
  };
  downloadText("murmure-reglages.json", JSON.stringify(file, null, 2));
  return "~/Téléchargements/murmure-reglages.json";
}

/** Choix d'un fichier dans le navigateur ; `null` si l'utilisateur ferme la boîte. */
function pickFile(accept: string): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.oncancel = () => resolve(null);
    input.click();
  });
}

async function importConfig(): Promise<Settings | null> {
  const file = await pickFile(".json,application/json");
  if (!file) return null;
  let parsed: { format?: string; version?: number; settings?: Partial<Settings>; secrets?: { remote_api_key?: string } };
  try {
    parsed = JSON.parse(await file.text());
  } catch {
    throw "Fichier illisible : ce n'est pas du JSON valide.";
  }
  if (parsed.format !== "murmure-config") throw "Ce fichier n'est pas un export de réglages de Murmure.";
  if (!parsed.version || parsed.version > 1) throw "Ce fichier vient d'une version plus récente de Murmure : mettez Murmure à jour.";
  const known = Object.keys(DEFAULT_SETTINGS) as (keyof Settings)[];
  const next = { ...data.settings } as Record<string, unknown>;
  for (const key of known) {
    if (key === "onboarding_done" || key === "wayland_restore_token") continue;
    if (parsed.settings && key in parsed.settings) next[key] = parsed.settings[key];
  }
  if (parsed.secrets?.remote_api_key) data.apiKey = parsed.secrets.remote_api_key;
  await impl.save_settings({ settings: next as unknown as Settings });
  return structuredClone(data.settings);
}

const pickedFiles = new Map<string, File>();
let fileCancelled = false;

async function pickAudioFile(): Promise<string | null> {
  const file = await pickFile("audio/*,.wav,.mp3,.m4a,.aac,.ogg,.opus,.flac");
  if (!file) return null;
  pickedFiles.set(file.name, file);
  return file.name;
}

async function transcribeFile(path: string) {
  fileCancelled = false;
  const file = pickedFiles.get(path);
  // Durée approximative : 1 Mo par minute.
  const durationMs = Math.max(20_000, Math.round(((file?.size ?? 2_000_000) / 1_000_000) * 60_000));
  const total = Math.max(1, Math.ceil(durationMs / 20_000));
  const started = Date.now();
  for (let i = 0; i <= 10; i++) {
    if (fileCancelled) throw "Transcription annulée";
    mockEmit("file-transcription-progress", { stage: "decoding", fraction: (i / 10) * 0.1, done: 0, total: 0 });
    await sleep(40);
  }
  const parts: string[] = [];
  for (let i = 0; i < total; i++) {
    if (fileCancelled) throw "Transcription annulée";
    await sleep(350);
    parts.push(PHRASES[i % PHRASES.length]);
    mockEmit("file-transcription-progress", { stage: "transcribing", fraction: 0.1 + (0.9 * (i + 1)) / total, done: i + 1, total });
  }
  return { text: parts.join(" "), file_name: path, duration_ms: durationMs, latency_ms: Date.now() - started };
}

// ---------- Commandes ----------

type Impl = {
  [K in CommandName]: (args: CommandMap[K]["args"]) => Promise<CommandMap[K]["ret"]> | CommandMap[K]["ret"];
};

const impl: Impl = {
  get_settings: () => structuredClone(data.settings),
  save_settings: async ({ settings }) => {
    await sleep(120);
    const old = data.settings;
    data.settings = structuredClone(settings);
    if (settings.engine === "local" && (old.engine !== "local" || old.local_model !== settings.local_model)) {
      ensureInstalled();
    }
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
  get_downloads: () => downloadStatuses(),
  delete_model: async ({ id }) => {
    await sleep(150);
    data.downloaded = data.downloaded.filter((d) => d !== id);
    persist();
  },
  start_dictation: () => startDictation(false),
  stop_dictation: () => stopDictation(),
  cancel_dictation: () => cancelDictation(),
  toggle_dictation: () => (dictation ? stopDictation() : startDictation(true)),
  get_history: () => structuredClone(data.history.slice(0, 500)),
  search_history: ({ query }) => searchHistory(query),
  delete_history_entry: ({ id }) => {
    data.history = data.history.filter((h) => h.id !== id);
    data.corrections = data.corrections.filter((c) => c.entryId !== id);
    persist();
  },
  clear_history: () => {
    data.history = [];
    data.corrections = [];
    data.dismissed = [];
    persist();
  },
  correct_history_entry: ({ id, text }): CorrectionResult => {
    const h = data.history.find((e) => e.id === id);
    if (!h) throw "Dictée introuvable.";
    const t = text.trim();
    if (!t) throw "Le texte ne peut pas être vide : supprimez plutôt la dictée.";
    const changes = diffWords(h.text, t);
    if (t !== h.text) data.corrections.push({ entryId: id, changes });
    h.text = t;
    h.corrected_text = t === h.final_text ? null : t;
    h.corrected_at = h.corrected_text ? new Date().toISOString() : null;
    persist();
    return { entry: structuredClone(h), changes };
  },
  get_vocabulary_suggestions: () => suggestions(),
  dismiss_vocabulary_suggestion: ({ term }) => {
    data.dismissed.push(term);
    persist();
  },
  get_audio_usage: () => audioUsage(),
  delete_all_audio: () => {
    for (const h of data.history) {
      h.has_audio = false;
      h.audio_bytes = 0;
    }
    persist();
  },
  get_dictation_audio: ({ id }) => {
    const h = data.history.find((e) => e.id === id);
    if (!h?.has_audio) throw "Aucun audio pour cette dictée.";
    return demoWav(h.duration_ms);
  },
  export_history: async ({ query, format }) => {
    await sleep(400);
    const n = searchHistory({ ...query, limit: 1e9 }).entries.filter((e) => format !== "corpus" || e.has_audio).length;
    if (!n) throw format === "corpus" ? "Aucune dictée avec audio à exporter." : "Aucune dictée à exporter.";
    const stamp = new Date().toISOString().slice(0, 19).replace(/[T:]/g, "-");
    const base = detectPlatform() === "windows" ? "C:\\Users\\vous\\Documents\\Murmure" : "~/Documents/Murmure";
    const sep = detectPlatform() === "windows" ? "\\" : "/";
    return `${base}${sep}${format === "corpus" ? `corpus-${stamp}` : `historique-${stamp}.${format}`}`;
  },
  reveal_path: () => undefined,
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
  get_acceleration: () => (detectPlatform() === "macos" ? "Metal · Apple M2" : "Vulkan · NVIDIA GeForce RTX 5050"),
  check_update: () => checkUpdate(),
  install_update: () => installUpdate(),
  get_wayland_status: () => wayland,
  wayland_authorize: () => waylandAuthorize(),
  wayland_configure_shortcut: async () => {
    await sleep(300);
    wayland = { ...wayland, shortcut_trigger: "Meta+D" };
    mockEmit("wayland-status", wayland);
  },
  wayland_refresh: () => wayland,
  export_config: ({ includeSecrets }) => exportConfig(includeSecrets),
  import_config: () => importConfig(),
  pick_audio_file: () => pickAudioFile(),
  transcribe_file: ({ path }) => transcribeFile(path),
  cancel_file_transcription: () => {
    fileCancelled = true;
  },
};

export async function mockInvoke(cmd: CommandName, args: unknown): Promise<unknown> {
  const fn = impl[cmd] as (a: unknown) => unknown;
  // Petite latence réaliste.
  await sleep(20);
  return fn(args);
}

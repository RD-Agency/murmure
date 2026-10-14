import type { Platform } from "./types";

// ---------- Raccourcis ----------

const MODIFIER_ORDER = ["Control", "Alt", "Shift", "Command"] as const;
type Mod = (typeof MODIFIER_ORDER)[number];

function normalizeModifier(part: string, platform: Platform): Mod | null {
  switch (part) {
    case "Ctrl":
    case "Control":
      return "Control";
    case "Alt":
    case "Option":
      return "Alt";
    case "Shift":
      return "Shift";
    case "Command":
    case "Cmd":
    case "Super":
    case "Meta":
      return "Command";
    case "CommandOrControl":
    case "CmdOrCtrl":
      return platform === "macos" ? "Command" : "Control";
    default:
      return null;
  }
}

const MAC_SYMBOLS: Record<Mod, string> = { Control: "⌃", Alt: "⌥", Shift: "⇧", Command: "⌘" };
const PC_LABELS: Record<Mod, string> = { Control: "Ctrl", Alt: "Alt", Shift: "Maj", Command: "Super" };

const KEY_LABELS: Record<string, string> = {
  Space: "Espace",
  Enter: "Entrée",
  Tab: "Tab",
  Backspace: "⌫",
  Delete: "Suppr",
  Escape: "Échap",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Home: "Début",
  End: "Fin",
  PageUp: "Pg ↑",
  PageDown: "Pg ↓",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
};

/** "CommandOrControl+Shift+D" → ["⌘", "⇧", "D"] (macOS) ou ["Ctrl", "Maj", "D"]. */
export function hotkeyParts(accel: string, platform: Platform): string[] {
  const parts = accel.split("+").filter(Boolean);
  const mods = new Set<Mod>();
  const keys: string[] = [];
  for (const p of parts) {
    const m = normalizeModifier(p, platform);
    if (m) mods.add(m);
    else keys.push(p);
  }
  const ordered = MODIFIER_ORDER.filter((m) => mods.has(m));
  const modLabels = ordered.map((m) => (platform === "macos" ? MAC_SYMBOLS[m] : PC_LABELS[m]));
  const keyLabels = keys.map((k) => {
    if (KEY_LABELS[k]) return KEY_LABELS[k];
    if (k.startsWith("Key")) return k.slice(3);
    if (k.startsWith("Digit")) return k.slice(5);
    if (k.startsWith("Numpad")) return "Pavé " + k.slice(6);
    return k.length === 1 ? k.toUpperCase() : k;
  });
  return [...modLabels, ...keyLabels];
}

/** Texte compact pour une phrase : « ⌥ Espace », « Ctrl + Alt + Espace ». */
export function hotkeyLabel(accel: string, platform: Platform): string {
  return hotkeyParts(accel, platform).join(platform === "macos" ? " " : " + ");
}

const CODE_TO_ACCEL: Record<string, string> = {
  Space: "Space",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  ArrowUp: "ArrowUp",
  ArrowDown: "ArrowDown",
  ArrowLeft: "ArrowLeft",
  ArrowRight: "ArrowRight",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Backquote: "Backquote",
  Minus: "Minus",
  Equal: "Equal",
  BracketLeft: "BracketLeft",
  BracketRight: "BracketRight",
  Backslash: "Backslash",
  Semicolon: "Semicolon",
  Quote: "Quote",
  Comma: "Comma",
  Period: "Period",
  Slash: "Slash",
};

/** Touche principale (hors modificateurs) au format accelerator Tauri, à partir de `KeyboardEvent.code`. */
export function codeToAccelKey(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  if (/^Numpad[0-9]$/.test(code)) return code;
  return CODE_TO_ACCEL[code] ?? null;
}

/** Construit un accelerator Tauri depuis un keydown, ou null si incomplet. */
export function eventToAccelerator(e: KeyboardEvent, platform: Platform): string | null {
  const key = codeToAccelKey(e.code);
  if (!key) return null;
  const mods: string[] = [];
  if (platform === "macos") {
    if (e.metaKey) mods.push("CommandOrControl");
    if (e.ctrlKey) mods.push("Control");
  } else {
    if (e.ctrlKey) mods.push("CommandOrControl");
    if (e.metaKey) mods.push("Super");
  }
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  const isFn = /^F\d+$/.test(key);
  if (mods.length === 0 && !isFn) return null;
  return [...mods, key].join("+");
}

/** Modificateurs actuellement enfoncés, pour l'aperçu pendant la capture. */
export function liveModifiers(e: KeyboardEvent, platform: Platform): string[] {
  const out: Mod[] = [];
  if (e.ctrlKey) out.push("Control");
  if (e.altKey) out.push("Alt");
  if (e.shiftKey) out.push("Shift");
  if (e.metaKey) out.push("Command");
  return out.map((m) => (platform === "macos" ? MAC_SYMBOLS[m] : PC_LABELS[m]));
}

// ---------- Texte, durées, dates ----------

export function countWords(text: string): number {
  const t = text.trim();
  return t ? t.split(/\s+/).length : 0;
}

const nf = new Intl.NumberFormat("fr-FR");
export function formatNumber(n: number): string {
  return nf.format(n);
}

/** 3200 → « 3,2 s », 640 → « 640 ms » */
export function formatLatency(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toLocaleString("fr-FR", { maximumFractionDigits: 1 })} s`;
}

/** Durée d'enregistrement : « 7 s », « 1 min 12 s » */
export function formatDuration(ms: number): string {
  const s = Math.max(1, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  const r = s % 60;
  return r ? `${m} min ${r} s` : `${m} min`;
}

/** Chrono de l'overlay : « 0:07 » */
export function formatClock(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function formatSize(mb: number): string {
  if (mb >= 1000) return `${(mb / 1024).toLocaleString("fr-FR", { maximumFractionDigits: 1 })} Go`;
  return `${Math.round(mb)} Mo`;
}

export function formatBytes(bytes: number): string {
  return formatSize(bytes / (1024 * 1024));
}

const rtf = new Intl.RelativeTimeFormat("fr-FR", { numeric: "auto" });
const timeFmt = new Intl.DateTimeFormat("fr-FR", { hour: "2-digit", minute: "2-digit" });
const dayFmt = new Intl.DateTimeFormat("fr-FR", { day: "numeric", month: "short" });
const fullFmt = new Intl.DateTimeFormat("fr-FR", { dateStyle: "full", timeStyle: "short" });

export function formatRelative(iso: string, now = Date.now()): string {
  const d = new Date(iso);
  const diff = (d.getTime() - now) / 1000;
  const abs = Math.abs(diff);
  if (abs < 45) return "à l'instant";
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (isSameDay(d, new Date(now))) return `aujourd'hui, ${timeFmt.format(d)}`;
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (isSameDay(d, yesterday)) return `hier, ${timeFmt.format(d)}`;
  if (abs < 6 * 86400) {
    const wd = d.toLocaleDateString("fr-FR", { weekday: "long" });
    return `${wd}, ${timeFmt.format(d)}`;
  }
  return dayFmt.format(d);
}

export function formatFullDate(iso: string): string {
  return fullFmt.format(new Date(iso));
}

export function isSameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export const LANGUAGES: { code: string; label: string }[] = [
  { code: "fr", label: "Français" },
  { code: "auto", label: "Détection automatique" },
  { code: "en", label: "Anglais" },
  { code: "es", label: "Espagnol" },
  { code: "de", label: "Allemand" },
  { code: "it", label: "Italien" },
  { code: "pt", label: "Portugais" },
  { code: "nl", label: "Néerlandais" },
  { code: "ar", label: "Arabe" },
];

/** Minuscules sans accents, pour la recherche. */
export function normalizeSearch(s: string): string {
  return s.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLowerCase();
}

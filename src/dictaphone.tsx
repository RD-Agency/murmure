// SPDX-License-Identifier: AGPL-3.0-or-later
// Fenêtre dictaphone : dicter en plusieurs fois (Enregistrer / Pause), voir le texte en direct,
// corriger, insérer une phrase n'importe où (curseur), puis tout coller dans l'application.
import { StrictMode, useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Button } from "./components/Button";
import { IconCheck, IconCopy, IconLoader, IconMic, IconPause, IconTrash } from "./components/Icons";
import { api, errorMessage, isMock, loadMock, on } from "./lib/api";
import { fitInsertion } from "./lib/insertion";
import type { DictationState } from "./lib/types";
import "./styles.css";

const DRAFT_KEY = "murmure-dictaphone-draft";
const FONT_KEY = "murmure-dictaphone-font";
const FONT_MIN = 13;
const FONT_MAX = 32;

function load(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function save(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* stockage indisponible : le brouillon vit seulement en mémoire */
  }
}

function Dictaphone() {
  const [text, setText] = useState(() => load(DRAFT_KEY) ?? "");
  const [phase, setPhase] = useState<DictationState>("idle");
  const [message, setMessage] = useState<string | null>(null);
  const [partial, setPartial] = useState("");
  const [fontSize, setFontSize] = useState(() => Number(load(FONT_KEY)) || 17);
  const [copied, setCopied] = useState(false);
  const [inserting, setInserting] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);
  const selection = useRef({ start: text.length, end: text.length });
  const textRef = useRef(text);
  textRef.current = text;

  useEffect(() => save(DRAFT_KEY, text), [text]);
  useEffect(() => save(FONT_KEY, String(fontSize)), [fontSize]);

  const remember = () => {
    const el = area.current;
    if (el) selection.current = { start: el.selectionStart, end: el.selectionEnd };
  };

  const insertDictated = useCallback((dictated: string) => {
    const current = textRef.current;
    const { start, end } = selection.current;
    const s = Math.min(start, current.length);
    const e = Math.min(Math.max(end, s), current.length);
    const piece = fitInsertion(current.slice(0, s), current.slice(e), dictated);
    if (!piece) return;
    const next = current.slice(0, s) + piece + current.slice(e);
    const caret = s + piece.length - (piece.endsWith(" ") && current.slice(e) !== "" ? 1 : 0);
    selection.current = { start: caret, end: caret };
    setText(next);
    requestAnimationFrame(() => {
      const el = area.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(caret, caret);
    });
  }, []);

  useEffect(() => {
    if (isMock) void loadMock().then((m) => m.setDictaphoneMode(true));
    const subs = [
      on("dictation-state", (p) => {
        setPhase(p.state);
        setMessage(p.state === "error" ? (p.message ?? "Erreur") : null);
        if (p.state !== "recording") setPartial("");
      }),
      on("dictaphone-partial", (p) => setPartial(p.text)),
      on("dictaphone-text", (p) => {
        setPartial("");
        insertDictated(p.text);
      }),
    ];
    return () => subs.forEach((u) => u());
  }, [insertDictated]);

  // Taille du texte : Ctrl + molette, Ctrl + « + » / « − » / « 0 ».
  useEffect(() => {
    const clamp = (n: number) => Math.min(FONT_MAX, Math.max(FONT_MIN, n));
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      setFontSize((f) => clamp(f + (e.deltaY < 0 ? 1 : -1)));
    };
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      if (e.key === "+" || e.key === "=") setFontSize((f) => clamp(f + 1));
      else if (e.key === "-") setFontSize((f) => clamp(f - 1));
      else if (e.key === "0") setFontSize(17);
      else if (e.key === "Enter") void insert();
      else return;
      e.preventDefault();
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("wheel", onWheel);
      window.removeEventListener("keydown", onKey);
    };
  });

  const toggle = () => {
    remember();
    setMessage(null);
    void api.dictaphoneToggle();
  };

  const insert = async () => {
    const value = textRef.current.trim();
    if (!value || inserting) return;
    setInserting(true);
    setMessage(null);
    try {
      await api.dictaphoneInsert(value);
      setText("");
      selection.current = { start: 0, end: 0 };
    } catch (e) {
      // La fenêtre a été masquée pour coller : on la rouvre avec l'explication.
      await api.dictaphoneOpen().catch(() => undefined);
      setMessage(errorMessage(e));
    } finally {
      setInserting(false);
    }
  };

  const copy = async () => {
    if (!text.trim()) return;
    try {
      await api.dictaphoneCopy(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      setMessage(errorMessage(e));
    }
  };

  const recording = phase === "recording";
  const transcribing = phase === "transcribing";
  const words = text.trim() ? text.trim().split(/\s+/).length : 0;

  return (
    <div className="flex h-full flex-col bg-bg text-fg">
      <header className="flex items-center gap-3 border-b border-line px-4 py-3">
        <button
          type="button"
          onClick={toggle}
          disabled={transcribing}
          aria-label={recording ? "Mettre en pause" : "Enregistrer"}
          title={recording ? "Pause (le texte s'insère au curseur)" : "Enregistrer"}
          className={`flex h-11 w-11 shrink-0 items-center justify-center rounded-full text-white shadow-soft transition-colors disabled:opacity-60 ${
            recording ? "bg-danger hover:opacity-90" : "bg-accent hover:bg-accent-hover"
          }`}
        >
          {transcribing ? <IconLoader size={18} className="animate-spin" /> : recording ? <IconPause size={18} /> : <IconMic size={18} />}
        </button>
        <div className="min-w-0 flex-1">
          <div className="text-[14px] font-medium" role="status" aria-live="polite">
            {recording ? (
              <span className="flex items-center gap-2">
                <span className="h-2 w-2 animate-pulse rounded-full bg-danger" /> Enregistrement…
              </span>
            ) : transcribing ? (
              "Transcription…"
            ) : text ? (
              "En pause"
            ) : (
              "Prêt"
            )}
          </div>
          <div className="truncate text-[12.5px] text-muted">
            Bouton ou raccourci de dictée : enregistrer / pause. Le texte s'insère là où est le curseur.
          </div>
        </div>
      </header>

      <div className="relative min-h-0 flex-1 p-3">
        <textarea
          ref={area}
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            selection.current = { start: e.target.selectionStart, end: e.target.selectionEnd };
          }}
          onSelect={remember}
          onKeyUp={remember}
          onClick={remember}
          onBlur={remember}
          spellCheck
          lang="fr"
          placeholder="Appuyez sur Enregistrer et dictez. Pause pour réfléchir, cliquez où vous voulez ajouter du texte, puis reprenez."
          aria-label="Texte dicté"
          style={{ fontSize: `${fontSize}px` }}
          className="field h-full w-full resize-none p-3 leading-relaxed"
        />
      </div>

      {(partial || message) && (
        <div className="px-4 pb-2 text-[13px]">
          {partial && <p className="truncate text-muted italic">En cours : {partial}</p>}
          {message && (
            <p role="alert" className="text-danger">
              {message}
            </p>
          )}
        </div>
      )}

      <footer className="flex items-center gap-2 border-t border-line px-4 py-3">
        <span className="text-[12.5px] text-subtle">{words > 0 ? `${words} mot${words > 1 ? "s" : ""}` : ""}</span>
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="md"
          onClick={() => {
            setText("");
            selection.current = { start: 0, end: 0 };
          }}
          disabled={!text || recording}
        >
          <IconTrash size={14} /> Effacer
        </Button>
        <Button variant="secondary" size="md" onClick={() => void copy()} disabled={!text.trim()}>
          {copied ? <IconCheck size={14} /> : <IconCopy size={14} />} {copied ? "Copié" : "Copier"}
        </Button>
        <Button
          variant="primary"
          size="md"
          onClick={() => void insert()}
          disabled={!text.trim() || recording || transcribing}
          loading={inserting}
          title="Ctrl + Entrée"
        >
          Insérer dans l'application
        </Button>
      </footer>
    </div>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <Dictaphone />
  </StrictMode>,
);

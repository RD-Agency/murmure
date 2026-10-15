// SPDX-License-Identifier: AGPL-3.0-or-later
import { type ReactNode, useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { formatDuration, formatFullDate, formatLatency, formatRelative } from "../lib/format";
import { usePlayer } from "../lib/player";
import type { CorrectionResult, HistoryEntry, TermChange } from "../lib/types";
import { Button } from "./Button";
import {
  IconAudio,
  IconCheck,
  IconCopy,
  IconLaptop,
  IconLoader,
  IconPause,
  IconPencil,
  IconPlay,
  IconServer,
  IconTrash,
} from "./Icons";

interface Props {
  entry: HistoryEntry;
  onDelete?: (id: string) => void;
  /** Correction enregistrée (mode édition disponible si fourni). */
  onCorrected?: (result: CorrectionResult) => void;
  /** Transforme une correction en règle de remplacement. */
  onCreateRule?: (change: TermChange) => void;
  compact?: boolean;
  highlight?: string;
}

const actionClass =
  "pointer-events-auto inline-flex size-7 items-center justify-center rounded-[7px] text-subtle opacity-0 transition-[opacity,color,background-color] group-focus-within:opacity-100 group-hover:opacity-100 hover:bg-surface-3 hover:text-fg focus-visible:opacity-100";

export function HistoryItem({ entry, onDelete, onCorrected, onCreateRule, compact = false, highlight }: Props) {
  const [copied, setCopied] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(entry.text);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [changes, setChanges] = useState<TermChange[]>([]);
  const [ruled, setRuled] = useState<string[]>([]);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const area = useRef<HTMLTextAreaElement | null>(null);
  const playing = usePlayer((s) => s.playingId === entry.id);
  const loadingAudio = usePlayer((s) => s.loadingId === entry.id);
  const toggleAudio = usePlayer((s) => s.toggle);

  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);
  useEffect(() => {
    if (editing && area.current) {
      area.current.focus();
      area.current.setSelectionRange(area.current.value.length, area.current.value.length);
    }
  }, [editing]);

  const copy = async () => {
    try {
      await api.copyText(entry.text);
    } catch {
      await navigator.clipboard?.writeText(entry.text).catch(() => undefined);
    }
    setCopied(true);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 1400);
  };

  const startEdit = () => {
    setDraft(entry.text);
    setError(null);
    setChanges([]);
    setEditing(true);
  };

  const save = async () => {
    if (draft.trim() === entry.text) {
      setEditing(false);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const result = await api.correctHistoryEntry(entry.id, draft);
      setEditing(false);
      setChanges(result.changes);
      setRuled([]);
      onCorrected?.(result);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  if (editing) {
    return (
      <div className="animate-fade-in rounded-[10px] bg-surface-2/60 px-4 py-3">
        <label htmlFor={`edit-${entry.id}`} className="sr-only">
          Corriger le texte de la dictée
        </label>
        <textarea
          id={`edit-${entry.id}`}
          ref={area}
          rows={Math.min(8, Math.max(3, Math.ceil(draft.length / 80)))}
          className="field resize-y leading-relaxed"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") setEditing(false);
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void save();
          }}
        />
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <p className="mr-auto text-[11.5px] text-subtle">
            {entry.corrected_text ? "Déjà corrigée. " : ""}Les termes corrigés servent à proposer du vocabulaire.
          </p>
          {error && <span className="text-[12px] text-danger">{error}</span>}
          <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
            Annuler
          </Button>
          <Button size="sm" variant="primary" loading={saving} onClick={() => void save()}>
            Enregistrer
          </Button>
        </div>
      </div>
    );
  }

  const autoFixes = entry.auto_corrections ?? [];

  return (
    <div className="group relative animate-rise">
      <button
        type="button"
        onClick={copy}
        title="Cliquer pour copier"
        aria-label={`Copier : ${entry.text}`}
        className={`block w-full rounded-[10px] text-left transition-colors hover:bg-surface-2/80 focus-visible:bg-surface-2/80 ${
          compact ? "px-3 py-2.5" : "px-4 py-3"
        }`}
      >
        <p className={`text-[13.5px] leading-relaxed whitespace-pre-line text-fg ${compact ? "line-clamp-1 pr-16" : "line-clamp-4 pr-32"}`}>
          <Highlighted text={entry.text} query={highlight} />
        </p>
        <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[11.5px] text-subtle">
          <time dateTime={entry.created_at} title={formatFullDate(entry.created_at)}>
            {formatRelative(entry.created_at)}
          </time>
          <span aria-hidden>·</span>
          <span>{formatDuration(entry.duration_ms)}</span>
          {!compact && (
            <>
              <span aria-hidden>·</span>
              <span className="inline-flex items-center gap-1">
                {entry.engine === "local" ? <IconLaptop size={11} /> : <IconServer size={11} />}
                {entry.engine === "local" ? "Local" : "Serveur"}
              </span>
              <span aria-hidden>·</span>
              <span title="Temps de transcription">{formatLatency(entry.latency_ms)}</span>
              {entry.has_audio && (
                <>
                  <span aria-hidden>·</span>
                  <span className="inline-flex items-center gap-1" title="Audio conservé">
                    <IconAudio size={11} /> Audio
                  </span>
                </>
              )}
              {entry.corrected_text && (
                <span
                  className="ml-1 rounded-[5px] bg-accent-soft px-1.5 py-px text-[11px] font-medium text-accent"
                  title={`Texte collé à l'origine : ${entry.final_text}`}
                >
                  Corrigée
                </span>
              )}
              {autoFixes.length > 0 && (
                <span
                  className="ml-1 rounded-[5px] bg-surface-3 px-1.5 py-px text-[11px] text-muted"
                  title={autoFixes.map((f) => `${f.from} devient ${f.to} (confiance ${Math.round(f.probability * 100)} %)`).join("\n")}
                >
                  {autoFixes.length === 1 ? "1 mot corrigé" : `${autoFixes.length} mots corrigés`} d'après le vocabulaire
                </span>
              )}
            </>
          )}
        </div>
      </button>
      <div className="pointer-events-none absolute top-2 right-2 flex items-center gap-0.5">
        {copied ? (
          <span className="inline-flex h-7 animate-pop items-center gap-1 rounded-[7px] bg-success-soft px-2 text-[11.5px] font-medium text-success">
            <IconCheck size={12} /> Copié
          </span>
        ) : (
          <>
            {!compact && entry.has_audio && (
              <button
                type="button"
                aria-label={playing ? "Arrêter l'écoute" : "Écouter la dictée"}
                title={playing ? "Arrêter" : "Écouter"}
                onClick={() => void toggleAudio(entry.id)}
                className={`${actionClass} ${playing || loadingAudio ? "text-accent opacity-100" : ""}`}
              >
                {loadingAudio ? <IconLoader size={14} /> : playing ? <IconPause size={14} /> : <IconPlay size={13} />}
              </button>
            )}
            {!compact && onCorrected && (
              <button type="button" aria-label="Corriger le texte" title="Corriger" onClick={startEdit} className={actionClass}>
                <IconPencil size={14} />
              </button>
            )}
            <span className="inline-flex size-7 items-center justify-center text-subtle opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
              <IconCopy size={14} />
            </span>
          </>
        )}
        {onDelete && !copied && (
          <button
            type="button"
            aria-label="Supprimer cette dictée"
            title="Supprimer"
            onClick={() => onDelete(entry.id)}
            className="pointer-events-auto inline-flex size-7 items-center justify-center rounded-[7px] text-subtle opacity-0 transition-[opacity,color,background-color] group-focus-within:opacity-100 group-hover:opacity-100 hover:bg-danger-soft hover:text-danger focus-visible:opacity-100"
          >
            <IconTrash size={14} />
          </button>
        )}
      </div>
      {changes.length > 0 && (
        <div className="mx-4 mb-2.5 animate-fade-in rounded-[8px] border border-line bg-surface-2/60 px-3 py-2 text-[12px] text-muted">
          <div className="mb-1 font-medium text-fg">Correction enregistrée</div>
          <ul className="grid gap-1">
            {changes.map((c) => (
              <li key={`${c.wrong}>${c.right}`} className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <span>
                  <span className="text-subtle line-through">{c.wrong}</span> devient <span className="font-medium text-fg">{c.right}</span>
                </span>
                {onCreateRule &&
                  (ruled.includes(c.wrong) ? (
                    <span className="inline-flex items-center gap-1 text-success">
                      <IconCheck size={12} /> Règle créée
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="font-medium text-accent underline-offset-2 hover:underline"
                      onClick={() => {
                        onCreateRule(c);
                        setRuled((r) => [...r, c.wrong]);
                      }}
                    >
                      Remplacer automatiquement à l'avenir
                    </button>
                  ))}
              </li>
            ))}
          </ul>
          <button type="button" className="mt-1.5 text-[11.5px] text-subtle hover:text-fg" onClick={() => setChanges([])}>
            Fermer
          </button>
        </div>
      )}
    </div>
  );
}

function Highlighted({ text, query }: { text: string; query?: string }) {
  const terms = (query ?? "").trim().split(/\s+/).filter(Boolean);
  if (!terms.length) return <>{text}</>;
  // Normalisation caractère par caractère pour garder les index alignés sur le texte d'origine.
  const norm = (s: string) =>
    s
      .split("")
      .map((c) => {
        const n = c.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLowerCase();
        return n.length === 1 ? n : c.toLowerCase().slice(0, 1) || c;
      })
      .join("");
  const nText = norm(text);
  // Plages à surligner : chaque mot cherché, en début de mot (recherche par préfixe).
  const ranges: [number, number][] = [];
  for (const t of terms) {
    const nQ = norm(t);
    let i = 0;
    while (nQ) {
      const idx = nText.indexOf(nQ, i);
      if (idx === -1) break;
      const prev = idx > 0 ? nText[idx - 1] : " ";
      if (!/[\p{L}\p{N}]/u.test(prev)) ranges.push([idx, idx + nQ.length]);
      i = idx + nQ.length;
    }
  }
  ranges.sort((a, b) => a[0] - b[0]);
  const out: ReactNode[] = [];
  let pos = 0;
  let k = 0;
  for (const [s, e] of ranges) {
    if (s < pos) continue;
    if (s > pos) out.push(text.slice(pos, s));
    out.push(
      <mark key={k++} className="rounded-[3px] bg-accent-soft text-accent shadow-[0_0_0_1.5px_var(--accent-soft)]">
        {text.slice(s, e)}
      </mark>,
    );
    pos = e;
  }
  out.push(text.slice(pos));
  return <>{out}</>;
}

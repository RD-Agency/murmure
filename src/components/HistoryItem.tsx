import { type ReactNode, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { formatDuration, formatFullDate, formatLatency, formatRelative } from "../lib/format";
import type { HistoryEntry } from "../lib/types";
import { IconCheck, IconCopy, IconLaptop, IconServer, IconTrash } from "./Icons";

interface Props {
  entry: HistoryEntry;
  onDelete?: (id: string) => void;
  compact?: boolean;
  highlight?: string;
}

export function HistoryItem({ entry, onDelete, compact = false, highlight }: Props) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);

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
        <p className={`pr-16 text-[13.5px] leading-relaxed text-fg ${compact ? "line-clamp-1" : "line-clamp-3"}`}>
          <Highlighted text={entry.text} query={highlight} />
        </p>
        <div className="mt-1 flex items-center gap-1.5 text-[11.5px] text-subtle">
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
          <span className="inline-flex size-7 items-center justify-center text-subtle opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
            <IconCopy size={14} />
          </span>
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
    </div>
  );
}

function Highlighted({ text, query }: { text: string; query?: string }) {
  const q = query?.trim();
  if (!q) return <>{text}</>;
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
  const nQ = norm(q);
  const out: ReactNode[] = [];
  let i = 0;
  let k = 0;
  while (true) {
    const idx = nText.indexOf(nQ, i);
    if (idx === -1) break;
    if (idx > i) out.push(text.slice(i, idx));
    out.push(
      <mark key={k++} className="rounded-[3px] bg-accent-soft text-accent shadow-[0_0_0_1.5px_var(--accent-soft)]">
        {text.slice(idx, idx + q.length)}
      </mark>,
    );
    i = idx + q.length;
  }
  out.push(text.slice(i));
  return <>{out}</>;
}

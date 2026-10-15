// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from "react";
import { formatClock } from "../lib/format";
import type { DictationStatePayload } from "../lib/types";
import { Waveform } from "./Waveform";

interface Props {
  status: DictationStatePayload;
  levelRef: { current: number };
  /** Progression en direct de l'installation du modèle, si un téléchargement tourne. */
  installPct?: number | null;
}

/** Message de dictée sans modèle prêt (voir `install::dictation_message` côté Rust). */
const INSTALL_RE = /^Installation du modèle/;

/** Pastille de dictée : enregistrement, transcription, erreur. */
export function Pill({ status, levelRef, installPct = null }: Props) {
  const visible = status.state !== "idle";
  // Garde le dernier contenu visible pendant l'animation de sortie.
  const [shown, setShown] = useState<DictationStatePayload>(status);
  const [startedAt, setStartedAt] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  const prev = useRef(status.state);

  useEffect(() => {
    if (status.state === "recording" && prev.current !== "recording") setStartedAt(Date.now());
    prev.current = status.state;
    if (status.state !== "idle") setShown(status);
  }, [status]);

  useEffect(() => {
    if (shown.state !== "recording") return;
    const t = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(t);
  }, [shown.state]);

  const s = shown.state;
  // Un collage impossible arrive en « error » avec un message du type « texte copié dans le presse-papiers ».
  const info = s === "error" && /copi/i.test(shown.message ?? "");
  // Dictée pendant l'installation du modèle : état informatif, pourcentage tenu à jour en direct.
  const installing = s === "error" && INSTALL_RE.test(shown.message ?? "");
  const parsedPct = /\((\d+)\s*%\)/.exec(shown.message ?? "")?.[1];
  const pct = installPct ?? (parsedPct ? Number(parsedPct) : null);

  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-1">
      <div
        role="status"
        aria-live="polite"
        className={`pill flex h-[38px] items-center rounded-full px-3.5 text-white transition-[opacity,transform] duration-200 ease-[cubic-bezier(0.2,0.8,0.2,1)] ${
          visible ? "scale-100 opacity-100" : "scale-[0.92] opacity-0"
        }`}
      >
        {s === "recording" && (
          <div key="rec" className="flex animate-fade-in items-center gap-2.5">
            <span className="relative flex size-[7px]">
              <span className="absolute inset-0 animate-[pulse-soft_1.4s_ease-in-out_infinite] rounded-full bg-[#ff5a52]" />
            </span>
            <Waveform levelRef={levelRef} active={visible && s === "recording"} />
            <span className="w-[30px] text-right text-[11.5px] font-medium text-white/65 tabular-nums">
              {formatClock(Math.max(0, now - startedAt))}
            </span>
          </div>
        )}
        {s === "transcribing" && (
          <div key="tr" className="flex animate-fade-in items-center gap-2.5 px-1">
            <span className="flex items-center gap-[4px]" aria-hidden>
              {[0, 1, 2].map((i) => (
                <span
                  key={i}
                  className="size-[5px] rounded-full bg-white/85"
                  style={{ animation: `dot-wave 1.1s ${i * 0.14}s ease-in-out infinite` }}
                />
              ))}
            </span>
            <span className="shimmer text-[12.5px] font-medium">Transcription</span>
          </div>
        )}
        {installing && (
          <div key="inst" className="flex animate-fade-in items-center gap-2 px-0.5">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#a9a9ff" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden className="shrink-0">
              <path d="M12 4v11" />
              <path d="m7.5 10.5 4.5 4.5 4.5-4.5" />
              <path d="M4.5 19.5h15" />
            </svg>
            <span className="text-[12.5px] font-medium text-white/90">Installation du modèle</span>
            {pct !== null && (
              <span className="w-[34px] text-right text-[11.5px] font-medium text-white/65 tabular-nums">{pct} %</span>
            )}
          </div>
        )}
        {s === "error" && !installing && (
          <div key="err" className="flex max-w-[220px] animate-fade-in items-center gap-2 px-0.5">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={info ? "#a9a9ff" : "#ff8a80"} strokeWidth="2" strokeLinecap="round" aria-hidden className="shrink-0">
              {info ? (
                <>
                  <rect x="5" y="4.5" width="14" height="16.5" rx="2.2" />
                  <path d="M9 4.5V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v.5" />
                </>
              ) : (
                <>
                  <circle cx="12" cy="12" r="8.5" />
                  <path d="M12 7.8v4.7M12 16h.01" />
                </>
              )}
            </svg>
            <span className="truncate text-[12.5px] font-medium text-white/90">{shown.message || "Une erreur est survenue"}</span>
          </div>
        )}
      </div>
      <div
        className={`pill-chip rounded-full px-2.5 py-[2px] text-[10.5px] font-medium text-white/75 transition-[opacity,transform] duration-200 ${
          visible && ((s === "recording" && shown.hands_free) || installing) ? "translate-y-0 opacity-100" : "-translate-y-1 opacity-0"
        }`}
        aria-hidden={!((s === "recording" && shown.hands_free) || installing)}
      >
        {installing ? (pct !== null ? "Réessayez dans un instant" : "Murmure télécharge son modèle") : "Mains libres · Échap pour annuler"}
      </div>
    </div>
  );
}

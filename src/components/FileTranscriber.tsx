// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { formatDuration } from "../lib/format";
import { useApp } from "../store";
import { Button } from "./Button";
import { Card } from "./Card";
import { IconCheck, IconCopy, IconFileAudio } from "./Icons";
import { Progress } from "./Progress";

/** Accueil : transcription d'un enregistrement de dictaphone, avec progression et annulation. */
export function FileTranscriber() {
  const job = useApp((s) => s.fileJob);
  const start = useApp((s) => s.transcribeFile);
  const cancel = useApp((s) => s.cancelFileTranscription);
  const dismiss = useApp((s) => s.dismissFileJob);
  const [copied, setCopied] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);
  useEffect(() => {
    if (job.status !== "running") setCancelling(false);
  }, [job.status]);

  const copy = async () => {
    if (!job.result) return;
    try {
      await api.copyText(job.result.text);
    } catch {
      await navigator.clipboard?.writeText(job.result.text).catch(() => undefined);
    }
    setCopied(true);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 1400);
  };

  const p = job.progress;
  const stageLabel =
    !p || p.stage === "decoding"
      ? "Lecture du fichier…"
      : `Transcription · segment ${Math.min(p.done + 1, p.total)} sur ${p.total}`;

  return (
    <Card className="px-5 py-4">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
        <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-surface-2 text-subtle">
          <IconFileAudio size={16} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-fg">Transcrire un fichier audio</div>
          <p className="mt-0.5 text-[12.5px] text-muted">
            Enregistrement de dictaphone ou mémo vocal : WAV, MP3, M4A, Ogg, Opus ou FLAC.
          </p>
        </div>
        {job.status !== "running" && (
          <Button onClick={() => void start()}>Transcrire un fichier…</Button>
        )}
      </div>

      {job.status === "running" && (
        <div className="mt-4 animate-fade-in">
          <div className="mb-2 flex items-baseline justify-between gap-3 text-[12.5px]">
            <span className="truncate text-fg">
              <span className="font-medium">{job.name}</span>
              <span className="text-muted"> · {stageLabel}</span>
            </span>
            <span className="shrink-0 text-muted tabular-nums">{Math.round((p?.fraction ?? 0) * 100)} %</span>
          </div>
          <Progress value={p?.fraction ?? 0} label="Transcription du fichier" />
          <div className="mt-3 flex justify-end">
            <Button
              size="sm"
              variant="ghost"
              loading={cancelling}
              onClick={() => {
                setCancelling(true);
                void cancel();
              }}
            >
              {cancelling ? "Annulation après le segment en cours…" : "Annuler"}
            </Button>
          </div>
        </div>
      )}

      {job.status === "error" && (
        <div className="mt-4 flex items-start justify-between gap-3 animate-fade-in">
          <p className="text-[12.5px] text-danger" role="alert">
            {job.error}
          </p>
          <Button size="sm" variant="ghost" onClick={dismiss}>
            Fermer
          </Button>
        </div>
      )}

      {job.status === "done" && job.result && (
        <div className="mt-4 animate-fade-in">
          <div className="max-h-64 overflow-y-auto rounded-[10px] border border-line bg-surface-2/50 px-4 py-3">
            <p className="text-[13.5px] leading-relaxed whitespace-pre-wrap text-fg select-text">{job.result.text}</p>
          </div>
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
            <span className="text-[11.5px] text-subtle">
              {job.result.file_name} · {formatDuration(job.result.duration_ms)} d'audio
            </span>
            <div className="flex gap-2">
              <Button size="sm" variant="ghost" onClick={dismiss}>
                Fermer
              </Button>
              <Button size="sm" variant="primary" icon={copied ? <IconCheck size={13} /> : <IconCopy size={13} />} onClick={copy}>
                {copied ? "Copié" : "Copier le texte"}
              </Button>
            </div>
          </div>
        </div>
      )}
    </Card>
  );
}

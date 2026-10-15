// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { downloadPercent, formatProgressSize, formatSize } from "../lib/format";
import type { ModelInfo } from "../lib/types";
import { useApp } from "../store";
import { Badge } from "./Badge";
import { Button } from "./Button";
import { IconAlert, IconCheck, IconDownload, IconTrash } from "./Icons";
import { Progress } from "./Progress";

export function ModelList() {
  const models = useApp((s) => s.models);
  const selected = useApp((s) => s.settings?.local_model);
  const updateSettings = useApp((s) => s.updateSettings);

  if (!models.length) {
    return <div className="px-4 py-6 text-center text-[13px] text-muted">Aucun modèle disponible.</div>;
  }

  return (
    <div role="radiogroup" aria-label="Modèle local" className="divide-y divide-line">
      {models.map((m) => (
        <ModelRow
          key={m.id}
          model={m}
          selected={m.id === selected}
          onSelect={() => updateSettings({ local_model: m.id })}
        />
      ))}
    </div>
  );
}

function ModelRow({ model, selected, onSelect }: { model: ModelInfo; selected: boolean; onSelect: () => void }) {
  const progress = useApp((s) => s.downloads[model.id]);
  const error = useApp((s) => s.downloadErrors[model.id]);
  const download = useApp((s) => s.downloadModel);
  const cancel = useApp((s) => s.cancelDownload);
  const remove = useApp((s) => s.deleteModel);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const downloading = !!progress && !progress.done;
  const percent = progress ? downloadPercent(progress.downloaded, progress.total) : 0;
  const selectable = model.downloaded;

  return (
    <div className={`px-4 py-3.5 transition-colors ${selected && model.downloaded ? "bg-accent-soft/50" : ""}`}>
      <div className="flex items-center gap-3.5">
        <button
          type="button"
          role="radio"
          aria-checked={selected}
          aria-label={`Utiliser ${model.name}`}
          disabled={!selectable}
          onClick={onSelect}
          className={`flex size-[18px] shrink-0 items-center justify-center rounded-full border transition-colors ${
            selected && selectable
              ? "border-accent bg-accent"
              : selectable
                ? "border-line-strong bg-surface hover:border-accent"
                : "border-line bg-surface-2 opacity-60"
          }`}
        >
          {selected && selectable && <span className="size-[7px] rounded-full bg-accent-fg" />}
        </button>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-[13px] font-medium text-fg">{model.name}</span>
            {model.recommended && <Badge tone="accent">Recommandé</Badge>}
            {model.downloaded && (
              <span className="inline-flex items-center gap-1 text-[11.5px] text-success">
                <IconCheck size={12} /> Sur l'appareil
              </span>
            )}
          </div>
          <div className="mt-0.5 text-[12.5px] text-muted">
            {model.description} <span className="text-subtle">· {formatSize(model.size_mb)}</span>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {downloading ? (
            <Button size="sm" variant="ghost" onClick={() => cancel(model.id)}>
              Annuler
            </Button>
          ) : model.downloaded ? (
            confirmDelete ? (
              <span className="inline-flex animate-fade-in items-center gap-1.5">
                <Button size="sm" variant="ghost" onClick={() => setConfirmDelete(false)}>
                  Garder
                </Button>
                <Button
                  size="sm"
                  variant="danger"
                  loading={deleting}
                  onClick={async () => {
                    setDeleting(true);
                    try {
                      await remove(model.id);
                    } finally {
                      setDeleting(false);
                      setConfirmDelete(false);
                    }
                  }}
                >
                  Supprimer
                </Button>
              </span>
            ) : (
              <Button
                size="sm"
                variant="ghost"
                icon={<IconTrash size={14} />}
                aria-label={`Supprimer ${model.name}`}
                onClick={() => setConfirmDelete(true)}
              >
                <span className="sr-only">Supprimer</span>
              </Button>
            )
          ) : (
            <Button size="sm" icon={<IconDownload size={14} />} onClick={() => download(model.id)}>
              Télécharger
            </Button>
          )}
        </div>
      </div>
      {downloading && (
        <div className="mt-3 ml-8 flex animate-fade-in items-center gap-3">
          <Progress value={percent / 100} label={`Installation de ${model.name}`} />
          <span className="w-[150px] shrink-0 text-right text-[11.5px] text-muted tabular-nums">
            {percent} % · {formatProgressSize(progress.downloaded, progress.total)}
          </span>
        </div>
      )}
      {error && !downloading && (
        <div className="mt-2 ml-8 flex items-center gap-1.5 text-[12px] text-danger">
          <IconAlert size={13} /> {error}
        </div>
      )}
    </div>
  );
}

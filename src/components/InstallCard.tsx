import { downloadPercent, formatProgressSize } from "../lib/format";
import type { DownloadProgress, ModelInfo } from "../lib/types";
import { useApp } from "../store";
import { Card } from "./Card";
import { IconDownload } from "./Icons";
import { Progress } from "./Progress";

export interface ActiveInstall {
  progress: DownloadProgress;
  model: ModelInfo | undefined;
  percent: number;
}

/** Téléchargement de modèle en cours, de préférence celui du modèle choisi. */
export function useActiveInstall(): ActiveInstall | null {
  const downloads = useApp((s) => s.downloads);
  const models = useApp((s) => s.models);
  const selected = useApp((s) => s.settings?.local_model);
  const list = Object.values(downloads).filter((d) => !d.done);
  const progress = list.find((d) => d.id === selected) ?? list[0];
  if (!progress) return null;
  return {
    progress,
    model: models.find((m) => m.id === progress.id),
    percent: downloadPercent(progress.downloaded, progress.total),
  };
}

/** Carte discrète de l'accueil : « Installation du modèle · 42 % · 312 / 547 Mo ». */
export function InstallCard() {
  const install = useActiveInstall();
  if (!install) return null;
  const { progress, model, percent } = install;
  return (
    <Card className="animate-fade-in px-4 py-3">
      <div className="flex items-center gap-3">
        <div className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-accent-soft text-accent">
          <IconDownload size={14} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline justify-between gap-3 text-[12.5px]">
            <span className="truncate text-fg">
              <span className="font-medium">Installation du modèle</span>
              <span className="text-muted tabular-nums"> · {percent} % · {formatProgressSize(progress.downloaded, progress.total)}</span>
            </span>
            {model && <span className="hidden shrink-0 text-[11.5px] text-subtle sm:inline">{model.name}</span>}
          </div>
          <div className="mt-2">
            <Progress value={percent / 100} label="Installation du modèle" />
          </div>
        </div>
      </div>
    </Card>
  );
}

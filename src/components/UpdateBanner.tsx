import { type ReactNode, useEffect, useState } from "react";
import { api } from "../lib/api";
import { downloadPercent, formatProgressSize } from "../lib/format";
import { useApp } from "../store";
import { Button, IconButton } from "./Button";
import { IconAlert, IconCheck, IconDownload, IconRefresh, IconSparkle, IconX } from "./Icons";
import { Progress } from "./Progress";

/** Texte de progression : « 42 % · 7,6 / 18 Mo », ou « Préparation… » avant le premier octet. */
function progressText(downloaded: number, total: number): string {
  if (total > 0) return `${downloadPercent(downloaded, total)} % · ${formatProgressSize(downloaded, total)}`;
  return downloaded > 0 ? `${(downloaded / 1_048_576).toFixed(1).replace(".", ",")} Mo` : "Préparation…";
}

/**
 * Bannière discrète en haut de l'accueil. Mise à jour prête (cas normal, tout est automatique) :
 * « Murmure x.y.z est prête » + « Redémarrer maintenant ». Sinon : « disponible · Installer ».
 */
export function UpdateBanner() {
  const update = useApp((s) => s.update);
  const install = useApp((s) => s.installUpdate);
  const [dismissed, setDismissed] = useState<string | null>(null);

  const installing = update.status === "installing";
  const ready = update.status === "ready";
  const info = update.available;
  if (!info && !installing) return null;
  if (!installing && info && dismissed === info.version) return null;

  const downloaded = update.progress?.downloaded ?? 0;
  const total = update.progress?.total ?? 0;

  return (
    <div className="animate-fade-in rounded-xl border border-accent/20 bg-accent-soft px-4 py-2.5">
      <div className="flex items-center gap-3">
        <span className="flex size-6 shrink-0 items-center justify-center text-accent">
          {installing ? <IconDownload size={14} /> : <IconSparkle size={14} />}
        </span>
        <div className="min-w-0 flex-1 text-[12.5px]">
          {installing ? (
            <span className="text-fg">
              <span className="font-medium">Installation de Murmure {info?.version ?? ""}</span>
              <span className="text-muted tabular-nums"> · {progressText(downloaded, total)}</span>
            </span>
          ) : ready ? (
            <span className="text-fg" title={info?.notes ?? undefined}>
              <span className="font-medium">Murmure {info?.version} est prête</span>
              <span className="text-muted"> · elle s'installera toute seule au prochain redémarrage</span>
              {update.error && <span className="text-danger"> · {update.error}</span>}
            </span>
          ) : (
            <span className="text-fg" title={info?.notes ?? undefined}>
              <span className="font-medium">Murmure {info?.version} est disponible</span>
              {update.error && <span className="text-danger"> · {update.error}</span>}
            </span>
          )}
        </div>
        {!installing && info && (
          <>
            <Button size="sm" variant="primary" onClick={() => void install()}>
              {ready ? "Redémarrer maintenant" : "Installer"}
            </Button>
            <IconButton label="Plus tard" onClick={() => setDismissed(info.version)}>
              <IconX size={13} />
            </IconButton>
          </>
        )}
      </div>
      {installing && (
        <div className="mt-2 pl-9">
          <Progress value={total > 0 ? downloaded / total : 0} label="Téléchargement de la mise à jour" />
          <p className="mt-1.5 text-[11.5px] text-subtle">Murmure redémarrera automatiquement une fois la mise à jour installée.</p>
        </div>
      )}
    </div>
  );
}

/** Réglages › Général : version actuelle + « Rechercher les mises à jour ». */
export function UpdateRow() {
  const update = useApp((s) => s.update);
  const check = useApp((s) => s.checkUpdate);
  const install = useApp((s) => s.installUpdate);
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    api.getVersion().then(setVersion).catch(() => setVersion(null));
  }, []);

  const { status, available, progress, error } = update;
  const current = version ? `Version ${version}` : "Version inconnue";

  let description: ReactNode = current;
  switch (status) {
    case "checking":
      description = `${current} · Recherche en cours…`;
      break;
    case "uptodate":
      description = (
        <span className="inline-flex items-center gap-1">
          {current} · <IconCheck size={13} className="text-success" /> Murmure est à jour.
        </span>
      );
      break;
    case "available":
      description = (
        <>
          {current} · <span className="text-accent">Version {available?.version} disponible.</span>
          {error && <span className="text-danger"> {error}</span>}
        </>
      );
      break;
    case "ready":
      description = (
        <>
          {current} · <span className="text-accent">Version {available?.version} prête, installée automatiquement au prochain redémarrage.</span>
          {error && <span className="text-danger"> {error}</span>}
        </>
      );
      break;
    case "installing":
      description = `${current} · Installation de la version ${available?.version ?? ""} — ${progressText(progress?.downloaded ?? 0, progress?.total ?? 0)}`;
      break;
    case "error":
      description = (
        <span className="inline-flex items-center gap-1">
          {current} · <IconAlert size={13} className="text-danger" />
          <span className="text-danger">{error}</span>
        </span>
      );
      break;
  }

  return (
    <div className="flex items-center gap-6 px-4 py-3.5">
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-fg">Mises à jour</div>
        <div className="mt-0.5 text-[12.5px] leading-relaxed text-muted tabular-nums">{description}</div>
        {status === "installing" && (
          <div className="mt-2 max-w-[360px]">
            <Progress
              value={progress && progress.total > 0 ? progress.downloaded / progress.total : 0}
              label="Téléchargement de la mise à jour"
            />
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {status === "ready" ? (
          <Button size="sm" variant="primary" icon={<IconRefresh size={13} />} onClick={() => void install()}>
            Redémarrer maintenant
          </Button>
        ) : status === "available" ? (
          <Button size="sm" variant="primary" icon={<IconDownload size={13} />} onClick={() => void install()}>
            Installer et redémarrer
          </Button>
        ) : (
          <Button
            size="sm"
            icon={<IconRefresh size={13} />}
            loading={status === "checking"}
            disabled={status === "installing"}
            onClick={() => void check()}
          >
            Rechercher les mises à jour
          </Button>
        )}
      </div>
    </div>
  );
}

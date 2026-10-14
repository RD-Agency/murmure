import { downloadPercent, hostOf } from "../lib/format";
import { useApp } from "../store";
import { StatusDot } from "./Badge";
import { Button } from "./Button";
import { IconLaptop, IconServer } from "./Icons";

/** Résumé de l'état du moteur de transcription. */
export function EngineStatus() {
  const settings = useApp((s) => s.settings)!;
  const models = useApp((s) => s.models);
  const hasApiKey = useApp((s) => s.hasApiKey);
  const downloads = useApp((s) => s.downloads);
  const setView = useApp((s) => s.setView);

  const local = settings.engine === "local";
  const model = models.find((m) => m.id === settings.local_model);
  const downloading = model ? downloads[model.id] : undefined;

  let tone: "success" | "warning" | "accent" = "success";
  let status = "";
  let detail = "";

  if (local) {
    detail = model?.name ?? settings.local_model;
    if (model?.downloaded) {
      status = "Prêt · hors ligne";
    } else if (downloading) {
      tone = "accent";
      status = `Installation… ${downloadPercent(downloading.downloaded, downloading.total)} %`;
    } else {
      tone = "warning";
      status = "Modèle non téléchargé";
    }
  } else {
    detail = settings.remote_endpoint ? hostOf(settings.remote_endpoint) : "Aucun serveur configuré";
    if (!settings.remote_endpoint) {
      tone = "warning";
      status = "À configurer";
    } else {
      status = hasApiKey ? "Connecté · clé enregistrée" : "Sans clé API";
    }
  }

  return (
    <div className="flex items-center gap-3.5">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-[10px] border border-line bg-surface-2 text-muted">
        {local ? <IconLaptop size={17} /> : <IconServer size={17} />}
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-medium text-fg">{detail}</div>
        <div className="text-[12px] text-subtle">{local ? "Sur cet appareil" : "Serveur"}</div>
      </div>
      <div className="flex shrink-0 items-center gap-2 text-[12.5px] text-muted">
        <StatusDot tone={tone} />
        {status}
      </div>
      {tone === "warning" && (
        <Button size="sm" onClick={() => setView("settings")}>
          Configurer
        </Button>
      )}
    </div>
  );
}

/** Vrai si le moteur courant peut transcrire. */
export function useEngineReady(): boolean {
  return useApp((s) => {
    const st = s.settings;
    if (!st) return false;
    if (st.engine === "remote") return st.remote_endpoint.trim().length > 0;
    return s.models.some((m) => m.id === st.local_model && m.downloaded);
  });
}

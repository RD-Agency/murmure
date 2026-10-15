// SPDX-License-Identifier: AGPL-3.0-or-later
import { type ReactNode, useEffect, useState } from "react";
import { api, errorMessage } from "../lib/api";
import type { PortalState, WaylandStatus } from "../lib/types";
import { useApp } from "../store";
import { Badge } from "./Badge";
import { Button } from "./Button";
import { IconClipboard, IconInfo, IconKeyboard } from "./Icons";
import { Kbd } from "./Kbd";

/** État Wayland (null hors session Wayland). Relit le raccourci au retour du focus (modifié dans KDE ?). */
export function useWayland(): WaylandStatus | null {
  const wayland = useApp((s) => s.wayland);
  const active = !!wayland?.wayland;
  useEffect(() => {
    if (!active) return;
    const refresh = () => void api.waylandRefresh().catch(() => undefined);
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [active]);
  return active ? wayland : null;
}

/** Bouton « Autoriser Murmure » : déclenche les deux demandes de portail. */
export function useAuthorize() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const authorize = async () => {
    setBusy(true);
    setError(null);
    try {
      useApp.setState({ wayland: await api.waylandAuthorize() });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, authorize };
}

function StateBadge({ state }: { state: PortalState }) {
  switch (state) {
    case "active":
      return <Badge tone="success">Autorisé</Badge>;
    case "pending":
      return <Badge tone="accent">En attente de votre réponse…</Badge>;
    case "denied":
      return <Badge tone="warning">Refusé</Badge>;
    case "unavailable":
      return <Badge tone="warning">Indisponible</Badge>;
    default:
      return <Badge>À autoriser</Badge>;
  }
}

/** « Ctrl+Alt+Espace » → touches. */
export function TriggerKeys({ trigger, size = "sm" }: { trigger: string; size?: "sm" | "md" | "lg" }) {
  const parts = trigger.split("+").map((p) => p.trim()).filter(Boolean);
  return (
    <span className={`inline-flex items-center ${size === "lg" ? "gap-2" : "gap-1"}`} aria-label={parts.join(" + ")}>
      {parts.map((p, i) => (
        <Kbd key={`${p}-${i}`} size={size}>
          {p}
        </Kbd>
      ))}
    </span>
  );
}

/** Procédure de secours : raccourci KDE lié à `murmure --toggle`. */
export function ToggleFallbackHelp() {
  return (
    <div className="flex gap-3 rounded-[10px] bg-surface-2/70 px-3.5 py-3 text-[12.5px] leading-relaxed text-muted">
      <IconInfo size={15} className="mt-[2px] shrink-0 text-subtle" />
      <div>
        <p className="font-medium text-fg">Raccourci de secours</p>
        <p className="mt-0.5">
          Dans KDE : <b>Configuration du système › Clavier › Raccourcis</b>, bouton <b>Ajouter nouveau › Commande ou script</b>,
          commande <code className="rounded bg-surface-3 px-1 py-px font-mono text-[11.5px] text-fg">murmure --toggle</code>,
          puis attribuez la touche de votre choix. Un appui démarre la dictée, un second la termine (le maintien n'est pas
          possible dans ce mode).
        </p>
      </div>
    </div>
  );
}

function PanelRow({
  icon,
  title,
  state,
  description,
  error,
  children,
}: {
  icon: ReactNode;
  title: string;
  state: PortalState;
  description: ReactNode;
  error: string | null;
  children?: ReactNode;
}) {
  return (
    <div className="flex items-start gap-4 px-4 py-4">
      <div
        className={`flex size-9 shrink-0 items-center justify-center rounded-[10px] transition-colors ${
          state === "active" ? "bg-success-soft text-success" : "bg-surface-2 text-muted"
        }`}
      >
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-[13px] font-medium text-fg">{title}</span>
          <StateBadge state={state} />
        </div>
        <div className="mt-0.5 text-[12.5px] leading-relaxed text-muted">{description}</div>
        {error && state !== "active" && <p className="mt-1 text-[12px] text-danger">{error}</p>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2 self-center">{children}</div>}
    </div>
  );
}

/**
 * Wayland : raccourci global (portail GlobalShortcuts) et collage (portail RemoteDesktop).
 * Utilisé dans Réglages › Raccourci et dans l'onboarding.
 */
export function WaylandPanel({ status, showFallbackHelp = true }: { status: WaylandStatus; showFallbackHelp?: boolean }) {
  const { busy, error, authorize } = useAuthorize();
  const [configError, setConfigError] = useState<string | null>(null);
  const shortcutOk = status.shortcut === "active";
  const keyboardOk = status.keyboard === "active";
  const pending = busy || status.shortcut === "pending" || status.keyboard === "pending";

  const configure = async () => {
    setConfigError(null);
    try {
      await api.waylandConfigureShortcut();
    } catch (e) {
      setConfigError(errorMessage(e));
    }
  };

  const fallback =
    status.paste_fallback === "ydotool"
      ? "En attendant, Murmure colle avec ydotool."
      : status.paste_fallback === "wtype"
        ? "En attendant, Murmure essaie wtype."
        : "En attendant, le texte est copié : collez-le avec Ctrl+V.";

  return (
    <div className="divide-y divide-line">
      <PanelRow
        icon={<IconKeyboard size={18} />}
        title="Raccourci global"
        state={status.shortcut}
        error={configError ?? status.shortcut_error}
        description={
          shortcutOk ? (
            status.shortcut_trigger ? (
              <span className="inline-flex flex-wrap items-center gap-1.5">
                Attribué par le bureau : <TriggerKeys trigger={status.shortcut_trigger} />
              </span>
            ) : (
              "Aucune touche attribuée : choisissez-la dans le réglage du bureau."
            )
          ) : (
            "Sous Wayland, c'est le bureau (KDE, GNOME) qui attribue le raccourci : il vous le demandera une seule fois."
          )
        }
      >
        {shortcutOk ? (
          status.shortcut_configurable && (
            <Button size="sm" onClick={configure}>
              Modifier…
            </Button>
          )
        ) : null}
      </PanelRow>
      <PanelRow
        icon={<IconClipboard size={18} />}
        title="Collage automatique"
        state={status.keyboard}
        error={status.keyboard_error}
        description={
          keyboardOk
            ? "Murmure tape Ctrl+V pour vous dans l'application active. Autorisation mémorisée par le bureau."
            : `Le bureau demande d'autoriser Murmure à « contrôler le clavier » : il ne fait que coller le texte dicté. ${fallback}`
        }
      />
      {(!shortcutOk || !keyboardOk) && (
        <div className="flex flex-col items-end gap-2 px-4 py-3.5">
          <Button variant="primary" size="sm" loading={pending} onClick={authorize}>
            Autoriser Murmure
          </Button>
          {error && <p className="text-[12px] text-danger">{error}</p>}
        </div>
      )}
      {showFallbackHelp && (status.shortcut === "denied" || status.shortcut === "unavailable") && (
        <div className="px-4 py-3.5">
          <ToggleFallbackHelp />
        </div>
      )}
    </div>
  );
}

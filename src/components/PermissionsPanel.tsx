// SPDX-License-Identifier: AGPL-3.0-or-later
import { type ReactNode, useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { Permissions, Platform } from "../lib/types";
import { Badge } from "./Badge";
import { Button } from "./Button";
import { IconAccessibility, IconExternal, IconMic } from "./Icons";

/** Statut des permissions, rafraîchi périodiquement et au retour du focus. */
export function usePermissions(active = true) {
  const [perms, setPerms] = useState<Permissions | null>(null);
  const refresh = useCallback(async () => {
    try {
      setPerms(await api.checkPermissions());
    } catch {
      setPerms({ microphone: "unknown", accessibility: "unknown" });
    }
  }, []);
  useEffect(() => {
    if (!active) return;
    void refresh();
    const t = setInterval(refresh, 2000);
    window.addEventListener("focus", refresh);
    return () => {
      clearInterval(t);
      window.removeEventListener("focus", refresh);
    };
  }, [active, refresh]);
  return { perms, setPerms, refresh };
}

export function PermissionsPanel({ platform }: { platform: Platform }) {
  const { perms, setPerms } = usePermissions();
  const [requesting, setRequesting] = useState(false);

  const mic = perms?.microphone ?? "unknown";
  const acc = perms?.accessibility ?? "unknown";
  const showAccessibility = platform === "macos";

  return (
    <div className="divide-y divide-line">
      <PermissionRow
        icon={<IconMic size={18} />}
        title="Microphone"
        description="Pour entendre votre voix pendant que vous maintenez le raccourci. Rien n'est enregistré en dehors."
        status={mic}
      >
        {mic === "granted" ? null : mic === "undetermined" ? (
          <Button
            variant="primary"
            size="sm"
            loading={requesting}
            onClick={async () => {
              setRequesting(true);
              try {
                setPerms(await api.requestMicrophone());
              } finally {
                setRequesting(false);
              }
            }}
          >
            Autoriser
          </Button>
        ) : mic === "denied" ? (
          <Button size="sm" icon={<IconExternal size={13} />} onClick={() => api.openPermissionSettings("microphone")}>
            Ouvrir les réglages
          </Button>
        ) : null}
      </PermissionRow>
      {showAccessibility && (
        <PermissionRow
          icon={<IconAccessibility size={18} />}
          title="Accessibilité"
          description="Pour coller le texte dicté directement dans l'application active."
          status={acc}
        >
          {acc !== "granted" && (
            <Button
              size="sm"
              variant={acc === "denied" ? "primary" : "secondary"}
              icon={<IconExternal size={13} />}
              onClick={() => api.openPermissionSettings("accessibility")}
            >
              Ouvrir les réglages
            </Button>
          )}
        </PermissionRow>
      )}
    </div>
  );
}

function PermissionRow({
  icon,
  title,
  description,
  status,
  children,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  status: Permissions["microphone"];
  children?: ReactNode;
}) {
  const badge =
    status === "granted" ? (
      <Badge tone="success">Autorisé</Badge>
    ) : status === "denied" ? (
      <Badge tone="warning">Non autorisé</Badge>
    ) : status === "undetermined" ? (
      <Badge>À autoriser</Badge>
    ) : (
      <Badge>Géré par le système</Badge>
    );
  return (
    <div className="flex items-center gap-4 px-4 py-4">
      <div
        className={`flex size-9 shrink-0 items-center justify-center rounded-[10px] transition-colors ${
          status === "granted" ? "bg-success-soft text-success" : "bg-surface-2 text-muted"
        }`}
      >
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[13px] font-medium text-fg">{title}</span>
          {badge}
        </div>
        <p className="mt-0.5 text-[12.5px] leading-relaxed text-muted">{description}</p>
      </div>
      {children}
    </div>
  );
}

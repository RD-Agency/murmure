// SPDX-License-Identifier: AGPL-3.0-or-later
import { useApp } from "../store";
import { IconAlert, IconCheck, IconLoader } from "./Icons";

export function SaveIndicator() {
  const status = useApp((s) => s.saveStatus);
  const error = useApp((s) => s.saveError);
  return (
    <div aria-live="polite" className="flex h-6 items-center text-[12px]">
      {status === "saving" && (
        <span key="saving" className="inline-flex animate-fade-in items-center gap-1.5 text-subtle">
          <IconLoader size={12} /> Enregistrement…
        </span>
      )}
      {status === "saved" && (
        <span key="saved" className="inline-flex animate-fade-in items-center gap-1.5 text-success">
          <IconCheck size={13} /> Enregistré
        </span>
      )}
      {status === "error" && (
        <span key="err" className="inline-flex animate-fade-in items-center gap-1.5 text-danger" title={error ?? undefined}>
          <IconAlert size={13} /> Échec de l'enregistrement
        </span>
      )}
    </div>
  );
}

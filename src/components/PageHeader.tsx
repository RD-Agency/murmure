// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode } from "react";

/** En-tête de page, servant aussi de zone de drag de la fenêtre. */
export function PageHeader({ title, subtitle, right }: { title: string; subtitle?: ReactNode; right?: ReactNode }) {
  return (
    <header data-tauri-drag-region className="flex shrink-0 items-end gap-4 px-10 pt-12 pb-6">
      <div data-tauri-drag-region className="min-w-0 flex-1">
        <h1 data-tauri-drag-region className="text-[22px] leading-tight font-semibold tracking-[-0.02em] text-fg">
          {title}
        </h1>
        {subtitle && (
          <p data-tauri-drag-region className="mt-1 text-[13px] text-muted">
            {subtitle}
          </p>
        )}
      </div>
      {right && <div className="flex shrink-0 items-center gap-2">{right}</div>}
    </header>
  );
}

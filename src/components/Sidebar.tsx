import type { ReactNode } from "react";
import { hotkeyParts } from "../lib/format";
import type { View } from "../lib/types";
import { useApp } from "../store";
import { IconHistory, IconHome, IconSettings } from "./Icons";
import { Kbd } from "./Kbd";
import { Logo } from "./Logo";

const ITEMS: { view: View; label: string; icon: ReactNode }[] = [
  { view: "home", label: "Accueil", icon: <IconHome size={16} /> },
  { view: "history", label: "Historique", icon: <IconHistory size={16} /> },
  { view: "settings", label: "Réglages", icon: <IconSettings size={16} /> },
];

export function Sidebar() {
  const view = useApp((s) => s.view);
  const setView = useApp((s) => s.setView);
  const platform = useApp((s) => s.platform);
  const settings = useApp((s) => s.settings);
  const mac = platform === "macos";

  return (
    <aside className="flex w-[208px] shrink-0 flex-col border-r border-line bg-sidebar">
      {/* Zone de drag + feux tricolores (macOS, titlebar en overlay). */}
      <div data-tauri-drag-region className={mac ? "h-[52px] shrink-0" : "h-3 shrink-0"} />
      <div data-tauri-drag-region className="flex items-center gap-2.5 px-4 pb-5">
        <Logo size={24} />
        <span data-tauri-drag-region className="text-[14px] font-semibold tracking-[-0.01em] text-fg">
          Murmure
        </span>
      </div>
      <nav aria-label="Navigation principale" className="flex flex-col gap-0.5 px-2.5">
        {ITEMS.map((item) => {
          const active = item.view === view;
          return (
            <button
              key={item.view}
              type="button"
              onClick={() => setView(item.view)}
              aria-current={active ? "page" : undefined}
              className={`group flex h-8 items-center gap-2.5 rounded-lg px-2.5 text-[13px] transition-colors ${
                active
                  ? "bg-surface font-medium text-fg shadow-[0_0_0_0.5px_var(--line-strong),0_1px_2px_rgba(0,0,0,0.04)]"
                  : "text-muted hover:bg-surface-3/60 hover:text-fg"
              }`}
            >
              <span className={active ? "text-accent" : "text-subtle group-hover:text-muted"}>{item.icon}</span>
              {item.label}
            </button>
          );
        })}
      </nav>
      <div data-tauri-drag-region className="flex-1" />
      {settings && (
        <div className="m-2.5 rounded-[10px] border border-line bg-surface/60 px-3 py-2.5">
          <div className="text-[11.5px] leading-snug text-muted">Maintenez pour dicter</div>
          <div className="mt-1.5 flex items-center gap-1">
            {hotkeyParts(settings.hotkey, platform).map((p, i) => (
              <Kbd key={`${p}-${i}`} size="sm">
                {p}
              </Kbd>
            ))}
          </div>
        </div>
      )}
    </aside>
  );
}

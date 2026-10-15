// SPDX-License-Identifier: AGPL-3.0-or-later
import { hotkeyParts } from "../lib/format";
import type { Platform } from "../lib/types";

export function Kbd({ children, size = "md" }: { children: string; size?: "sm" | "md" | "lg" }) {
  const sizes = {
    sm: "h-[22px] min-w-[22px] px-1.5 text-[11.5px] rounded-[6px]",
    md: "h-7 min-w-7 px-2 text-[13px] rounded-[7px]",
    lg: "h-11 min-w-11 px-3.5 text-[17px] rounded-[10px]",
  };
  const symbol = children.length === 1 && /[⌘⌥⌃⇧]/.test(children);
  return (
    <kbd
      className={`inline-flex items-center justify-center font-sans font-medium text-fg ${sizes[size]} ${symbol ? (size === "lg" ? "text-[19px]" : "") : ""}`}
      style={{
        background: "var(--kbd-bg)",
        border: "1px solid var(--kbd-border)",
        boxShadow: "0 1px 0 var(--kbd-shadow), inset 0 -1px 0 var(--kbd-border)",
      }}
    >
      {children}
    </kbd>
  );
}

export function HotkeyKeys({
  accelerator,
  platform,
  size = "md",
}: {
  accelerator: string;
  platform: Platform;
  size?: "sm" | "md" | "lg";
}) {
  const parts = hotkeyParts(accelerator, platform);
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

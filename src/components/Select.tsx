// SPDX-License-Identifier: AGPL-3.0-or-later
import type { SelectHTMLAttributes } from "react";
import { IconChevronDown } from "./Icons";

export function Select({ className = "", children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <div className={`relative inline-flex ${className}`}>
      <select
        className="h-8 w-full appearance-none rounded-lg border border-line-strong bg-surface py-0 pr-8 pl-2.5 text-[13px] text-fg shadow-soft transition-[border-color,box-shadow] focus:border-accent focus:shadow-[0_0_0_3px_var(--accent-soft)]"
        {...rest}
      >
        {children}
      </select>
      <IconChevronDown size={14} className="pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2 text-subtle" />
    </div>
  );
}

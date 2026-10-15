// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode } from "react";

export function Card({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div className={`rounded-xl border border-line bg-surface shadow-soft ${className}`}>{children}</div>
  );
}

/** Groupe de lignes de réglage, séparées par des filets. */
export function Group({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div className={`divide-y divide-line overflow-hidden rounded-xl border border-line bg-surface shadow-soft ${className}`}>
      {children}
    </div>
  );
}

export function Row({
  label,
  description,
  htmlFor,
  children,
  align = "center",
}: {
  label: ReactNode;
  description?: ReactNode;
  htmlFor?: string;
  children?: ReactNode;
  align?: "center" | "start";
}) {
  return (
    <div className={`flex gap-6 px-4 py-3.5 ${align === "center" ? "items-center" : "items-start"}`}>
      <div className="min-w-0 flex-1">
        <label htmlFor={htmlFor} className="block text-[13px] font-medium text-fg">
          {label}
        </label>
        {description && <div className="mt-0.5 text-[12.5px] leading-relaxed text-muted">{description}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

export function Section({
  id,
  title,
  description,
  icon,
  children,
}: {
  id?: string;
  title: string;
  description?: string;
  icon?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section id={id} className="scroll-mt-6" aria-labelledby={id ? `${id}-title` : undefined}>
      <div className="mb-2.5 flex items-center gap-2 px-1">
        {icon && <span className="text-subtle">{icon}</span>}
        <h2 id={id ? `${id}-title` : undefined} className="text-[13px] font-semibold text-fg">
          {title}
        </h2>
      </div>
      {description && <p className="-mt-1.5 mb-3 px-1 text-[12.5px] text-muted">{description}</p>}
      {children}
    </section>
  );
}

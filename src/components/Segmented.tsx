import type { ReactNode } from "react";

interface Option<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
}

interface Props<T extends string> {
  value: T;
  options: Option<T>[];
  onChange: (v: T) => void;
  label: string;
  className?: string;
}

export function Segmented<T extends string>({ value, options, onChange, label, className = "" }: Props<T>) {
  const index = Math.max(0, options.findIndex((o) => o.value === value));
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={`relative inline-grid rounded-[9px] bg-surface-3/70 p-[3px] ${className}`}
      style={{ gridTemplateColumns: `repeat(${options.length}, minmax(0, 1fr))` }}
      onKeyDown={(e) => {
        if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
        e.preventDefault();
        const next = (index + (e.key === "ArrowRight" ? 1 : options.length - 1)) % options.length;
        onChange(options[next].value);
        const btns = e.currentTarget.querySelectorAll<HTMLButtonElement>("button");
        btns[next]?.focus();
      }}
    >
      <span
        aria-hidden
        className="absolute top-[3px] bottom-[3px] left-[3px] rounded-[7px] bg-surface shadow-[0_1px_2px_rgba(0,0,0,0.08),0_0_0_0.5px_var(--line-strong)] transition-transform duration-200 ease-[cubic-bezier(0.3,0.7,0.4,1)]"
        style={{
          width: `calc((100% - 6px) / ${options.length})`,
          transform: `translateX(${index * 100}%)`,
        }}
      />
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={active}
            tabIndex={active ? 0 : -1}
            onClick={() => onChange(o.value)}
            className={`relative z-10 inline-flex h-7 items-center justify-center gap-1.5 rounded-[7px] px-3.5 text-[12.5px] font-medium transition-colors ${
              active ? "text-fg" : "text-muted hover:text-fg"
            }`}
          >
            {o.icon}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

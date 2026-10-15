// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { IconLoader } from "./Icons";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md" | "lg";

interface Props extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  loading?: boolean;
  icon?: ReactNode;
}

const variants: Record<Variant, string> = {
  primary:
    "bg-accent text-accent-fg hover:bg-accent-hover shadow-soft disabled:opacity-50",
  secondary:
    "bg-surface text-fg border border-line-strong hover:bg-surface-2 shadow-soft disabled:opacity-50",
  ghost: "text-muted hover:text-fg hover:bg-surface-2 disabled:opacity-40",
  danger: "bg-danger-soft text-danger hover:bg-danger hover:text-white disabled:opacity-50",
};

const sizes: Record<Size, string> = {
  sm: "h-7 px-2.5 text-[12.5px] gap-1.5 rounded-[7px]",
  md: "h-8 px-3 text-[13px] gap-2 rounded-lg",
  lg: "h-10 px-4.5 text-[14px] gap-2 rounded-[10px]",
};

export function Button({
  variant = "secondary",
  size = "md",
  loading = false,
  icon,
  className = "",
  children,
  disabled,
  ...rest
}: Props) {
  return (
    <button
      type="button"
      className={`inline-flex shrink-0 items-center justify-center font-medium whitespace-nowrap transition-[background-color,color,box-shadow,opacity] duration-150 select-none active:translate-y-px disabled:pointer-events-none ${variants[variant]} ${sizes[size]} ${className}`}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...rest}
    >
      {loading ? <IconLoader size={size === "lg" ? 16 : 14} /> : icon}
      {children}
    </button>
  );
}

export function IconButton({
  label,
  className = "",
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={`inline-flex size-7 items-center justify-center rounded-[7px] text-subtle transition-colors hover:bg-surface-3 hover:text-fg ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

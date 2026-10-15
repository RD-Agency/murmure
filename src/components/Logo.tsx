// SPDX-License-Identifier: AGPL-3.0-or-later
export function Logo({ size = 28 }: { size?: number }) {
  return (
    <span
      className="inline-flex shrink-0 items-center justify-center rounded-[28%] bg-gradient-to-b from-[#6d6de0] to-[#4b4bc4] text-white shadow-[0_1px_2px_rgba(40,40,120,0.3),inset_0_1px_0_rgba(255,255,255,0.18)]"
      style={{ width: size, height: size }}
      aria-hidden
    >
      <svg width={size * 0.58} height={size * 0.58} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.6} strokeLinecap="round">
        <path d="M5 10v4M9.5 6.5v11M14 4v16M18.5 8.5v7" />
      </svg>
    </span>
  );
}

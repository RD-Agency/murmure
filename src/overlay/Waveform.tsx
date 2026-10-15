// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef } from "react";

const BARS = 20;

/** Forme d'onde réactive : barres arrondies, enveloppe en cloche, lissage attaque/relâchement. */
export function Waveform({ levelRef, active }: { levelRef: { current: number }; active: boolean }) {
  const barsRef = useRef<(HTMLSpanElement | null)[]>([]);

  useEffect(() => {
    if (!active) return;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const heights = new Array(BARS).fill(0);
    const seeds = Array.from({ length: BARS }, (_, i) => 0.6 + 0.4 * Math.abs(Math.sin(i * 12.9898)));
    let smooth = 0;
    let raf = 0;
    const start = performance.now();

    const frame = (now: number) => {
      const t = (now - start) / 1000;
      // Courbe perceptive : les niveaux RMS de la voix sont bas.
      const target = Math.min(1, Math.max(0, levelRef.current));
      smooth += (target - smooth) * (target > smooth ? 0.35 : 0.12);
      for (let i = 0; i < BARS; i++) {
        const x = (i - (BARS - 1) / 2) / (BARS / 2);
        const envelope = Math.exp(-x * x * 1.6);
        const wobble = reduced ? 1 : 0.72 + 0.28 * Math.sin(t * (5 + seeds[i] * 4) + i * 1.7);
        const goal = 0.12 + smooth * envelope * seeds[i] * wobble * 0.95;
        heights[i] += (goal - heights[i]) * 0.3;
        const el = barsRef.current[i];
        if (el) el.style.transform = `scaleY(${Math.min(1, heights[i]).toFixed(3)})`;
      }
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, [active, levelRef]);

  return (
    <div className="flex h-[22px] items-center gap-[2.5px]" aria-hidden>
      {Array.from({ length: BARS }, (_, i) => (
        <span
          key={i}
          ref={(el) => {
            barsRef.current[i] = el;
          }}
          className="h-full w-[3px] origin-center rounded-full bg-white/90"
          style={{ transform: "scaleY(0.12)" }}
        />
      ))}
    </div>
  );
}

import { StrictMode, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { isMock, on } from "./lib/api";
import type { DictationStatePayload } from "./lib/types";
import { Pill } from "./overlay/Pill";
import "./styles.css";

// Les sons de début/fin sont joués côté Rust : l'overlay ne fait qu'afficher.

type Demo = "recording" | "handsfree" | "transcribing" | "error" | "cycle";

function demoParam(): Demo | null {
  if (!isMock) return null;
  const v = new URLSearchParams(location.search).get("demo");
  return v === "recording" || v === "handsfree" || v === "transcribing" || v === "error" || v === "cycle" ? v : null;
}

const DEMO_STATES: Record<Exclude<Demo, "cycle">, DictationStatePayload> = {
  recording: { state: "recording" },
  handsfree: { state: "recording", hands_free: true },
  transcribing: { state: "transcribing" },
  error: { state: "error", message: "Micro introuvable" },
};

function Overlay() {
  const demo = demoParam();
  const [status, setStatus] = useState<DictationStatePayload>(
    demo && demo !== "cycle" ? DEMO_STATES[demo] : { state: "idle" },
  );
  const levelRef = useRef(0);

  useEffect(() => {
    const offState = on("dictation-state", setStatus);
    const offLevel = on("audio-level", (l) => {
      levelRef.current = l;
    });
    return () => {
      offState();
      offLevel();
    };
  }, []);

  // Démo : niveau audio synthétique et enchaînement des états.
  useEffect(() => {
    if (!demo) return;
    let t = 0;
    const lv = setInterval(() => {
      t++;
      const syl = Math.max(0, Math.sin(t / 2.3)) * Math.max(0.15, Math.sin(t / 11));
      levelRef.current = 0.04 + syl * (0.45 + Math.random() * 0.35);
    }, 33);
    let cyc: ReturnType<typeof setTimeout> | undefined;
    if (demo === "cycle") {
      const seq: [DictationStatePayload, number][] = [
        [{ state: "recording" }, 3200],
        [{ state: "recording", hands_free: true }, 2600],
        [{ state: "transcribing" }, 1600],
        [{ state: "idle" }, 900],
        [{ state: "error", message: "Texte copié dans le presse-papiers" }, 2200],
        [{ state: "idle" }, 900],
      ];
      let i = 0;
      const step = () => {
        setStatus(seq[i][0]);
        cyc = setTimeout(step, seq[i][1]);
        i = (i + 1) % seq.length;
      };
      step();
    }
    return () => {
      clearInterval(lv);
      if (cyc) clearTimeout(cyc);
    };
  }, [demo]);

  return <Pill status={status} levelRef={levelRef} />;
}

if (demoParam()) document.documentElement.classList.add("overlay-demo");

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <Overlay />
  </StrictMode>,
);

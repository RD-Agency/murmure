import { useEffect, useRef, useState } from "react";
import { eventToAccelerator, liveModifiers } from "../lib/format";
import type { Platform } from "../lib/types";
import { HotkeyKeys, Kbd } from "./Kbd";

interface Props {
  value: string;
  platform: Platform;
  onChange: (accel: string) => void;
}

/** Clic → « Appuyez sur la combinaison… » → capture keydown → accelerator Tauri. */
export function HotkeyRecorder({ value, platform, onChange }: Props) {
  const [recording, setRecording] = useState(false);
  const [mods, setMods] = useState<string[]>([]);
  const [hint, setHint] = useState<string | null>(null);
  const ref = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.type === "keyup") {
        setMods(liveModifiers(e, platform));
        return;
      }
      if (e.key === "Escape" && !e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
        setRecording(false);
        return;
      }
      setMods(liveModifiers(e, platform));
      if (["Shift", "Control", "Alt", "Meta", "OS", "AltGraph"].includes(e.key)) return;
      const accel = eventToAccelerator(e, platform);
      if (!accel) {
        setHint("Ajoutez au moins une touche de modification (⌥, ⌘, ⌃ ou ⇧).");
        return;
      }
      onChange(accel);
      setRecording(false);
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKey, true);
    };
  }, [recording, platform, onChange]);

  const start = () => {
    setMods([]);
    setHint(null);
    setRecording(true);
  };

  const pcHint = "Ajoutez au moins une touche de modification (Ctrl, Alt ou Maj).";

  return (
    <div className="flex flex-col items-end gap-1.5" data-hotkey-recorder>
      <button
        ref={ref}
        type="button"
        onClick={() => (recording ? setRecording(false) : start())}
        onBlur={() => setRecording(false)}
        aria-label={recording ? "Enregistrement du raccourci, appuyez sur la combinaison" : "Modifier le raccourci"}
        className={`group inline-flex h-10 min-w-[176px] items-center justify-center gap-2 rounded-[10px] border px-3 transition-[border-color,box-shadow,background-color] ${
          recording
            ? "border-accent bg-accent-soft shadow-[0_0_0_3px_var(--accent-soft)]"
            : "border-line-strong bg-surface-2/60 hover:border-subtle/60 hover:bg-surface-2"
        }`}
      >
        {recording ? (
          mods.length ? (
            <span className="inline-flex items-center gap-1">
              {mods.map((m) => (
                <Kbd key={m} size="sm">
                  {m}
                </Kbd>
              ))}
              <span className="ml-1 text-[12.5px] text-accent">…</span>
            </span>
          ) : (
            <span className="animate-[pulse-soft_1.6s_ease-in-out_infinite] text-[12.5px] font-medium text-accent">
              Appuyez sur la combinaison…
            </span>
          )
        ) : (
          <HotkeyKeys accelerator={value} platform={platform} size="sm" />
        )}
      </button>
      <span className="h-4 text-[11.5px] text-subtle">
        {recording ? (hint ? (platform === "macos" ? hint : pcHint) : "Échap pour annuler") : "Cliquez pour modifier"}
      </span>
    </div>
  );
}

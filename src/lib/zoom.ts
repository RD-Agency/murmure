// SPDX-License-Identifier: AGPL-3.0-or-later
// Zoom de l'interface : Ctrl + molette, Ctrl + « + » / « − » / « 0 » (Cmd sur macOS).
// La valeur est enregistrée dans les réglages (`ui_zoom`, en %) et réappliquée au lancement.

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useRef } from "react";
import { useApp } from "../store";
import { isTauri } from "./api";

export const ZOOM_MIN = 80;
export const ZOOM_MAX = 200;
export const ZOOM_DEFAULT = 110;
const STEP = 10;

export function clampZoom(z: number): number {
  if (!Number.isFinite(z)) return ZOOM_DEFAULT;
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(z / 5) * 5));
}

async function applyZoom(percent: number) {
  const z = clampZoom(percent) / 100;
  if (isTauri) {
    try {
      await getCurrentWebview().setZoom(z);
      return;
    } catch (e) {
      console.warn("zoom webview indisponible, repli CSS", e);
    }
  }
  (document.documentElement.style as CSSStyleDeclaration & { zoom: string }).zoom = String(z);
}

/** Applique le zoom enregistré et branche les raccourcis. À appeler une fois (fenêtre principale). */
export function useUiZoom() {
  const zoom = useApp((s) => s.settings?.ui_zoom);
  const updateSettings = useApp((s) => s.updateSettings);
  const current = useRef(ZOOM_DEFAULT);

  useEffect(() => {
    if (zoom === undefined) return;
    current.current = clampZoom(zoom);
    void applyZoom(current.current);
  }, [zoom]);

  useEffect(() => {
    const set = (z: number) => {
      const next = clampZoom(z);
      if (next === current.current) return;
      current.current = next;
      void applyZoom(next);
      updateSettings({ ui_zoom: next }, 400);
    };
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      set(current.current + (e.deltaY < 0 ? STEP : -STEP));
    };
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      if (e.key === "+" || e.key === "=" || e.code === "NumpadAdd") set(current.current + STEP);
      else if (e.key === "-" || e.code === "NumpadSubtract") set(current.current - STEP);
      else if (e.key === "0" || e.code === "Numpad0") set(ZOOM_DEFAULT);
      else return;
      e.preventDefault();
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("wheel", onWheel);
      window.removeEventListener("keydown", onKey);
    };
  }, [updateSettings]);
}

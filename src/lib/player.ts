// SPDX-License-Identifier: AGPL-3.0-or-later
// Écoute de l'audio d'une dictée : une seule à la fois, WAV décodé par Rust.
import { create } from "zustand";
import { api, errorMessage } from "./api";

interface PlayerState {
  playingId: string | null;
  loadingId: string | null;
  error: string | null;
  toggle: (id: string) => Promise<void>;
  stop: () => void;
}

let audio: HTMLAudioElement | null = null;
let url: string | null = null;

function release() {
  audio?.pause();
  audio = null;
  if (url) URL.revokeObjectURL(url);
  url = null;
}

export const usePlayer = create<PlayerState>((set, get) => ({
  playingId: null,
  loadingId: null,
  error: null,

  toggle: async (id) => {
    if (get().playingId === id || get().loadingId === id) {
      get().stop();
      return;
    }
    release();
    set({ playingId: null, loadingId: id, error: null });
    try {
      const wav = await api.getDictationAudio(id);
      if (get().loadingId !== id) return;
      url = URL.createObjectURL(new Blob([wav], { type: "audio/wav" }));
      audio = new Audio(url);
      audio.onended = () => get().stop();
      await audio.play();
      set({ playingId: id, loadingId: null });
    } catch (e) {
      release();
      set({ playingId: null, loadingId: null, error: errorMessage(e) });
    }
  },

  stop: () => {
    release();
    set({ playingId: null, loadingId: null });
  },
}));

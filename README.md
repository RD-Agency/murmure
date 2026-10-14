# Murmure

Dictée vocale pour macOS, Linux et Windows (Tauri v2 + React). Voir `ARCHITECTURE.md`.

- `pnpm tauri dev` : application complète.
- `pnpm dev` : front seul dans un navigateur (http://localhost:1420), en **mode démo** (données factices).
  - `?reset` réinitialise les données de démo (relance l'onboarding).
  - `overlay.html?demo=recording|handsfree|transcribing|error` prévisualise la pastille.

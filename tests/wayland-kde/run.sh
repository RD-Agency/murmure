#!/bin/bash
# Test d'intégration Wayland KDE (portail GlobalShortcuts) dans Docker, x86_64.
#   tests/wayland-kde/run.sh
# 1. compile Murmure pour Linux (conteneur Debian, cache dans des volumes Docker) ;
# 2. lance KWin Wayland headless + xdg-desktop-portal-kde (Fedora 44) et Murmure ;
# 3. vérifie : enregistrement de l'app id, BindShortcuts, raccourci relu, Activated → démarrage.
# Sur Mac Apple Silicon, l'émulation x86_64 rend la première compilation longue (≈ 30 min).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OUT="${OUT:-$(mktemp -d)}"
PLATFORM=linux/amd64

docker build --platform $PLATFORM -q -t murmure-build -f "$HERE/Dockerfile.build" "$HERE" > /dev/null
docker build --platform $PLATFORM -q -t murmure-kde-wayland -f "$HERE/Dockerfile" "$HERE" > /dev/null

[ -d "$ROOT/dist" ] || (cd "$ROOT" && pnpm install --frozen-lockfile && pnpm build)
docker run --rm --platform $PLATFORM \
  -v "$ROOT":/src -v murmure-cargo-registry:/usr/local/cargo/registry -v murmure-target:/target \
  -v "$OUT":/out -e CARGO_TARGET_DIR=/target -w /src/src-tauri murmure-build \
  bash -c 'cargo build --features tauri/custom-protocol && cp /target/debug/murmure /out/murmure'

cp "$HERE/session.sh" "$OUT/session.sh"
echo "Journaux : $OUT"
docker run --rm --platform $PLATFORM --cap-add SYS_NICE --security-opt seccomp=unconfined \
  -v "$OUT":/out murmure-kde-wayland dbus-run-session -- /out/session.sh

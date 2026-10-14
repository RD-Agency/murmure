# Murmure

Dictée vocale façon Wispr Flow, pour macOS, Linux et Windows. On maintient un raccourci, on parle,
on relâche : le texte arrive là où se trouve le curseur, dans n'importe quelle application.

Deux moteurs au choix :

- **Sur cet appareil** : Whisper large-v3-turbo via whisper.cpp. Hors ligne, rien ne sort de la machine.
  Accélération Metal sur Mac, CUDA ou Vulkan en option ailleurs.
- **Serveur** : n'importe quel endpoint compatible OpenAI (`/v1/audio/transcriptions`) avec une clé API :
  notre serveur auto-hébergé (`server/`), OpenAI ou Groq.

## Utilisation

| Geste | Effet |
|---|---|
| Maintenir **⌥ Espace** (Mac) / **Ctrl+Alt+Espace** (Linux, Windows) | Push-to-talk : on parle, on relâche, c'est collé |
| Appui bref | Mains libres ; un second appui arrête |
| **Échap** pendant l'enregistrement | Annule |

Le raccourci, le mode (hybride, maintien, bascule), la langue, le vocabulaire, le micro et le collage se règlent dans l'app.
La clé API est rangée dans le trousseau du système, jamais dans un fichier.

## Développement

Prérequis : Rust stable, Node 22+, pnpm, CMake. Linux : `libwebkit2gtk-4.1-dev libasound2-dev libxdo-dev clang`.

```bash
pnpm install
pnpm tauri dev          # application complète
pnpm dev                # front seul dans le navigateur, en mode démo (données factices)
pnpm tauri build        # installeurs (.dmg, .deb/.rpm/AppImage, .msi)
pnpm tauri build -- --features cuda   # Linux/Windows avec GPU NVIDIA
```

Tests :

```bash
cd src-tauri && cargo test --lib
# Moteur local sur un vrai modèle (voir ~/.whisper/models ou le dossier de modèles de l'app) :
MURMURE_TEST_WAV=t.wav cargo test --release --lib engine::local -- --ignored --nocapture
# Client distant contre un serveur réel :
MURMURE_E2E_URL=http://127.0.0.1:8000 MURMURE_E2E_KEY=mrm_… MURMURE_TEST_WAV=t.wav \
  cargo test --lib engine::remote -- --ignored --nocapture
cd server && uv run pytest
```

Mode démo du front : `?reset` relance l'onboarding, `?theme=light`, `?platform=linux`,
`overlay.html?demo=recording|handsfree|transcribing|error|cycle` pour la pastille.

## Téléchargement et release

Page publique : **https://crgenerator.s3.eu-west-par.io.cloud.ovh.net/murmure/index.html**

Liens stables vers la dernière version : `…/murmure/latest/Murmure-macos-arm64.dmg`, `Murmure-macos-x64.dmg`,
`Murmure-windows-x64-setup.exe`, `Murmure-linux-x64.AppImage` / `.deb` / `.rpm`, `Murmure-linux-x64-cuda.deb`.

Publier une version : `git tag v0.2.0 && git push --tags` (ou Actions › Release › Run workflow). La CI construit
macOS (Apple Silicon et Intel, signé et notarisé), Windows, Linux (et une variante CUDA), publie sur S3 et dans une
release GitHub brouillon, régénère la page et `latest.json` : les apps installées proposent alors la mise à jour.

Secrets du repo : `APPLE_*` (signature et notarisation), `OVH_S3_ACCESS_KEY` / `OVH_S3_SECRET_KEY`,
`TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)` (clé des mises à jour, sauvegardée dans `~/.murmure-signing/`).

## Modèle local

Le modèle s'installe tout seul en arrière-plan au premier lancement (environ 550 Mo, vérifié par SHA256,
reprise automatique après une coupure). Pour l'installer à l'avance sur un parc de postes, sans fenêtre :

```bash
murmure --install-model                     # modèle des réglages (par défaut large-v3-turbo-q5_0)
murmure --install-model small-q5_1          # modèle précis
MURMURE_DATA_DIR=/chemin murmure --install-model   # dossier de données imposé (tests, CI)
```

Sous Windows, l'exécutable de release n'a pas de console : la progression ne s'affiche pas, seul le code
de sortie fait foi.

## Permissions

- **macOS** : Micro, et Accessibilité pour coller dans les autres apps. L'onboarding ouvre les bons panneaux.
- **Linux X11** : rien de spécial (`xdotool` conseillé).
- **Linux Wayland** : le collage passe par `wtype` ou `ydotool`. Le raccourci global n'est pas capté sous Wayland :
  liez un raccourci du bureau (KDE : Réglages › Raccourcis) à la commande `murmure --toggle`.

## Serveur

Voir [`server/README.md`](server/README.md) : FastAPI + faster-whisper large-v3-turbo, Docker GPU/CPU, Caddy en HTTPS,
clés `mrm_…` générées par `murmure-server gen-key`.

Architecture et contrat interne : [`ARCHITECTURE.md`](ARCHITECTURE.md).

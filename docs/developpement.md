# Développement

## Prérequis

Pour tous les systèmes : Rust stable, Node.js 22 ou plus récent, pnpm 11 et CMake. Le serveur demande en
plus Python 3.11 à 3.13 et [uv](https://docs.astral.sh/uv/).

- **macOS** : les outils en ligne de commande de Xcode (`xcode-select --install`).
- **Windows** : Visual Studio Build Tools avec la charge de travail C++, et WebView2 (présent d'office sur
  Windows 11).
- **Linux** (Ubuntu 22.04 ou équivalent) :

  ```bash
  sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf \
    libasound2-dev libxdo-dev libssl-dev cmake clang
  ```

## Lancer l'application

```bash
pnpm install
pnpm tauri dev          # application complète, rechargement à chaud du front
pnpm dev                # front seul dans le navigateur, en mode démo, sur http://localhost:1420
```

Le mode démo remplace les appels à Rust par des données factices (`src/lib/mock.ts`). Quelques paramètres
d'URL utiles :

| Paramètre | Effet |
|---|---|
| `?reset` | relance l'assistant du premier lancement |
| `?theme=light` | force le thème clair (sinon, celui du système) |
| `?platform=linux` | simule Linux (ou `macos`, `windows`) |
| `?wayland`, `?wayland=deny` | simule une session Wayland, ou un refus du portail |
| `?update`, `?update=ready`, `?update=error` | simule une mise à jour disponible, prête, ou un serveur injoignable |
| `overlay.html?demo=cycle` | pastille seule ; aussi `recording`, `handsfree`, `transcribing`, `installing`, `error` |

## Compiler les installeurs

```bash
pnpm tauri build                            # .dmg, .deb, .rpm, .AppImage ou .msi selon le système
pnpm tauri build -- --features cuda         # GPU NVIDIA (Linux, Windows) : CUDA Toolkit requis
pnpm tauri build -- --features vulkan       # GPU NVIDIA, AMD, Intel : libvulkan-dev et glslc requis
```

Les fonctionnalités `cuda` et `vulkan` s'excluent. Ubuntu 22.04 ne fournit pas `glslc` : la CI prend
`glslc` et les en-têtes Vulkan dans le SDK LunarG et définit `VULKAN_SDK` (voir le job « Vulkan » de
`.github/workflows/ci.yml`).

Pour un binaire portable, comme en release, définissez `SOURCE_DATE_EPOCH` : ggml désactive alors
`-march=native` et vise une base AVX2, FMA et F16C sur x86.

En local, sans la clé de signature des mises à jour, désactivez les artefacts de mise à jour :

```bash
pnpm tauri build --bundles deb,rpm --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

## Tests

```bash
cd src-tauri
cargo test --lib                                   # tests unitaires, sans matériel ni modèle
cargo clippy --all-targets -- -D warnings
```

Tests ignorés par défaut, à lancer à la main :

```bash
# Moteur local sur un vrai modèle (large-v3-turbo par défaut, ou MURMURE_TEST_MODEL=/chemin/ggml-base.bin)
MURMURE_TEST_WAV=essai.wav cargo test --release --lib engine::local -- --ignored --nocapture

# Client « Serveur » contre un serveur réel
MURMURE_E2E_URL=http://127.0.0.1:8000 MURMURE_E2E_KEY=mrm_… MURMURE_TEST_WAV=essai.wav \
  cargo test --lib engine::remote -- --ignored --nocapture

# Ponctuation dictée sur un corpus de fichiers WAV accompagnés du texte attendu
MURMURE_PUNCT_DIR=/chemin/corpus cargo test --release --lib engine::local::tests::corpus_ponctuation -- --ignored --nocapture
```

Le serveur a ses propres tests, avec un moteur factice qui évite de télécharger un modèle :

```bash
cd server
uv sync
uv run pytest
```

Le dossier `tests/wayland-kde/` contient un test d'intégration du portail des raccourcis dans une session
KDE Wayland sans écran (KWin virtuel et xdg-desktop-portal-kde), exécuté dans Docker sur une machine Linux.

## Intégration continue

`.github/workflows/ci.yml` s'exécute sur chaque push vers `main` et sur chaque pull request : front et tests
Rust sur macOS, Linux et Windows, variante Vulkan, tests du serveur, et paquets Linux de test pour les pull
requests. Il n'utilise que des machines hébergées par GitHub et un jeton en lecture seule.

Les builds CUDA tournent sur un runner auto-hébergé équipé d'une carte NVIDIA. Ce runner n'est utilisé que
par la release (sur tag) et par le workflow manuel `tour.yml`. Il n'exécute jamais le code d'une pull
request.

## Publier une version

1. Mettre à jour la version dans `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` et
   `src-tauri/Cargo.lock` (`cargo update -p murmure --offline` dans `src-tauri/`).
2. Compléter `CHANGELOG.md` et les nouveautés de `.github/release-notes.md`.
3. Committer `chore: release X.Y.Z`, puis pousser le tag :

   ```bash
   git tag vX.Y.Z && git push origin main vX.Y.Z
   ```

Le workflow `release.yml` construit alors macOS (Apple Silicon et Intel, signé et notarisé), Windows et les
trois variantes Linux. Il publie les fichiers sur le stockage S3 et dans une release GitHub en brouillon,
puis régénère la page de téléchargement, les liens `latest/` et les manifestes de mise à jour
(`latest.json`, `cuda/latest.json`, `vulkan/latest.json`). Les applications installées se mettent à jour
dans l'heure. La release GitHub reste en brouillon jusqu'à sa publication manuelle.

Secrets du dépôt utilisés par la release :

| Secret | Usage |
|---|---|
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | signature macOS |
| `APPLE_API_ISSUER`, `APPLE_API_KEY_ID`, `APPLE_API_KEY_P8` | notarisation macOS |
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | signature des mises à jour |
| `OVH_S3_ACCESS_KEY`, `OVH_S3_SECRET_KEY` | publication sur le stockage S3 |

La variable de dépôt `CUDA_RUNNER` (`["self-hosted","murmure-cuda"]`) envoie le build CUDA sur le runner
auto-hébergé ; sans elle, il tourne sur une machine Ubuntu de GitHub.

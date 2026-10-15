# Murmure

Dictée vocale façon Wispr Flow, pour macOS, Linux et Windows. On maintient un raccourci, on parle,
on relâche : le texte arrive là où se trouve le curseur, dans n'importe quelle application.

Deux moteurs au choix :

- **Sur cet appareil** : Whisper large-v3-turbo via whisper.cpp. Hors ligne, rien ne sort de la machine.
  Accélération Metal sur Mac ; sous Linux, variantes NVIDIA (CUDA) et Vulkan (NVIDIA, AMD, Intel),
  repli automatique sur le processeur sans GPU utilisable.
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
pnpm tauri build -- --features cuda     # Linux/Windows avec GPU NVIDIA (CUDA Toolkit requis)
pnpm tauri build -- --features vulkan   # GPU NVIDIA/AMD/Intel via Vulkan (Linux : libvulkan-dev + glslc)
```

`cuda` et `vulkan` s'excluent. Sous Ubuntu 22.04, `glslc` n'est pas packagé : la CI prend `glslc` et les en-têtes
dans le SDK LunarG (`VULKAN_SDK`). L'accélération retenue s'affiche dans l'app (« Accélération : Vulkan · … ») et
dans le journal (`Accélération : Vulkan (NVIDIA GeForce RTX 5050)`).

Tests :

```bash
cd src-tauri && cargo test --lib
# Moteur local sur un vrai modèle (voir ~/.whisper/models ou le dossier de modèles de l'app) :
MURMURE_TEST_WAV=t.wav cargo test --release --lib engine::local -- --ignored --nocapture
# (MURMURE_TEST_MODEL=/chemin/ggml-base.bin pour un autre modèle que large-v3-turbo)
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
`Murmure-windows-x64-setup.exe`, `Murmure-linux-x64.AppImage` / `.deb` / `.rpm`, `Murmure-linux-x64-cuda.deb` / `.rpm`,
`Murmure-linux-x64-vulkan.deb` / `.rpm`.

Publier une version : `git tag v0.2.0 && git push --tags` (ou Actions › Release › Run workflow). La CI construit
macOS (Apple Silicon et Intel, signé et notarisé), Windows, Linux (et des variantes CUDA et Vulkan), publie sur S3 et dans une
release GitHub brouillon, régénère la page et `latest.json` (plus `cuda/latest.json` et `vulkan/latest.json`, pour
qu'une installation GPU reste sur sa variante) : les apps installées proposent alors la mise à jour.

### Variante Linux Vulkan

Accélération GPU par Vulkan : NVIDIA, AMD et Intel, sans CUDA ni bibliothèque embarquée (paquet léger). Seuls
`libvulkan.so.1` et le pilote ICD du GPU sont requis, présents d'office avec les pilotes NVIDIA et Mesa. Dépendances
des paquets : `libvulkan1` (.deb), `libvulkan.so.1()(64bit)` (.rpm, satisfaite par `libvulkan1` sur openSUSE et
`vulkan-loader` sur Fedora). Sans périphérique Vulkan (pas de pilote, ou seulement le pilote logiciel lavapipe que
whisper.cpp ignore), la transcription se fait sur le processeur. C'est aussi la solution de repli si la variante
CUDA ne démarre pas. `GGML_VK_VISIBLE_DEVICES=1` choisit un autre GPU (index `vulkaninfo`).

Secrets du repo : `APPLE_*` (signature et notarisation), `OVH_S3_ACCESS_KEY` / `OVH_S3_SECRET_KEY`,
`TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)` (clé des mises à jour, sauvegardée dans `~/.murmure-signing/`).

## Distributions Linux supportées

Plancher : **glibc ≥ 2.34 et webkit2gtk-4.1** (Tauri v2). Les binaires sont construits sur Ubuntu 22.04.

| Famille | Versions testées (installation, lancement, mode CLI) | Paquet |
|---|---|---|
| openSUSE | Leap 15.6, Leap 16.0, Tumbleweed | `.rpm` (`zypper install --allow-unsigned-rpm`) |
| Fedora | 43, 44 | `.rpm` (`dnf install`) |
| Debian / Ubuntu / Mint | Debian 12, 13 · Ubuntu 22.04, 24.04 · Mint 22 | `.deb` (`apt install ./…`) |
| Toutes | idem | `.AppImage` (variante standard seulement) |

Exclues faute de webkit2gtk-4.1 ou de glibc 2.34 : Debian 11, Ubuntu 20.04, openSUSE Leap ≤ 15.5, RHEL/Alma 9.

Variante NVIDIA (`-cuda`) : CUDA 12 (cudart, cuBLAS) est livré dans `/usr/lib/murmure/cuda` ; seul le pilote NVIDIA
propriétaire est requis (≥ 525, CUDA 12 ou 13), cartes Turing (RTX 20) à Blackwell (RTX 50).

Son : la capture passe par ALSA. Sous PulseAudio ou PipeWire, le périphérique « default » doit être routé par
`alsa-plugins-pulse` (openSUSE), `alsa-plugins-pulseaudio` / `pipewire-alsa` (Fedora) ou `libasound2-plugins` /
`pipewire-alsa` (Debian) : le paquet les recommande. Si la source par défaut du système est le moniteur d'une
sortie (HDMI…), l'app le signale ; corriger dans les réglages audio de KDE ou avec `pactl set-default-source`.

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
- **Linux Wayland** (KDE Plasma ≥ 5.27, GNOME ≥ 48) : Murmure passe par les portails XDG. Au premier lancement
  (étape « Autoriser Murmure » ou Réglages › Raccourci), le bureau demande **une seule fois** :
  1. le **raccourci global** (portail GlobalShortcuts) — on peut y choisir une autre touche ; le raccourci attribué
     s'affiche dans Réglages › Raccourci, « Modifier… » rouvre le réglage du bureau ;
  2. le **contrôle du clavier** (portail RemoteDesktop) — uniquement pour taper Ctrl+V ; l'accord est mémorisé.

  Collage : portail → `ydotool` (si `ydotoold` tourne) → `wtype` → à défaut le texte est copié et une notification
  invite à coller avec Ctrl+V. Si le portail des raccourcis est absent ou refusé : liez un raccourci KDE
  (Configuration du système › Clavier › Raccourcis › Ajouter nouveau › Commande ou script) à `murmure --toggle`.
  Échap n'annule pas une dictée sous Wayland (pas de capture globale).

  Repli sans portail (bureau trop ancien, autorisation refusée) : KDE (KWin) ne gère pas le protocole de `wtype`,
  utilisez `ydotool` (accès à `/dev/uinput` + démon utilisateur) :

  ```bash
  sudo zypper install -y ydotool        # Fedora : sudo dnf install -y ydotool
  echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' | sudo tee /etc/udev/rules.d/80-uinput.rules
  echo uinput | sudo tee /etc/modules-load.d/uinput.conf
  sudo usermod -aG input "$USER"        # puis se déconnecter / reconnecter
  # Démon côté utilisateur (même socket que le client ; le service système de Fedora tourne en root) :
  mkdir -p ~/.config/systemd/user && printf '[Unit]\nDescription=ydotoold\n[Service]\nExecStart=/usr/bin/ydotoold\nRestart=always\n[Install]\nWantedBy=default.target\n' > ~/.config/systemd/user/ydotool.service
  systemctl --user daemon-reload && systemctl --user enable --now ydotool
  ```

  Autre repli : la session « Plasma (X11) » à l'écran de connexion quand elle existe
  (openSUSE Leap 16 / Tumbleweed : `sudo zypper install -y plasma6-session-x11`).

## Serveur

Voir [`server/README.md`](server/README.md) : FastAPI + faster-whisper large-v3-turbo, Docker GPU/CPU, Caddy en HTTPS,
clés `mrm_…` générées par `murmure-server gen-key`.

Architecture et contrat interne : [`ARCHITECTURE.md`](ARCHITECTURE.md).

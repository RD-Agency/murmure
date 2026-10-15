# Murmure sous Linux

## Distributions prises en charge

Murmure repose sur Tauri 2 : il faut glibc 2.34 ou plus récente et webkit2gtk-4.1. Les paquets sont
construits sur Ubuntu 22.04.

| Distribution | Versions testées | Paquet |
|---|---|---|
| openSUSE | Leap 15.6, Leap 16.0, Tumbleweed | `.rpm` |
| Fedora | 43, 44 | `.rpm` |
| Debian, Ubuntu, Linux Mint | Debian 12 et 13, Ubuntu 22.04 et 24.04, Mint 22 | `.deb` |
| Autres | — | `.AppImage` (variante standard seulement) |

Debian 11, Ubuntu 20.04, openSUSE Leap 15.5 et antérieures, RHEL et Alma 9 ne sont pas prises en charge :
il leur manque webkit2gtk-4.1 ou glibc 2.34.

## Installation

La [page de téléchargement](https://crgenerator.s3.eu-west-par.io.cloud.ovh.net/murmure/index.html) donne
la commande exacte pour votre distribution et votre carte graphique. Les liens `latest/` pointent toujours
vers la dernière version.

```bash
# openSUSE (les paquets ne sont pas signés)
curl -fLO https://crgenerator.s3.eu-west-par.io.cloud.ovh.net/murmure/latest/Murmure-linux-x64.rpm
sudo zypper install --allow-unsigned-rpm ./Murmure-linux-x64.rpm

# Fedora
curl -fLO https://crgenerator.s3.eu-west-par.io.cloud.ovh.net/murmure/latest/Murmure-linux-x64.rpm
sudo dnf install -y ./Murmure-linux-x64.rpm

# Debian, Ubuntu, Mint
curl -fLO https://crgenerator.s3.eu-west-par.io.cloud.ovh.net/murmure/latest/Murmure-linux-x64.deb
sudo apt install ./Murmure-linux-x64.deb
```

Remplacez `Murmure-linux-x64` par `Murmure-linux-x64-cuda` ou `Murmure-linux-x64-vulkan` pour les
variantes GPU. Une seule variante peut être installée à la fois : le paquet s'appelle `murmure` dans les
trois cas.

L'AppImage demande FUSE 2. S'il ne démarre pas, installez `libfuse2t64` (Ubuntu 24.04 et suivantes),
`libfuse2` (Debian, openSUSE) ou `fuse-libs` (Fedora).

## Choisir une variante

**Standard.** La transcription se fait sur le processeur. Comptez quelques secondes pour une phrase sur
une machine récente.

**NVIDIA.** Les bibliothèques CUDA 12 (`libcudart`, `libcublas`, `libcublasLt`) sont livrées dans
`/usr/lib/murmure/cuda`. Il suffit du pilote propriétaire NVIDIA, version 525 ou plus récente (CUDA 12 ou
13), et d'une carte de génération Turing (RTX 20) à Blackwell (RTX 50). `nvidia-smi` doit reconnaître la
carte.

**Vulkan.** Accélération par la carte graphique pour NVIDIA, AMD et Intel, sans bibliothèque embarquée.
Il faut `libvulkan.so.1` et le pilote Vulkan de la carte, installés d'office avec les pilotes NVIDIA et
Mesa. Vérifiez avec `vulkaninfo --summary` (paquet `vulkan-tools`). Sans périphérique Vulkan, ou avec le
seul pilote logiciel lavapipe, Murmure transcrit sur le processeur. Pour choisir une autre carte, lancez
Murmure avec `GGML_VK_VISIBLE_DEVICES=1` (l'index est celui de `vulkaninfo`).

L'accélération utilisée s'affiche dans Réglages, sous le choix du moteur, par exemple
« Vulkan · NVIDIA GeForce RTX 5060 ».

Chaque variante a son propre canal de mise à jour : une installation NVIDIA ou Vulkan reste sur sa variante.

## Son

Murmure capture le micro par ALSA. Sous PulseAudio ou PipeWire, le périphérique ALSA `default` doit être
redirigé vers le serveur de son. Les paquets recommandent le greffon nécessaire ; s'il manque :

```bash
sudo zypper install alsa-plugins-pulse                  # openSUSE
sudo dnf install pipewire-alsa                          # Fedora
sudo apt install pipewire-alsa                          # Debian, Ubuntu (ou libasound2-plugins)
```

Si la source par défaut du système est le moniteur d'une sortie (HDMI, casque), Murmure le signale.
Choisissez le bon micro dans les réglages audio du bureau, ou en ligne de commande :

```bash
pactl list short sources
pactl set-default-source <nom-du-micro>
```

## Wayland

Sous Wayland, une application ne peut ni capter un raccourci global, ni simuler une frappe dans une autre
fenêtre sans passer par le bureau. Murmure utilise donc les portails XDG, disponibles avec KDE Plasma 5.27
ou plus récent et GNOME 48 ou plus récent.

Au premier lancement, l'assistant affiche le bouton « Autoriser Murmure » (on le retrouve dans
Réglages › Raccourci). Le bureau demande alors, une seule fois :

1. d'accepter le raccourci global, avec la possibilité d'en choisir un autre ;
2. d'autoriser le contrôle du clavier, que Murmure utilise uniquement pour taper Ctrl+V.

Le raccourci attribué s'affiche dans Réglages › Raccourci ; « Modifier… » rouvre le réglage du bureau.
L'accord pour le clavier est mémorisé jusqu'à ce que vous le révoquiez.

Le collage essaie, dans l'ordre : le portail, puis `ydotool` si son démon tourne, puis `wtype`. Si rien
ne fonctionne, le texte est copié et une notification vous invite à le coller avec Ctrl+V.

Sous Wayland, Échap n'annule pas une dictée : Murmure ne capte pas le clavier en dehors du raccourci.

### Sans portail des raccourcis

Si le bureau est trop ancien ou si vous avez refusé la demande, associez un raccourci du bureau à la
commande `murmure --toggle`. Dans KDE : Configuration du système › Clavier › Raccourcis ›
Ajouter nouveau › Commande ou script. Ce mode fonctionne en bascule : un appui démarre la dictée, un second
l'arrête.

### Sans portail de contrôle du clavier

KDE (KWin) ne prend pas en charge le protocole utilisé par `wtype`. Le plus fiable est alors `ydotool`, qui
demande un accès à `/dev/uinput` et un démon lancé par l'utilisateur :

```bash
sudo zypper install -y ydotool        # Fedora : sudo dnf install -y ydotool
echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' | sudo tee /etc/udev/rules.d/80-uinput.rules
echo uinput | sudo tee /etc/modules-load.d/uinput.conf
sudo usermod -aG input "$USER"        # puis fermer la session et se reconnecter

# Démon côté utilisateur : le service système de Fedora tourne en root et n'utilise pas le même socket.
mkdir -p ~/.config/systemd/user
printf '[Unit]\nDescription=ydotoold\n[Service]\nExecStart=/usr/bin/ydotoold\nRestart=always\n[Install]\nWantedBy=default.target\n' \
  > ~/.config/systemd/user/ydotool.service
systemctl --user daemon-reload && systemctl --user enable --now ydotool
```

Autre solution : choisir la session « Plasma (X11) » à l'écran de connexion, quand elle existe
(openSUSE Leap 16 et Tumbleweed : `sudo zypper install -y plasma6-session-x11`).

## Installer le modèle à l'avance

Pour préparer des postes sans ouvrir l'application :

```bash
murmure --install-model                     # modèle choisi dans les réglages (par défaut large-v3-turbo-q5_0)
murmure --install-model small-q5_1          # modèle plus léger
MURMURE_DATA_DIR=/chemin murmure --install-model
```

La commande affiche la progression et renvoie 0 en cas de succès, 1 en cas d'échec, 2 si l'identifiant du
modèle est inconnu. Les modèles sont rangés dans `~/.local/share/fr.rdagency.murmure/`. Un modèle déjà
présent dans `~/.whisper/models` est réutilisé.

## Diagnostic

Ces commandes donnent l'essentiel des informations utiles dans une issue :

```bash
cat /etc/os-release | head -4
echo "session=$XDG_SESSION_TYPE bureau=$XDG_CURRENT_DESKTOP"
rpm -q murmure 2>/dev/null || dpkg -s murmure 2>/dev/null | grep -E '^(Package|Version)'
pactl info | grep -E 'Server Name|Default Source'
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
vulkaninfo --summary | grep -E 'deviceName|driverName'
```

Pour obtenir le journal détaillé, quittez Murmure (icône de la barre des tâches, « Quitter Murmure »), puis
relancez-le depuis un terminal :

```bash
RUST_LOG=debug murmure 2>&1 | tee murmure.log
```

Les fichiers de Murmure se trouvent dans `~/.config/fr.rdagency.murmure/` (réglages) et
`~/.local/share/fr.rdagency.murmure/` (modèles, historique).

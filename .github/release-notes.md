## Nouveautés de la 0.1.5

- **Wayland natif sous Linux** (KDE Plasma 5.27 et plus, GNOME 48 et plus) : le raccourci global et le
  collage passent par les portails XDG du bureau. Au premier lancement, cliquez sur « Autoriser Murmure » ;
  le bureau demande une seule fois d'accepter le raccourci, puis le contrôle du clavier. Sans portail,
  Murmure se replie sur `ydotool`, ou copie le texte et vous invite à le coller.
- **Murmure passe en open source**, sous licence GNU AGPL v3 ou ultérieure. Le code est sur
  https://github.com/RD-Agency/murmure.

Le détail des versions est dans le [CHANGELOG](https://github.com/RD-Agency/murmure/blob/main/CHANGELOG.md).

## Installer Murmure

La page https://crgenerator.s3.eu-west-par.io.cloud.ovh.net/murmure/index.html choisit le bon fichier et
donne la commande d'installation pour Linux.

| Système | Fichier |
|---|---|
| Mac Apple Silicon (M1 à M5) | `Murmure-…-macos-arm64.dmg` |
| Mac Intel | `Murmure-…-macos-x64.dmg` |
| Windows 10 et 11 | `Murmure-…-windows-x64-setup.exe` (ou `.msi`) |
| Linux Debian, Ubuntu, Mint | `Murmure-…-linux-x64.deb` |
| Linux Fedora, openSUSE | `Murmure-…-linux-x64.rpm` |
| Linux, autres distributions | `Murmure-…-linux-x64.AppImage` |
| Linux avec une carte NVIDIA | `Murmure-…-linux-x64-cuda.rpm` ou `.deb` : CUDA est inclus, seul le pilote NVIDIA est requis |
| Linux avec une carte NVIDIA, AMD ou Intel | `Murmure-…-linux-x64-vulkan.rpm` ou `.deb` : paquet léger, seul le pilote graphique est requis ; solution de secours si la variante NVIDIA ne démarre pas |

Au premier lancement, Murmure télécharge le modèle Whisper large-v3-turbo (environ 550 Mo). Ensuite, tout
fonctionne hors ligne. Pour un parc de postes, `murmure --install-model` installe le modèle à l'avance.

**Mac** : application signée et notarisée. Au premier lancement, autorisez le micro et l'Accessibilité.

**Windows** : tant que l'installeur n'est pas signé, SmartScreen peut demander de cliquer sur
« Informations complémentaires », puis « Exécuter quand même ».

**Mises à jour** : automatiques et silencieuses (téléchargées en arrière-plan, appliquées quand l'application
est inactive). Avec les paquets Linux .deb et .rpm, Murmure propose la mise à jour par un bouton.

Empreintes des fichiers : `SHA256SUMS.txt`.

Murmure est distribué sous licence AGPL-3.0-or-later ; la variante NVIDIA bénéficie d'une exception pour
les bibliothèques CUDA (voir `LICENSE-EXCEPTION-CUDA.md` dans le dépôt).

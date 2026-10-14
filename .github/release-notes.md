## Installer Murmure

| Système | Fichier |
|---|---|
| Mac Apple Silicon (M1 à M5) | `Murmure-…-macos-arm64.dmg` |
| Mac Intel | `Murmure-…-macos-x64.dmg` |
| Windows 10/11 | `Murmure-…-windows-x64-setup.exe` (ou `.msi`) |
| Linux Debian / Ubuntu | `Murmure-…-linux-x64.deb` |
| Linux Fedora / openSUSE | `Murmure-…-linux-x64.rpm` |
| Linux, toute distribution | `Murmure-…-linux-x64.AppImage` |
| Linux avec carte NVIDIA | variantes `…-linux-x64-cuda.*` (nécessite les bibliothèques CUDA 12 : `cudart` et `cublas`) |

Au premier lancement, Murmure télécharge seul le modèle Whisper large-v3-turbo (~550 Mo), puis tout fonctionne hors ligne.
Pour un déploiement en parc : `murmure --install-model` installe le modèle avant le premier lancement.

**Mac** : si l'app n'est pas signée, faites clic droit › Ouvrir au premier lancement, puis autorisez le micro et l'Accessibilité.
**Windows** : SmartScreen peut afficher « Informations complémentaires › Exécuter quand même » tant que l'app n'est pas signée.

Vérification : `SHA256SUMS.txt`.

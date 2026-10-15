# Murmure

Murmure est une application de dictée vocale pour macOS, Windows et Linux. Vous maintenez un raccourci
clavier, vous parlez, vous relâchez : le texte est collé là où se trouve le curseur, dans n'importe quelle
application. Le principe est celui de Wispr Flow, mais la reconnaissance vocale tourne sur votre machine et
rien n'en sort.

La transcription repose sur Whisper large-v3-turbo, exécuté localement par whisper.cpp. Pour les postes
trop modestes, Murmure peut aussi envoyer l'audio à un serveur de transcription, y compris celui fourni
dans ce dépôt.

## Fonctionnalités

- Reconnaissance vocale locale, hors ligne, accélérée par Metal sur Mac, par CUDA ou Vulkan sous Linux,
  avec repli automatique sur le processeur.
- Moteur « Serveur » au choix : tout service compatible avec l'API OpenAI `/v1/audio/transcriptions`,
  notamment le serveur auto-hébergé de `server/`.
- Ponctuation dictée en français : « virgule », « point à la ligne », « ouvrez les guillemets »…
- Raccourci en maintien (on parle tant que la touche est enfoncée), en bascule, ou les deux selon la durée
  de l'appui.
- Collage automatique dans l'application active, avec restauration du presse-papiers.
- Mises à jour automatiques et silencieuses, signées.
- Sous Linux, trois variantes (standard, NVIDIA et Vulkan), sessions X11 et Wayland.

## Télécharger

Les installeurs de la dernière version sont sur la page publique :
<https://crgenerator.s3.eu-west-par.io.cloud.ovh.net/murmure/index.html>

La page détecte votre système et donne, pour Linux, la commande d'installation adaptée à votre
distribution. Chaque version est aussi publiée dans les
[releases GitHub](https://github.com/RD-Agency/murmure/releases), avec un fichier `SHA256SUMS.txt`.

## Installer

**macOS** (Apple Silicon et Intel) : ouvrez le `.dmg` et glissez Murmure dans Applications. L'application
est signée et notarisée par Apple.

**Windows** 10 et 11 : lancez `Murmure-…-windows-x64-setup.exe`. L'installeur n'est pas encore signé :
SmartScreen peut demander de cliquer sur « Informations complémentaires », puis « Exécuter quand même ».

**Linux** : il faut glibc 2.34 ou plus récente et webkit2gtk-4.1. Murmure est testé sur openSUSE Leap 15.6
et 16.0, Tumbleweed, Fedora 43 et 44, Debian 12 et 13, Ubuntu 22.04 et 24.04, et Linux Mint 22.
Choisissez la variante selon votre carte graphique :

- **standard** (`.rpm`, `.deb` ou `.AppImage`) : fonctionne partout, transcription sur le processeur ;
- **NVIDIA** (`-cuda`, `.rpm` ou `.deb`) : CUDA 12 est inclus, seul le pilote propriétaire NVIDIA est requis ;
- **Vulkan** (`-vulkan`, `.rpm` ou `.deb`) : cartes NVIDIA, AMD et Intel, paquet léger, seul le pilote
  graphique est requis. C'est aussi la solution de secours si la variante NVIDIA ne démarre pas.

Par exemple, sur Fedora avec une carte AMD :

```bash
curl -LO https://crgenerator.s3.eu-west-par.io.cloud.ovh.net/murmure/latest/Murmure-linux-x64-vulkan.rpm
sudo dnf install -y ./Murmure-linux-x64-vulkan.rpm
```

Sur openSUSE, les paquets ne sont pas signés : `sudo zypper install --allow-unsigned-rpm ./Murmure-….rpm`.
Les détails par distribution, le son et le dépannage sont dans [docs/linux.md](docs/linux.md).

Au premier lancement, Murmure télécharge le modèle de reconnaissance vocale (environ 550 Mo). Ensuite, tout
fonctionne hors ligne. Pour préparer un parc de postes, `murmure --install-model` installe le modèle à
l'avance, sans ouvrir de fenêtre, et les réglages d'un poste de référence se recopient en deux commandes :

```bash
murmure --export-config murmure-reglages.json     # sur le poste de référence
murmure --import-config murmure-reglages.json     # sur chaque autre poste, Murmure fermé
```

La clé API du serveur n'est pas exportée, sauf avec `--with-secrets` (ou la case correspondante dans
Réglages › Général) ; le fichier doit alors être gardé en lieu sûr.

## Autorisations

**macOS** : Murmure demande l'accès au micro, et l'accès à l'Accessibilité pour coller le texte dans les
autres applications. L'assistant du premier lancement ouvre directement les bons panneaux des Réglages.

**Linux, session X11** : aucune autorisation n'est nécessaire.

**Linux, session Wayland** (KDE Plasma 5.27 ou plus récent, GNOME 48 ou plus récent) : Murmure passe par
les portails XDG du bureau. Au premier lancement, le bouton « Autoriser Murmure » déclenche deux demandes
du bureau, une seule fois : le raccourci global, que vous pouvez changer à ce moment-là, puis le contrôle du
clavier, qui sert uniquement à taper Ctrl+V. Si le bureau ne propose pas ces portails, Murmure se replie
sur `ydotool`, ou copie simplement le texte et vous invite à le coller. La marche à suivre est dans
[docs/linux.md](docs/linux.md#wayland).

**Windows** : aucune autorisation particulière.

## Premiers pas

Le raccourci par défaut est Option+Espace sur Mac et Ctrl+Alt+Espace sous Windows et Linux.

- Maintenez le raccourci, parlez, relâchez : le texte est collé.
- Un appui bref passe en mains libres ; un second appui arrête la dictée.
- Échap annule la dictée en cours (sauf sous Wayland, où Murmure ne capte pas les touches globalement).
- En bascule ou en mains libres, la dictée se termine aussi seule après deux secondes de silence (réglable
  dans Réglages › Raccourci).

Une pastille discrète indique l'enregistrement et la transcription. La fenêtre principale sert surtout aux
réglages : raccourci, langue, micro, vocabulaire (noms propres, termes techniques), mode de collage et
moteur. L'historique des dictées reste sur la machine et peut être désactivé : on y cherche, réécoute et
corrige ses dictées, et les termes souvent corrigés sont proposés pour le vocabulaire.

## Confidentialité

Avec le moteur « Sur cet appareil », l'audio et le texte ne quittent jamais la machine. Murmure n'envoie
aucune statistique d'usage et ne contient aucun outil de mesure d'audience.

Murmure se connecte à Internet dans trois cas seulement :

- le téléchargement du modèle, une fois, depuis Hugging Face (`huggingface.co/ggerganov/whisper.cpp`) ;
- la vérification des mises à jour, dix secondes après le démarrage puis toutes les heures : une simple
  lecture du fichier `latest.json` sur la page de téléchargement, sans identifiant ;
- le moteur « Serveur », si vous le choisissez : l'audio de chaque dictée est alors envoyé au serveur
  configuré. Avec un service tiers comme OpenAI ou Groq, l'audio quitte votre infrastructure.

Les réglages sont un fichier JSON local, l'historique une base SQLite locale. L'audio des dictées y est
conservé par défaut, compressé (environ 9 Mo par heure, 5 Go au plus) : Réglages › Historique permet de le
désactiver ou de le supprimer. La clé API du moteur « Serveur » est rangée dans le trousseau du système,
jamais dans un fichier.

## Compiler depuis les sources

Il faut Rust stable, Node.js 22 ou plus récent, pnpm et CMake (et, sous Linux, les bibliothèques de
développement de webkit2gtk et d'ALSA).

```bash
pnpm install
pnpm tauri dev                            # application complète en mode développement
pnpm tauri build                          # installeurs pour le système courant
pnpm tauri build -- --features vulkan     # variante Vulkan (Linux)
```

Les prérequis par système, les variantes GPU, les tests et la publication d'une version sont décrits dans
[docs/developpement.md](docs/developpement.md). L'organisation du code est dans
[ARCHITECTURE.md](ARCHITECTURE.md).

## Serveur de transcription

Le dossier [`server/`](server/README.md) contient un serveur compatible avec l'API OpenAI, basé sur
faster-whisper, prêt à déployer avec Docker sur une machine équipée d'un GPU NVIDIA. Il permet à toute une
équipe d'utiliser Murmure sur des postes modestes sans que l'audio quitte l'infrastructure.

## Licence

Murmure est un logiciel libre, distribué sous licence
[GNU AGPL v3 ou ultérieure](LICENSE) (`AGPL-3.0-or-later`). Si vous distribuez une version modifiée, ou si
vous la proposez comme service en réseau, vous devez publier son code source sous la même licence.

La variante NVIDIA embarque des bibliothèques CUDA propriétaires : une
[exception de liaison](LICENSE-EXCEPTION-CUDA.md) autorise leur distribution avec Murmure. Les licences des
composants tiers sont listées dans [NOTICE](NOTICE).

## Contribuer

Les contributions sont bienvenues. Lisez [CONTRIBUTING.md](CONTRIBUTING.md) : il explique comment
préparer l'environnement, les vérifications attendues et l'[accord de licence de contribution](CLA.md) à
accepter dans chaque pull request. La participation au projet suit le
[code de conduite](CODE_OF_CONDUCT.md).

## Sécurité

Pour signaler une faille, utilisez le signalement privé de GitHub, décrit dans [SECURITY.md](SECURITY.md).
N'ouvrez pas d'issue publique.

## Contact

Murmure est développé par RD Agency (Rayan Drissi). Pour une question sur le projet, ouvrez une issue ;
pour un déploiement en entreprise ou une licence commerciale, écrivez à rayan.drissi@icloud.com.

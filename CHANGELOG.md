# Historique des versions

## Non publié

- Dictées de plus de 30 s : le moteur local ne perd plus de mots à la jonction des fenêtres de
  Whisper (WER de 16,6 % à 9,1 % sur des dictées de 35 à 48 s, RTX 5060). Les dictées plus courtes
  ne changent pas.
- Moteur local : la dictée est transcrite par segments pendant l'enregistrement, coupés sur les pauses.
  Au relâchement, il ne reste que la fin à transcrire. Pour une dictée de 35 à 48 s, l'attente passe
  de 1,3 s à 0,3 s avec une RTX 5060, et de 13,8 s à 6,9 s sur processeur (Ryzen 9 9950X, 8 threads).
  La ponctuation dictée s'applique au texte complet, y compris à une commande prononcée de part et
  d'autre d'une pause, et Échap annule aussi les segments en cours. Le moteur Serveur envoie toujours
  la dictée d'un bloc.
- Ponctuation dictée : « point. À la ligne » donne bien un point suivi d'un retour à la ligne.
- Textes courts : pour trois mots ou moins, plus de point final ni de majuscule imposée en tête (réglage
  activé par défaut). Un point dicté, un sigle ou un mot du vocabulaire gardent leur forme.
- Arrêt automatique sur silence en bascule et en mains libres : la dictée se termine après 2 s de silence
  qui suivent la parole (de 1 à 10 s, ou jamais, dans Réglages › Raccourci).
- Mot de validation facultatif (« terminé », « envoyer », liste modifiable) : prononcé en fin de dictée, il
  est retiré du texte et Murmure appuie sur Entrée après le collage. Fonctionne avec toutes les méthodes de
  collage (macOS, Windows, X11, portail Wayland, `ydotool`, `wtype`).
- Transcription de fichiers audio depuis l'accueil (WAV, MP3, M4A/AAC, Ogg Vorbis, Opus, FLAC), avec
  découpage sur les silences, barre de progression et annulation. Le texte reçoit la ponctuation dictée et
  se copie d'un clic.
- Baisse du volume pendant la dictée (facultatif) : le son de l'ordinateur est réduit pendant
  l'enregistrement puis rétabli, y compris après une annulation, une erreur ou un arrêt brutal. macOS,
  Windows, et Linux avec `wpctl` ou `pactl`.
- Export et import des réglages en `.json` versionné, depuis Réglages › Général ou en ligne de commande
  (`murmure --export-config fichier.json`, `murmure --import-config fichier.json`). La clé API n'est
  exportée qu'à la demande (`--with-secrets`).
- Historique dans une base locale (SQLite) : recherche plein texte sans accents ni casse, filtre par dates,
  pagination. L'historique existant est importé automatiquement, sans perte ; il n'est plus limité à 500
  dictées.
- Audio des dictées conservé en Opus (environ 9 Mo par heure), à réécouter depuis l'historique. Plafond de
  5 Go réglable plus bas : au-delà, l'audio le plus ancien est supprimé, le texte reste. Bouton
  « Supprimer les audios » dans Réglages › Historique.
- Correction des dictées dans l'historique. Les termes corrigés plusieurs fois sont proposés pour le
  vocabulaire (ajout en un clic, jamais automatique) ; une correction peut devenir une règle de
  remplacement.
- Corrections automatiques limitées aux mots douteux : quand Whisper hésite sur un mot proche d'un terme
  du vocabulaire, Murmure écrit ce terme. Les mots bien reconnus ne sont jamais modifiés.
- Règles de remplacement dans Réglages › Langue et vocabulaire.
- Export de l'historique en CSV ou JSON, et export du corpus (audio et texte corrigé) vers un dossier.

## 0.1.6 — 15 octobre 2026

- Transcription sur processeur environ dix fois plus rapide : les versions pour PC (Linux, Windows, Mac
  Intel) utilisent désormais les instructions AVX2, FMA et F16C. Un processeur de 2013 ou plus récent est
  requis pour le moteur local ; sur un processeur plus ancien, Murmure l'indique et propose le moteur
  « Serveur ».
- Nombre de threads adapté aux processeurs à nombreux cœurs (`MURMURE_THREADS` pour le forcer).
- Page de téléchargement refaite : plus claire, avec le paquet son adapté à chaque distribution Linux.

## 0.1.5 — 15 octobre 2026

- Linux : prise en charge native de Wayland par les portails XDG (KDE Plasma 5.27 et plus, GNOME 48 et
  plus). Le raccourci global passe par le portail GlobalShortcuts, avec les modes maintien, bascule et
  hybride ; le collage passe par le portail RemoteDesktop, avec repli sur `ydotool`, `wtype` ou la copie
  seule. Le bouton « Autoriser Murmure » apparaît dans l'assistant et dans Réglages › Raccourci.
- Presse-papiers Wayland natif (protocoles data-control de KWin et wlroots).
- Linux : l'AppImage embarque FreeType 2.11 et démarre sur openSUSE Leap 15.6.
- Passage en open source sous licence GNU AGPL v3 ou ultérieure, avec une exception pour les
  bibliothèques NVIDIA CUDA. Documentation réécrite.

## 0.1.4 — 15 octobre 2026

- Nouvelle variante Linux Vulkan pour les cartes NVIDIA, AMD et Intel, avec repli sur le processeur.
- Ponctuation dictée en français : « virgule », « point à la ligne », « ouvrez les guillemets »…
- Page de téléchargement refaite : détection du système et commandes prêtes à copier par distribution.
- Linux : plus de plantage au démarrage lié à la pastille ; micro par défaut fiable sous PulseAudio et
  PipeWire.

## 0.1.3 — 15 octobre 2026

- Linux : la variante NVIDIA embarque CUDA 12 ; seul le pilote NVIDIA est requis, y compris avec un pilote
  CUDA 13.

## 0.1.2 — 14 octobre 2026

- Mises à jour automatiques et silencieuses : téléchargement, installation, puis redémarrage discret quand
  l'application est inactive.

## 0.1.1 — 14 octobre 2026

- Première version publiée : moteurs local et serveur, raccourci global, collage automatique, pastille,
  installation automatique du modèle, installeurs pour macOS, Windows et Linux (dont une variante NVIDIA).

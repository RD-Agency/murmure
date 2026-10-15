# Historique des versions

## Non publié

- Dictées de plus de 30 s : le moteur local ne perd plus de mots à la jonction des fenêtres de
  Whisper (WER de 16,6 % à 9,1 % sur des dictées de 35 à 48 s, RTX 5060). Les dictées plus courtes
  ne changent pas.

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

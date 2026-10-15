# Historique des versions

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

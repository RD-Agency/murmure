#!/bin/bash
# Exécuté DANS le conteneur Fedora (sous dbus-run-session) : KWin Wayland virtuel, PipeWire,
# xdg-desktop-portal(-kde), puis Murmure (/out/murmure). Vérifie le portail GlobalShortcuts.
#
# Limite connue : sans GPU, KWin compose en QPainter et n'expose pas `zkde_screencast` ;
# xdg-desktop-portal-kde refuse alors toute session RemoteDesktop (CreateSession → erreur).
# Le collage par portail se teste donc sur une vraie machine (procédure dans la PR / ARCHITECTURE.md).
set -u
export HOME=/root XDG_RUNTIME_DIR=/tmp/xdg XDG_CURRENT_DESKTOP=KDE XDG_SESSION_TYPE=wayland XDG_SESSION_DESKTOP=KDE
mkdir -p -m700 "$XDG_RUNTIME_DIR" ~/.config/fr.rdagency.murmure
OUT=/out
FAIL=0
say() { echo "### $*"; }
check() { # check <fichier> <motif> <description>
  if grep -q -- "$2" "$1"; then say "OK    $3"; else say "ÉCHEC $3"; FAIL=1; fi
}

# Raccourci déjà connu de KDE : sinon le premier BindShortcuts ouvre une boîte de dialogue (aucun clic possible ici).
cat > ~/.config/kglobalshortcutsrc <<'K'
[Murmure]
_k_friendly_name=Murmure
dictation=Ctrl+Alt+Space,Ctrl+Alt+Space,Dicter avec Murmure (maintenir ou appuyer)
K
# Onboarding terminé ; jeton factice pour déclencher aussi la tentative de session clavier.
cat > ~/.config/fr.rdagency.murmure/settings.json <<'J'
{"engine":"remote","remote_endpoint":"http://localhost:9/v1","onboarding_done":true,"wayland_restore_token":"factice","sounds":false}
J

pipewire > $OUT/pipewire.log 2>&1 &
sleep 1
wireplumber > $OUT/wireplumber.log 2>&1 &
sleep 1
kwin_wayland --virtual --no-lockscreen --socket wayland-1 --width 1280 --height 800 > $OUT/kwin.log 2>&1 &
for _ in $(seq 1 60); do [ -S "$XDG_RUNTIME_DIR/wayland-1" ] && break; sleep 1; done
export WAYLAND_DISPLAY=wayland-1 QT_QPA_PLATFORM=wayland
/usr/libexec/xdg-desktop-portal-kde > $OUT/xdp-kde.log 2>&1 &
sleep 3
/usr/libexec/xdg-desktop-portal -r > $OUT/xdp.log 2>&1 &
sleep 4

run_murmure() { # run_murmure <n>
  RUST_LOG=info,murmure_lib=debug WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1 \
    $OUT/murmure > $OUT/murmure-$1.log 2>&1 &
  MPID=$!
  for _ in $(seq 1 90); do
    grep -q "raccourci actif\|Raccourci non accordé\|portail des raccourcis" $OUT/murmure-$1.log && break
    sleep 1
  done
  sleep 2
}

say "Lancement 1 (Murmure.desktop absent : créé puis enregistré)"
run_murmure 1
gdbus call --session --dest org.kde.kglobalaccel --object-path /component/Murmure \
  --method org.kde.kglobalaccel.Component.invokeShortcut dictation > /dev/null
sleep 4
check $OUT/murmure-1.log "Murmure.desktop créé" "fichier Murmure.desktop créé"
check $OUT/murmure-1.log "app id « Murmure » enregistré" "app id enregistré auprès du portail (Registry)"
check $OUT/murmure-1.log 'préféré : Some("CTRL+ALT+space")' "preferred_trigger au format portail"
check $OUT/murmure-1.log "raccourci actif (Ctrl+Alt+Space)" "BindShortcuts : raccourci relu"
check $OUT/murmure-1.log "raccourci Pressed (mode hybride) → Start" "Activated → machine d'états → démarrage"
kill $MPID 2>/dev/null; sleep 2

say "Lancement 2 (app id déjà connu)"
run_murmure 2
check $OUT/murmure-2.log "app id « Murmure » enregistré" "relance : app id enregistré du premier coup"
check $OUT/murmure-2.log "raccourci actif (Ctrl+Alt+Space)" "relance : raccourci rebranché sans boîte de dialogue"
kill $MPID 2>/dev/null

grep -E "portail|clavier|raccourci|panicked" $OUT/murmure-1.log $OUT/murmure-2.log | sed 's/^/    /'
[ $FAIL = 0 ] && say "SUCCÈS" || say "ÉCHEC"
exit $FAIL

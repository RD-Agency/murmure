# Session KDE Wayland headless (KWin virtuel + xdg-desktop-portal-kde) pour tester les portails.
FROM fedora:44
RUN dnf -y install --setopt=install_weak_deps=False kwin-wayland kglobalacceld xdg-desktop-portal xdg-desktop-portal-kde \
      dbus-daemon dbus-tools glib2 pipewire wireplumber webkit2gtk4.1 gtk3 libxdo alsa-lib libayatana-appindicator-gtk3 \
      mesa-dri-drivers xorg-x11-server-Xwayland qt6-qtwayland procps-ng \
    && dnf clean all

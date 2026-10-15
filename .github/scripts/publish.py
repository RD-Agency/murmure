# SPDX-License-Identifier: AGPL-3.0-or-later
"""Prépare la publication S3 d'une release Murmure.

Entrée : le dossier des fichiers d'une version (installeurs + .sig).
Sortie (--out) : latest.json (mise à jour auto Tauri), latest-map.txt (copies « latest »
sans numéro de version), index.html (page de téléchargement) et icon.png.

La page est pilotée par les fichiers présents : une variante ou un format absent de la
release n'affiche ni bouton ni commande.
"""
import argparse
import datetime as dt
import html
import json
import math
import shutil
from pathlib import Path

# Variantes et formats publiés (copies stables latest/Murmure-<variante><ext>).
VARIANTS = ["macos-arm64", "macos-x64", "windows-x64", "linux-x64", "linux-x64-cuda", "linux-x64-vulkan"]
FORMATS = [".dmg", "-setup.exe", ".msi", ".AppImage", ".deb", ".rpm"]
# Clés du manifeste Tauri v2 → (variante, extension). Les variantes CUDA et Vulkan sont exclues :
# la clé linux-x86_64 les ferait « mettre à jour » vers la version sans GPU.
UPDATER = {
    "darwin-aarch64": ("macos-arm64", ".app.tar.gz"),
    "darwin-x86_64": ("macos-x64", ".app.tar.gz"),
    "linux-x86_64": ("linux-x64", ".AppImage"),
    "linux-x86_64-appimage": ("linux-x64", ".AppImage"),
    "linux-x86_64-deb": ("linux-x64", ".deb"),
    "linux-x86_64-rpm": ("linux-x64", ".rpm"),
    "windows-x86_64": ("windows-x64", "-setup.exe"),
    "windows-x86_64-nsis": ("windows-x64", "-setup.exe"),
    "windows-x86_64-msi": ("windows-x64", ".msi"),
}
# Manifeste dédié à la variante CUDA (cuda/latest.json, endpoint posé au build de cette variante).
UPDATER_CUDA = {
    "linux-x86_64": ("linux-x64-cuda", ".rpm"),
    "linux-x86_64-deb": ("linux-x64-cuda", ".deb"),
    "linux-x86_64-rpm": ("linux-x64-cuda", ".rpm"),
}
# Idem pour la variante Vulkan (vulkan/latest.json).
UPDATER_VULKAN = {
    "linux-x86_64": ("linux-x64-vulkan", ".rpm"),
    "linux-x86_64-deb": ("linux-x64-vulkan", ".deb"),
    "linux-x86_64-rpm": ("linux-x64-vulkan", ".rpm"),
}


def human(n: int) -> str:
    return f"{n / 1_000_000:.0f} Mo" if n >= 1_000_000 else f"{n / 1000:.0f} ko"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--tag", required=True)
    ap.add_argument("--base", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    src, out = Path(a.dir), Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    version = a.tag.lstrip("v")
    rel = f"{a.base}/releases/{a.tag}"
    files = {p.name: p for p in src.iterdir() if p.is_file()}

    def name(variant: str, ext: str) -> str:
        return f"Murmure-{version}-{variant}{ext}"

    # Manifestes de mise à jour.
    def manifest(mapping: dict) -> dict:
        platforms = {}
        for key, (variant, ext) in mapping.items():
            n = name(variant, ext)
            if n in files and f"{n}.sig" in files:
                platforms[key] = {"signature": files[f"{n}.sig"].read_text().strip(), "url": f"{rel}/{n}"}
        return {
            "version": version,
            "notes": f"Murmure {version}",
            "pub_date": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "platforms": platforms,
        }

    (out / "latest.json").write_text(json.dumps(manifest(UPDATER), indent=2, ensure_ascii=False))
    for sub, mapping in (("cuda", UPDATER_CUDA), ("vulkan", UPDATER_VULKAN)):
        m = manifest(mapping)
        if m["platforms"]:
            (out / sub).mkdir(exist_ok=True)
            (out / sub / "latest.json").write_text(json.dumps(m, indent=2, ensure_ascii=False))

    # Copies stables : (variante, extension) → Fichier.
    latest_map, avail = [], {}
    for variant in VARIANTS:
        for ext in FORMATS:
            n = name(variant, ext)
            if n not in files:
                continue
            stable = f"Murmure-{variant}{ext}"
            latest_map.append(f"{n} {stable}")
            avail[(variant, ext)] = Fichier(stable, f"{a.base}/latest/{stable}", files[n].stat().st_size)
    (out / "latest-map.txt").write_text("\n".join(latest_map) + "\n")

    listing = [(n, f"{rel}/{n}", files[n].stat().st_size) for n in sorted(files) if not n.endswith(".sig")]
    page = render_page(version, dt.date.today(), avail, listing, f"{rel}/SHA256SUMS.txt" if "SHA256SUMS.txt" in files else None)
    (out / "index.html").write_text(page)
    shutil.copy(Path(__file__).resolve().parents[2] / "src-tauri/icons/128x128@2x.png", out / "icon.png")


# ---------------------------------------------------------------------------------------------
# Page de téléchargement (HTML autonome : aucune ressource externe hormis icon.png).
# ---------------------------------------------------------------------------------------------

class Fichier:
    def __init__(self, nom: str, url: str, taille: int):
        self.nom, self.url, self.taille = nom, url, taille


e = html.escape
MOIS = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août",
        "septembre", "octobre", "novembre", "décembre"]


def _ubuntu() -> str:
    """Cercle des amis : un anneau coupé en trois arcs et trois points."""
    parts, r, rd, gap = [], 6.6, 9.0, 24
    angles = [-60, 60, 180]
    for i, a0 in enumerate(angles):
        a1 = angles[(i + 1) % 3] + (360 if i == 2 else 0)
        s, t = math.radians(a0 + gap), math.radians(a1 - gap)
        parts.append(f"M{12 + r * math.cos(s):.2f} {12 + r * math.sin(s):.2f}"
                     f"A{r} {r} 0 0 1 {12 + r * math.cos(t):.2f} {12 + r * math.sin(t):.2f}")
    dots = "".join(f'<circle cx="{12 + rd * math.cos(math.radians(a)):.2f}" '
                   f'cy="{12 + rd * math.sin(math.radians(a)):.2f}" r="2.6"/>' for a in angles)
    return (f'<path d="{"".join(parts)}" fill="none" stroke="currentColor" stroke-width="2.6"/>'
            f'<g fill="currentColor">{dots}</g>')


def _debian() -> str:
    """Spirale ouverte, tracée point par point."""
    pts = []
    for k in range(0, 61):
        th = 0.9 + k * (2.45 * math.pi - 0.9) / 60
        r = 0.6 + 1.25 * th
        pts.append(f"{12.4 + r * math.cos(-th):.2f} {11.6 + r * math.sin(-th):.2f}")
    return (f'<path d="M{" L".join(pts)}" fill="none" stroke="currentColor" '
            f'stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>')


LOGOS = {
    "apple": '<path fill="currentColor" d="M16.4 12.7c0-2.5 2-3.7 2.1-3.8-1.2-1.7-3-1.9-3.6-1.9-1.5-.2-3 .9-3.8.9-.8 0-2-.9-3.3-.8-1.7 0-3.2 1-4.1 2.5-1.7 3-.4 7.5 1.3 9.9.8 1.2 1.8 2.5 3.1 2.5 1.2-.1 1.7-.8 3.2-.8 1.5 0 1.9.8 3.2.8 1.3 0 2.2-1.2 3-2.4.9-1.4 1.3-2.7 1.3-2.8 0 0-2.5-1-2.5-4.1zM14 5.3c.7-.8 1.1-1.9 1-3.1-1 0-2.2.7-2.9 1.5-.6.7-1.2 1.9-1 3 1.1.1 2.2-.6 2.9-1.4z"/>',
    "windows": '<g fill="currentColor"><rect x="3" y="3" width="8.4" height="8.4" rx=".6"/><rect x="12.6" y="3" width="8.4" height="8.4" rx=".6"/><rect x="3" y="12.6" width="8.4" height="8.4" rx=".6"/><rect x="12.6" y="12.6" width="8.4" height="8.4" rx=".6"/></g>',
    "linux": '<g fill="currentColor"><ellipse cx="12" cy="7.3" rx="4.3" ry="4.9"/><ellipse cx="12" cy="15.4" rx="6.3" ry="6.6"/><ellipse cx="7.9" cy="21.4" rx="3.3" ry="1.5"/><ellipse cx="16.1" cy="21.4" rx="3.3" ry="1.5"/></g><g fill="var(--cut)"><ellipse cx="12" cy="16" rx="4" ry="5.1"/><ellipse cx="10.4" cy="6.4" rx="1.25" ry="1.55"/><ellipse cx="13.6" cy="6.4" rx="1.25" ry="1.55"/><ellipse cx="12" cy="9.4" rx="2.1" ry="1"/></g><g fill="currentColor"><circle cx="10.65" cy="6.75" r=".6"/><circle cx="13.35" cy="6.75" r=".6"/></g>',
    "opensuse": '<path fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" d="M20.6 13.4V12.6C20.6 7.6 16.9 4.6 12.3 4.6 6.9 4.6 3.5 8.3 3.5 12.7c0 4 2.9 6.9 6.5 6.9 3 0 4.8-2.2 4.8-4.5 0-2-1.5-3.4-3.3-3.4-1.7 0-2.8 1.3-2.8 2.7 0 1.3 1 2.1 2.1 2.1"/><circle cx="16.4" cy="8.7" r="1.35" fill="currentColor"/>',
    "fedora": '<circle cx="12" cy="12" r="10" fill="currentColor"/><path fill="none" stroke="var(--cut)" stroke-width="2.6" stroke-linecap="round" d="M16.8 6.6h-1.4a3.4 3.4 0 0 0-3.4 3.4v5.2a3.2 3.2 0 0 1-3.2 3.2H7.4M8 12.2h8.2"/>',
    "debian": _debian(),
    "ubuntu": _ubuntu(),
    "appimage": '<rect x="3.5" y="3.5" width="17" height="17" rx="3" fill="none" stroke="currentColor" stroke-width="2"/><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M12 7.5v7m-3.2-3.2L12 14.5l3.2-3.2M8 17.2h8"/>',
    "copy": '<path fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" d="M8 8h11v12H8zM5 16V4h11"/>',
    "download": '<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M12 4v11m-4.5-4.5L12 15l4.5-4.5M5 20h14"/>',
}


def logo(key: str, size: int = 20, cls: str = "logo") -> str:
    return (f'<svg class="{cls}" width="{size}" height="{size}" viewBox="0 0 24 24" aria-hidden="true" '
            f'focusable="false">{LOGOS[key]}</svg>')


def code_block(lines: list, label: str) -> str:
    """Bloc de commandes avec bouton Copier ; les lignes « # » sont des commentaires."""
    body = "".join(
        f'<span class="l{" c" if l.lstrip().startswith("#") else ""}">{e(l)}</span>' for l in lines
    )
    return (f'<div class="code"><div class="code-bar"><span>{e(label)}</span>'
            f'<button type="button" class="copy">{logo("copy", 15, "ico")}<span>Copier</span></button></div>'
            f'<pre><code>{body}</code></pre></div>')


def dl_button(f: Fichier, title: str, sub: str, primary: bool = True) -> str:
    return (f'<a class="dl{" primary" if primary else ""}" href="{e(f.url)}">{logo("download", 20, "ico")}'
            f'<span><b>{e(title)}</b><small>{e(sub)} · {human(f.taille)}</small></span></a>')


# Distributions Linux : (id, libellé, logos, format).
DISTROS = [
    ("leap15", "openSUSE Leap 15.6", ["opensuse"], ".rpm"),
    ("leap16", "openSUSE Leap 16 · Tumbleweed", ["opensuse"], ".rpm"),
    ("fedora", "Fedora", ["fedora"], ".rpm"),
    ("debian", "Debian · Ubuntu", ["debian", "ubuntu"], ".deb"),
    ("appimage", "Autre distribution (AppImage)", ["appimage"], ".AppImage"),
]


def linux_commands(distro: str, f: Fichier) -> list:
    tmp = f"/tmp/{f.nom}"
    dl = f"curl -fL -o {tmp} {f.url}"
    launch = ["# 4. Lancer Murmure", "nohup murmure >/dev/null 2>&1 &"]
    if distro in ("leap15", "leap16"):
        steps = ["sudo zypper --non-interactive remove murmure >/dev/null 2>&1",
                 f"sudo zypper --non-interactive install --allow-unsigned-rpm {tmp}"]
    elif distro == "fedora":
        steps = ["sudo dnf remove -y murmure >/dev/null 2>&1",
                 f"sudo dnf install -y {tmp}"]
    elif distro == "debian":
        steps = ["sudo apt-get remove -y murmure >/dev/null 2>&1",
                 f"sudo apt-get install -y {tmp}"]
    else:
        app = "~/Applications/Murmure.AppImage"
        return ["# 1. Télécharger Murmure dans le dossier Applications", "mkdir -p ~/Applications",
                f"curl -fL -o {app} {f.url}",
                "# 2. Rendre le fichier exécutable", f"chmod +x {app}",
                "# 3. Lancer Murmure", f"nohup {app} >/dev/null 2>&1 &"]
    # Accolades : le shell lit tout le bloc avant d'exécuter, le mot de passe sudo
    # ne « mange » donc pas les lignes collées suivantes.
    return ["{", "# 1. Télécharger Murmure", dl,
            "# 2. Retirer une éventuelle installation précédente", steps[0],
            "# 3. Installer (le mot de passe administrateur est demandé)", steps[1],
            *launch, "}"]


def nvidia_note(distro: str) -> str:
    base = ("Seul le <b>pilote NVIDIA</b> est requis : CUDA est inclus dans Murmure. "
            "Vérifiez que la carte est reconnue avec <code>nvidia-smi</code>.")
    extra = {
        "leap15": " Commande introuvable&nbsp;? <code>sudo zypper install nvidia-compute-utils-G06</code> (dépôt NVIDIA activé).",
        "leap16": " Commande introuvable&nbsp;? <code>sudo zypper install nvidia-compute-utils-G06</code> (dépôt NVIDIA activé).",
        "fedora": " Pilote absent&nbsp;? Avec le dépôt RPM Fusion&nbsp;: <code>sudo dnf install akmod-nvidia xorg-x11-drv-nvidia-cuda</code>, puis redémarrer.",
        "debian": " Pilote absent&nbsp;? Ubuntu&nbsp;: <code>sudo ubuntu-drivers install</code>&nbsp;; Debian&nbsp;: paquet <code>nvidia-driver</code> (dépôt non-free), puis redémarrer.",
    }
    return base + extra.get(distro, "")


def vulkan_note(distro: str) -> str:
    base = ("Accélération par la carte graphique via <b>Vulkan</b>&nbsp;: cartes NVIDIA, AMD et Intel. "
            "Rien à installer en plus du pilote graphique habituel (NVIDIA ou Mesa). "
            "Sans pilote Vulkan, Murmure fonctionne quand même, sur le processeur.")
    extra = {
        "leap15": " Vérifier&nbsp;: <code>sudo zypper install vulkan-tools</code> puis <code>vulkaninfo --summary</code>.",
        "leap16": " Vérifier&nbsp;: <code>sudo zypper install vulkan-tools</code> puis <code>vulkaninfo --summary</code>.",
        "fedora": " Vérifier&nbsp;: <code>sudo dnf install vulkan-tools</code> puis <code>vulkaninfo --summary</code>. Carte AMD ou Intel&nbsp;: pilote <code>mesa-vulkan-drivers</code>.",
        "debian": " Vérifier&nbsp;: <code>sudo apt install vulkan-tools</code> puis <code>vulkaninfo --summary</code>. Carte AMD ou Intel&nbsp;: pilote <code>mesa-vulkan-drivers</code>.",
    }
    return base + extra.get(distro, "")


# Choix « carte graphique » : (mode, variante, libellé de la puce, titre sans JavaScript).
GPU_MODES = [
    ("nvidia", "linux-x64-cuda", "Carte NVIDIA", "carte NVIDIA"),
    ("vulkan", "linux-x64-vulkan", "Carte graphique (Vulkan) — si la version NVIDIA ne marche pas, ou carte AMD/Intel",
     "carte graphique (Vulkan)"),
    ("std", "linux-x64", "Sans carte graphique / je ne sais pas", "sans carte graphique"),
]
VARIANT_LABEL = {"linux-x64": "standard", "linux-x64-cuda": "NVIDIA", "linux-x64-vulkan": "Vulkan"}


def session_note(distro: str) -> str:
    if distro == "leap15":
        return "Session X11 (par défaut sur Leap 15.6)&nbsp;: le raccourci <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Espace</kbd> fonctionne directement."
    if distro == "appimage":
        return ("L'AppImage nécessite FUSE 2. S'il ne démarre pas&nbsp;: <code>sudo apt install libfuse2t64</code> "
                "(Ubuntu 24.04 et plus), <code>libfuse2</code> (Debian, openSUSE) ou <code>fuse-libs</code> (Fedora).")
    return ("Session Wayland (par défaut sur cette distribution)&nbsp;: au premier lancement, cliquez sur "
            "« Autoriser Murmure », voir <a href=\"#wayland\">Premiers pas</a>.")


def linux_panel(avail: dict) -> str:
    modes = [m for m in GPU_MODES if m[0] == "std" or any((m[1], ext) in avail for *_, ext in DISTROS)]
    distros, combos = [], []
    for did, label, logos, ext in DISTROS:
        std = avail.get(("linux-x64", ext))
        if not any(avail.get((variant, ext)) for _, variant, *_ in modes):
            continue
        distros.append(
            f'<button type="button" class="chip" data-distro="{did}" aria-pressed="false">'
            f'{"".join(logo(k, 18) for k in logos)}<span>{e(label)}</span></button>')
        for mode, variant, _, title in modes:
            own = avail.get((variant, ext))
            f = own or std or next(avail[(v, ext)] for _, v, *_ in modes if (v, ext) in avail)
            warn, notes = [], []
            if mode != "std" and not own:
                warn.append(f"La version {VARIANT_LABEL[variant]} n'existe pas au format "
                            f"{'AppImage' if ext == '.AppImage' else ext} pour cette version&nbsp;: "
                            "la version standard ci-dessous fonctionne aussi, la transcription "
                            "se fait alors sur le processeur (plus lentement)."
                            + (" Pour profiter de la carte graphique, choisissez plutôt votre distribution "
                               "ci-dessus (paquet .rpm ou .deb)." if ext == ".AppImage" else ""))
            elif mode == "nvidia":
                notes.append(nvidia_note(did))
            elif mode == "vulkan":
                notes.append(vulkan_note(did))
            if mode == "std" and not std:
                warn.append("Seule une version pour carte graphique est disponible pour cette version.")
            notes.append(session_note(did))
            shown = next(VARIANT_LABEL[v] for v in VARIANT_LABEL if avail.get((v, ext)) is f)
            combos.append(
                f'<div class="combo" data-distro="{did}" data-gpu="{mode}">'
                f'<h4 class="combo-title">{e(label)} · {e(title)}</h4>'
                + "".join(f'<p class="note warn">{n}</p>' for n in warn)
                + code_block(linux_commands(did, f), f"Terminal · version {shown}")
                + f'<p class="file">Fichier&nbsp;: <a href="{e(f.url)}">{e(f.nom)}</a> · {human(f.taille)}</p>'
                + "".join(f'<p class="note">{n}</p>' for n in notes)
                + "</div>")
    if not distros:
        return ""
    gpu_chips = "".join(
        f'<button type="button" class="chip" data-gpu="{mode}" aria-pressed="false"><span>{e(chip)}</span></button>'
        for mode, _, chip, _ in modes)
    return f"""
<div class="panel" id="p-linux" data-os="linux">
  <ol class="steps">
    <li><span class="step-t">Votre distribution</span>
      <div class="chips" role="group" aria-label="Distribution">{"".join(distros)}</div>
      <p class="hint">Pour la connaître&nbsp;: <code>grep PRETTY_NAME /etc/os-release</code></p></li>
    <li><span class="step-t">Votre carte graphique</span>
      <div class="chips" role="group" aria-label="Carte graphique">{gpu_chips}</div><p class="hint" id="gpu-hint"></p></li>
    <li><span class="step-t">Copiez le bloc, collez-le dans un terminal (Konsole&nbsp;: <kbd>Ctrl</kbd>+<kbd>Maj</kbd>+<kbd>V</kbd>), appuyez sur <kbd>Entrée</kbd></span>
      {"".join(combos)}
    </li>
  </ol>
</div>"""


def render_page(version: str, date: dt.date, avail: dict, listing: list, sums) -> str:
    tabs, panels = [], []

    mac = [(avail.get(("macos-arm64", ".dmg")), "Mac Apple Silicon", "M1, M2, M3, M4… · .dmg", "arm"),
           (avail.get(("macos-x64", ".dmg")), "Mac Intel", "processeur Intel · .dmg", "intel")]
    mac = [m for m in mac if m[0]]
    if mac:
        tabs.append(("macos", "macOS", "apple"))
        btns = "".join(f'<div data-arch="{arch}">{dl_button(f, t, s)}</div>' for f, t, s, arch in mac)
        panels.append(f"""
<div class="panel" id="p-macos" data-os="macos">
  <div class="buttons">{btns}</div>
  <p class="hint">Quel Mac&nbsp;? Menu Pomme › À propos de ce Mac&nbsp;: « Puce Apple » = Apple Silicon, « Processeur Intel » = Intel.</p>
  <ol class="steps">
    <li>Ouvrez le fichier <b>.dmg</b> téléchargé.</li>
    <li>Glissez <b>Murmure</b> dans le dossier <b>Applications</b>, puis lancez-le depuis Applications.</li>
    <li>Autorisez le <b>Micro</b> et l'<b>Accessibilité</b> quand macOS les demande (Réglages Système › Confidentialité et sécurité). L'Accessibilité sert à écrire le texte dans les autres applications.</li>
  </ol>
  <p class="note">Si macOS refuse d'ouvrir l'application&nbsp;: Réglages Système › Confidentialité et sécurité › « Ouvrir quand même ».</p>
</div>""")

    exe, msi = avail.get(("windows-x64", "-setup.exe")), avail.get(("windows-x64", ".msi"))
    if exe or msi:
        tabs.append(("windows", "Windows", "windows"))
        main_btn = dl_button(exe, "Télécharger pour Windows", "Windows 10 et 11 · installeur .exe") if exe else \
            dl_button(msi, "Télécharger pour Windows", "Windows 10 et 11 · paquet .msi")
        alt = (f'<p class="hint">Déploiement en entreprise&nbsp;: <a href="{e(msi.url)}">paquet .msi</a> · {human(msi.taille)}</p>'
               if exe and msi else "")
        panels.append(f"""
<div class="panel" id="p-windows" data-os="windows">
  <div class="buttons">{main_btn}</div>{alt}
  <ol class="steps">
    <li>Ouvrez le fichier téléchargé et suivez l'installation.</li>
    <li>Si Windows affiche « Windows a protégé votre ordinateur » (SmartScreen)&nbsp;: cliquez sur <b>Informations complémentaires</b> › <b>Exécuter quand même</b>.</li>
    <li>Murmure démarre et s'installe dans la zone de notification, en bas à droite.</li>
  </ol>
</div>""")

    lp = linux_panel(avail)
    if lp:
        tabs.append(("linux", "Linux", "linux"))
        panels.append(lp)

    tab_html = "".join(
        f'<button type="button" class="tab" role="tab" data-os="{oid}" aria-selected="false" aria-controls="p-{oid}">'
        f'{logo(lg, 20)}<span>{e(lbl)}</span></button>' for oid, lbl, lg in tabs)

    diag = [
        ("La carte NVIDIA et son pilote sont-ils reconnus ?", ["nvidia-smi | head -5"]),
        ("La version NVIDIA de Murmure est-elle installée ? (liste les bibliothèques CUDA)", ["ls /usr/lib/murmure/cuda"]),
        ("Version Vulkan : la carte est-elle visible en Vulkan ? (paquet vulkan-tools)", ["vulkaninfo --summary | grep -E 'deviceName|driverName'"]),
        ("Type de session : x11 ou wayland ?", ["echo $XDG_SESSION_TYPE"]),
        ("Relancer Murmure en gardant un journal (reproduisez le problème, puis quittez Murmure)",
         ["pkill -x murmure; murmure 2>&1 | tee /tmp/murmure.log"]),
        ("Accélération utilisée par Murmure (Vulkan, CUDA ou processeur), d'après ce journal", ["grep -a 'Accélération' /tmp/murmure.log"]),
        ("Afficher les 20 dernières lignes du journal, à envoyer au support", ["tail -20 /tmp/murmure.log"]),
    ]
    diag_html = "".join(f'<p class="diag-t">{e(t)}</p>{code_block(c, "Terminal")}' for t, c in diag)

    files_html = "".join(
        f'<li><a href="{e(u)}">{e(n)}</a><span>{human(s)}</span></li>' for n, u, s in listing)
    sums_link = f' · <a href="{e(sums)}">Sommes de contrôle SHA-256</a>' if sums else ""

    date_txt = f"{date.day}{'er' if date.day == 1 else ''} {MOIS[date.month - 1]} {date.year}"
    page = (TEMPLATE.replace("{{VERSION}}", e(version)).replace("{{DATE}}", date_txt)
            .replace("{{TABS}}", tab_html).replace("{{PANELS}}", "".join(panels))
            .replace("{{DIAG}}", diag_html).replace("{{FILES}}", files_html).replace("{{SUMS}}", sums_link))
    return page


TEMPLATE = r"""<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Télécharger Murmure</title>
<meta name="description" content="Murmure, dictée vocale sur votre poste : téléchargement pour macOS, Windows et Linux.">
<link rel="icon" href="icon.png">
<style>
:root {
  --bg:#f5f6f8; --surface:#fff; --text:#16181d; --muted:#5b616e; --line:#e2e5ea; --line-strong:#cfd3da;
  --accent:#3c43c4; --accent-hover:#2f35a8; --accent-ink:#fff; --accent-soft:#eef0fd;
  --code-bg:#f2f3f6; --code-text:#16181d; --comment:#6b7180; --kbd:#fff; --cut:#fff;
  --ok:#1e7a46;
  color-scheme: light dark;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg:#111317; --surface:#191b21; --text:#e7e9ee; --muted:#a2a7b3; --line:#2a2d35; --line-strong:#3a3e48;
    --accent:#9da2ff; --accent-hover:#b5b9ff; --accent-ink:#121318; --accent-soft:#22243d;
    --code-bg:#0e0f13; --code-text:#e7e9ee; --comment:#8b91a0; --kbd:#23262e; --cut:#191b21;
    --ok:#6fd49b;
  }
}
* { box-sizing:border-box; }
html { -webkit-text-size-adjust:100%; }
body { margin:0; background:var(--bg); color:var(--text);
  font:16px/1.55 system-ui, -apple-system, "Segoe UI", Roboto, "Noto Sans", Ubuntu, Cantarell, "Helvetica Neue", Arial, sans-serif; }
a { color:var(--accent); text-underline-offset:2px; }
a:hover { color:var(--accent-hover); }
code, kbd, pre { font-family:ui-monospace, "SF Mono", Menlo, Consolas, "DejaVu Sans Mono", "Liberation Mono", monospace; }
code { font-size:.88em; background:var(--code-bg); border:1px solid var(--line); border-radius:5px; padding:.08em .35em; overflow-wrap:anywhere; }
kbd { font-size:.82em; background:var(--kbd); border:1px solid var(--line-strong); border-bottom-width:2px; border-radius:5px; padding:.05em .4em; white-space:nowrap; }
.wrap { max-width:860px; margin:0 auto; padding:40px 16px 56px; }
.ico, .logo { flex:none; display:block; }

/* En-tête */
.top { display:flex; align-items:center; gap:16px; }
.top img { width:64px; height:64px; border-radius:14px; flex:none; }
h1 { font-size:30px; line-height:1.15; letter-spacing:-.015em; margin:0; }
.meta { margin:2px 0 0; color:var(--muted); font-size:14px; }
.lead { font-size:17px; margin:20px 0 0; max-width:640px; }

/* Sections */
section { margin-top:36px; }
h2 { font-size:20px; letter-spacing:-.01em; margin:0 0 6px; }
h2 .n { display:inline-flex; align-items:center; justify-content:center; width:26px; height:26px; margin-right:8px;
  border-radius:50%; background:var(--accent); color:var(--accent-ink); font-size:14px; vertical-align:2px; }
.card { background:var(--surface); border:1px solid var(--line); border-radius:12px; padding:20px; margin-top:14px; }
.detected { color:var(--muted); font-size:14px; margin:0; }

/* Onglets systèmes */
.tabs { display:grid; grid-template-columns:repeat(auto-fit, minmax(0, 1fr)); gap:0; border-bottom:1px solid var(--line); margin:-20px -20px 20px; }
.tab { appearance:none; background:none; border:0; border-bottom:3px solid transparent; color:var(--muted); font:inherit; font-weight:600;
  display:flex; align-items:center; justify-content:center; gap:8px; padding:14px 8px 11px; cursor:pointer; }
.tab:hover { color:var(--text); }
.tab[aria-selected="true"] { color:var(--text); border-bottom-color:var(--accent); }
.tab:focus-visible, .chip:focus-visible, .copy:focus-visible, .dl:focus-visible, summary:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }

/* Boutons de téléchargement */
.buttons { display:grid; grid-template-columns:repeat(auto-fit, minmax(240px, 1fr)); gap:10px; }
.dl { display:flex; align-items:center; gap:12px; text-decoration:none; border-radius:10px; padding:12px 16px;
  border:1px solid var(--line-strong); color:var(--text); background:var(--surface); }
.dl:hover { border-color:var(--accent); color:var(--text); }
.dl.primary { background:var(--accent); border-color:var(--accent); color:var(--accent-ink); }
.dl.primary:hover { background:var(--accent-hover); border-color:var(--accent-hover); color:var(--accent-ink); }
.dl b { display:block; font-size:16px; }
.dl small { display:block; font-size:13px; opacity:.85; }
.mac-other .dl.primary { background:var(--surface); color:var(--text); border-color:var(--line-strong); }

/* Étapes */
.steps { margin:18px 0 0; padding-left:1.3em; }
.steps > li { margin:0 0 12px; padding-left:4px; }
.steps > li::marker { color:var(--muted); font-weight:600; }
.step-t { display:block; font-weight:600; margin-bottom:8px; }
.hint { color:var(--muted); font-size:14px; margin:10px 0 0; }
.note.warn { margin:0 0 12px; border-left-color:var(--accent); }
.note { font-size:14px; margin:10px 0 0; padding:10px 12px; border-left:3px solid var(--line-strong); background:var(--bg); border-radius:0 6px 6px 0; }

/* Puces de choix (distribution, carte graphique) */
.chips { display:flex; flex-wrap:wrap; gap:8px; }
.chip { appearance:none; font:inherit; font-size:14px; color:var(--text); background:var(--surface); border:1px solid var(--line-strong);
  border-radius:8px; padding:8px 12px; display:inline-flex; align-items:center; gap:7px; cursor:pointer; --cut:var(--surface); }
.chip:hover { border-color:var(--accent); }
.chip[aria-pressed="true"] { border-color:var(--accent); background:var(--accent-soft); box-shadow:inset 0 0 0 1px var(--accent); --cut:var(--accent-soft); }
.chip .logo { color:var(--text); }

/* Blocs de commandes */
.code { border:1px solid var(--line); border-radius:10px; overflow:hidden; background:var(--code-bg); margin-top:4px; }
.code + .code { margin-top:10px; }
.code-bar { display:flex; align-items:center; justify-content:space-between; gap:8px; padding:6px 8px 6px 14px;
  border-bottom:1px solid var(--line); font-size:13px; color:var(--muted); background:var(--surface); }
.copy { appearance:none; font:inherit; font-size:13px; font-weight:600; color:var(--text); background:var(--surface);
  border:1px solid var(--line-strong); border-radius:7px; padding:5px 10px; display:inline-flex; align-items:center; gap:6px; cursor:pointer; }
.copy:hover { border-color:var(--accent); }
.copy.done { color:var(--ok); border-color:var(--ok); }
pre { margin:0; padding:14px; font-size:13.5px; line-height:1.55; color:var(--code-text); white-space:pre-wrap; }
pre .l { display:block; padding-left:2ch; text-indent:-2ch; overflow-wrap:anywhere; }
pre .l + .l { margin-top:1px; }
pre code { background:none; border:0; padding:0; font-size:inherit; overflow-wrap:normal; }
pre .c { color:var(--comment); }
.file { font-size:13px; color:var(--muted); margin:8px 0 0; overflow-wrap:anywhere; }
.combo-title { display:none; }

/* Mode JavaScript : un seul panneau / combinaison visible */
.js .panel:not(.on), .js .combo:not(.on) { display:none; }
html:not(.js) .tabs { display:none; }
html:not(.js) .panel + .panel { margin-top:28px; padding-top:20px; border-top:1px solid var(--line); }
html:not(.js) .combo-title { display:block; font-size:15px; margin:18px 0 6px; }

/* Premiers pas */
.grid { display:grid; grid-template-columns:repeat(auto-fit, minmax(220px, 1fr)); gap:10px; margin-top:14px; }
.tile { background:var(--surface); border:1px solid var(--line); border-radius:10px; padding:14px 16px; }
.tile h3 { font-size:15px; margin:0 0 4px; }
.tile p { margin:0; font-size:14px; color:var(--muted); }
.tile p kbd { color:var(--text); }
.wide { margin-top:10px; }
.wide p { color:var(--text); }

/* Repliables */
details { background:var(--surface); border:1px solid var(--line); border-radius:12px; margin-top:12px; }
summary { cursor:pointer; padding:14px 18px; font-weight:600; list-style:none; display:flex; align-items:center; gap:10px; }
summary::-webkit-details-marker { display:none; }
summary::before { content:""; width:8px; height:8px; border-right:2px solid var(--muted); border-bottom:2px solid var(--muted); transform:rotate(-45deg); transition:transform .15s; flex:none; }
details[open] summary::before { transform:rotate(45deg); }
.details-body { padding:0 18px 18px; }
.diag-t { font-size:14px; margin:14px 0 6px; }
.diag-t:first-child { margin-top:0; }
.files { list-style:none; margin:0; padding:0; font-size:14px; }
.files li { display:flex; justify-content:space-between; gap:12px; padding:7px 0; border-top:1px solid var(--line); }
.files li:first-child { border-top:0; }
.files a { overflow-wrap:anywhere; }
.files span { color:var(--muted); white-space:nowrap; }

footer { margin-top:40px; padding-top:18px; border-top:1px solid var(--line); font-size:13px; color:var(--muted); }

@media (max-width:560px) {
  .wrap { padding-top:24px; }
  .top img { width:52px; height:52px; }
  h1 { font-size:26px; }
  .card { padding:16px; }
  .tabs { margin:-16px -16px 16px; }
  .tab { flex-direction:column; gap:4px; font-size:14px; padding:12px 4px 9px; }
  .chip { font-size:13.5px; }
  pre { font-size:12.5px; }
}
</style>
</head>
<body>
<div class="wrap">
  <header class="top">
    <img src="icon.png" alt="Icône de Murmure">
    <div>
      <h1>Murmure</h1>
      <p class="meta">Version {{VERSION}} · publiée le {{DATE}}</p>
    </div>
  </header>
  <p class="lead">Dictée vocale&nbsp;: maintenez le raccourci, parlez, relâchez — le texte s'écrit là où se trouve le curseur. 100&nbsp;% sur votre poste.</p>

  <section id="installer">
    <h2><span class="n">1</span>Installer</h2>
    <p class="detected" id="detected">Choisissez votre système.</p>
    <div class="card">
      <div class="tabs" role="tablist" aria-label="Système">{{TABS}}</div>
      {{PANELS}}
    </div>
  </section>

  <section id="premiers-pas">
    <h2><span class="n">2</span>Premiers pas</h2>
    <p class="detected">Au premier lancement, Murmure télécharge son modèle de reconnaissance vocale (environ 550&nbsp;Mo), une seule fois. Ensuite tout fonctionne sans connexion.</p>
    <div class="grid">
      <div class="tile"><h3>Dicter</h3><p>Maintenez <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Espace</kbd> (Mac&nbsp;: <kbd>⌥</kbd>+<kbd>Espace</kbd>), parlez, relâchez.</p></div>
      <div class="tile"><h3>Mains libres</h3><p>Un appui bref sur le raccourci démarre la dictée, un second l'arrête.</p></div>
      <div class="tile"><h3>Annuler</h3><p><kbd>Échap</kbd> annule la dictée en cours, rien n'est écrit.</p></div>
    </div>
    <div class="tile wide"><h3>Ponctuation dictée</h3><p>Dites «&nbsp;virgule&nbsp;», «&nbsp;point&nbsp;», «&nbsp;point à la ligne&nbsp;», «&nbsp;deux-points&nbsp;», «&nbsp;ouvrez les guillemets&nbsp;» / «&nbsp;fermez les guillemets&nbsp;»…</p></div>
    <div class="tile wide" id="wayland"><h3>Linux sous Wayland (Leap 16, Tumbleweed, Fedora)</h3><p>Au premier lancement, cliquez sur <b>Autoriser Murmure</b>. Le bureau (KDE Plasma 5.27 et plus, GNOME 48 et plus) demande une seule fois d'accepter le raccourci global, que vous pouvez changer à ce moment-là, puis le contrôle du clavier, qui sert uniquement à coller le texte.</p><p>Bureau plus ancien ou autorisation refusée&nbsp;: dans KDE, Configuration du système › Clavier › Raccourcis › <b>Ajouter une commande</b>, saisissez <code>murmure --toggle</code> et attribuez-lui <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Espace</kbd>. Un appui démarre la dictée, un second l'arrête.</p></div>
  </section>

  <section id="mises-a-jour">
    <h2>Mises à jour</h2>
    <p>Automatiques. Sur Mac, Windows et avec l'AppImage, elles s'installent sans intervention. Avec un paquet .rpm ou .deb, Murmure propose la mise à jour et demande le mot de passe administrateur.</p>
  </section>

  <section id="aide">
    <h2>Besoin d'aide&nbsp;?</h2>
    <details>
      <summary>En cas de problème sous Linux&nbsp;: commandes de diagnostic</summary>
      <div class="details-body">{{DIAG}}</div>
    </details>
    <details id="fichiers">
      <summary>Tous les fichiers de la version {{VERSION}}</summary>
      <div class="details-body"><ul class="files">{{FILES}}</ul></div>
    </details>
  </section>

  <footer>Murmure · RD Agency{{SUMS}} · <a href="#fichiers">Tous les fichiers de cette version</a></footer>
</div>
<script>
(function () {
  var d = document, root = d.documentElement;
  root.className += " js";
  var $ = function (s, el) { return Array.prototype.slice.call((el || d).querySelectorAll(s)); };
  var NAMES = { macos: "macOS", windows: "Windows", linux: "Linux" };
  var state = { os: null, distro: null, gpu: null };

  function has(sel) { return !!d.querySelector(sel); }
  function save() {
    var h = "#" + state.os + (state.os === "linux" ? "/" + state.distro + "/" + state.gpu : "");
    try { history.replaceState(null, "", h); } catch (e) {}
  }
  function render() {
    $('a[href="#fichiers"]').forEach(function (a) { a.addEventListener("click", function () { d.getElementById("fichiers").open = true; }); });
  $(".tab").forEach(function (t) { t.setAttribute("aria-selected", t.dataset.os === state.os ? "true" : "false"); });
    $(".panel").forEach(function (p) { p.classList.toggle("on", p.dataset.os === state.os); });
    $(".chip[data-distro]").forEach(function (c) { c.setAttribute("aria-pressed", c.dataset.distro === state.distro ? "true" : "false"); });
    $(".chip[data-gpu]").forEach(function (c) { c.setAttribute("aria-pressed", c.dataset.gpu === state.gpu ? "true" : "false"); });
    $(".combo").forEach(function (c) { c.classList.toggle("on", c.dataset.distro === state.distro && c.dataset.gpu === state.gpu); });
  }
  function pick(k, v) { state[k] = v; render(); save(); }

  // Détection du système.
  var ua = navigator.userAgent, uad = navigator.userAgentData;
  var plat = (uad && uad.platform) || navigator.platform || "";
  var mobile = /Android|iPhone|iPad|iPod/i.test(ua) || (uad && uad.mobile);
  var os = mobile ? null : /Win/i.test(plat + ua) ? "windows" : /Mac/i.test(plat + ua) ? "macos" : /Linux|X11|CrOS/i.test(plat + ua) ? "linux" : null;
  var distro = /Fedora/i.test(ua) ? "fedora" : /Ubuntu|Debian|Mint/i.test(ua) ? "debian" : "leap15";
  var gpu = "std", gpuSeen = "";
  try {
    var gl = d.createElement("canvas").getContext("webgl");
    var ext = gl && gl.getExtension("WEBGL_debug_renderer_info");
    var r = ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl ? gl.getParameter(gl.RENDERER) : "";
    if (/nvidia|geforce|rtx|quadro/i.test(r)) { gpu = "nvidia"; gpuSeen = "NVIDIA"; }
    else if (/radeon|amd|intel.*arc/i.test(r) && has('.chip[data-gpu="vulkan"]')) { gpu = "vulkan"; gpuSeen = /intel/i.test(r) ? "Intel Arc" : "AMD"; }
  } catch (e) {}

  // Lien partagé : #linux/fedora/nvidia
  var detected = os;
  var h = location.hash.slice(1).split("/");
  if (NAMES[h[0]] && has('.tab[data-os="' + h[0] + '"]')) os = h[0];
  if (h[1] && has('.chip[data-distro="' + h[1] + '"]')) distro = h[1];
  if (h[2] && has('.chip[data-gpu="' + h[2] + '"]')) gpu = h[2];

  var firstTab = d.querySelector(".tab");
  state.os = os && has('.tab[data-os="' + os + '"]') ? os : firstTab && firstTab.dataset.os;
  var firstDistro = d.querySelector(".chip[data-distro]");
  state.distro = has('.chip[data-distro="' + distro + '"]') ? distro : firstDistro && firstDistro.dataset.distro;
  state.gpu = gpu;

  var det = d.getElementById("detected");
  if (det) det.textContent = detected && detected === state.os ? "Système détecté : " + NAMES[detected] + ". Pas le bon ? Choisissez un autre onglet."
    : mobile ? "Ouvrez cette page sur l'ordinateur à équiper, ou choisissez son système ci-dessous." : "Choisissez votre système.";
  var gh = d.getElementById("gpu-hint");
  if (gh) gh.textContent = gpuSeen ? "Carte " + gpuSeen + " détectée sur cet ordinateur." : "Pour vérifier : la commande lspci | grep -i vga affiche le nom de la carte graphique.";

  // Mac : le premier bouton est mis en avant ; Intel passe en tête si détecté.
  function markMac() {
    $('[data-os="macos"] .buttons > div').forEach(function (el, i) { el.classList.toggle("mac-other", i > 0); });
  }
  markMac();
  if (uad && uad.getHighEntropyValues && os === "macos") {
    uad.getHighEntropyValues(["architecture"]).then(function (v) {
      var b = $('[data-os="macos"] .buttons > div');
      if (v.architecture === "x86" && b.length === 2) { b[0].parentNode.appendChild(b[0]); markMac(); }
    }, function () {});
  }

  $('a[href="#fichiers"]').forEach(function (a) { a.addEventListener("click", function () { d.getElementById("fichiers").open = true; }); });
  $(".tab").forEach(function (t) { t.addEventListener("click", function () { pick("os", t.dataset.os); }); });
  $(".chip[data-distro]").forEach(function (c) { c.addEventListener("click", function () { pick("distro", c.dataset.distro); }); });
  $(".chip[data-gpu]").forEach(function (c) { c.addEventListener("click", function () { pick("gpu", c.dataset.gpu); }); });

  // Boutons Copier : Clipboard API, repli sur execCommand.
  function fallback(text) {
    var ta = d.createElement("textarea");
    ta.value = text; ta.setAttribute("readonly", ""); ta.style.position = "fixed"; ta.style.opacity = "0";
    d.body.appendChild(ta); ta.select();
    var ok = false; try { ok = d.execCommand("copy"); } catch (e) {}
    d.body.removeChild(ta); return ok;
  }
  $(".copy").forEach(function (b) {
    b.addEventListener("click", function () {
      var text = $(".l", b.closest(".code")).map(function (l) { return l.textContent; }).join("\n");
      var label = b.querySelector("span");
      function done(ok) {
        label.textContent = ok ? "Copié" : "Sélectionnez puis Ctrl+C";
        b.classList.toggle("done", ok);
        setTimeout(function () { label.textContent = "Copier"; b.classList.remove("done"); }, 2000);
      }
      if (navigator.clipboard && window.isSecureContext) {
        navigator.clipboard.writeText(text).then(function () { done(true); }, function () { done(fallback(text)); });
      } else { done(fallback(text)); }
    });
  });

  render();
})();
</script>
</body>
</html>
"""

if __name__ == "__main__":
    main()

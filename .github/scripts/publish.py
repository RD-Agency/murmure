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
import shutil
from pathlib import Path

# Nom du produit : seul endroit à modifier en cas de changement de nom. Il préfixe les fichiers
# publiés (productName de tauri.conf.json) ; en minuscules, c'est le nom du paquet et de la commande.
APP_NAME = "Murmure"
APP_CMD = APP_NAME.lower()
SOURCE_URL = "https://github.com/RD-Agency/murmure"

# Variantes et formats publiés (copies stables latest/<APP_NAME>-<variante><ext>).
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
        return f"{APP_NAME}-{version}-{variant}{ext}"

    # Manifestes de mise à jour.
    def manifest(mapping: dict) -> dict:
        platforms = {}
        for key, (variant, ext) in mapping.items():
            n = name(variant, ext)
            if n in files and f"{n}.sig" in files:
                platforms[key] = {"signature": files[f"{n}.sig"].read_text().strip(), "url": f"{rel}/{n}"}
        return {
            "version": version,
            "notes": f"{APP_NAME} {version}",
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
            stable = f"{APP_NAME}-{variant}{ext}"
            latest_map.append(f"{n} {stable}")
            avail[(variant, ext)] = Fichier(stable, f"{a.base}/latest/{stable}", files[n].stat().st_size)
    (out / "latest-map.txt").write_text("\n".join(latest_map) + "\n")

    listing = [(n, f"{rel}/{n}", files[n].stat().st_size) for n in sorted(files) if not n.endswith(".sig")]
    page = render_page(version, dt.date.today(), avail, listing, f"{rel}/SHA256SUMS.txt" if "SHA256SUMS.txt" in files else None)
    (out / "index.html").write_text(page)
    shutil.copy(Path(__file__).resolve().parents[2] / "src-tauri/icons/128x128@2x.png", out / "icon.png")


# ---------------------------------------------------------------------------------------------
# Page de téléchargement (HTML autonome : aucune ressource externe hormis icon.png).
# Public : des utilisateurs non informaticiens. Phrases courtes, vouvoiement, typographie
# française (espace insécable avant « : ; ? ! », guillemets « »), aucun tiret d'incise.
# ---------------------------------------------------------------------------------------------

class Fichier:
    def __init__(self, nom: str, url: str, taille: int):
        self.nom, self.url, self.taille = nom, url, taille


e = html.escape
MOIS = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août",
        "septembre", "octobre", "novembre", "décembre"]
NB = "&nbsp;"

# Logos monochromes (24 × 24, couleur du texte).
LOGOS = {
    "apple": '<path fill="currentColor" d="M16.4 12.7c0-2.5 2-3.7 2.1-3.8-1.2-1.7-3-1.9-3.6-1.9-1.5-.2-3 .9-3.8.9-.8 0-2-.9-3.3-.8-1.7 0-3.2 1-4.1 2.5-1.7 3-.4 7.5 1.3 9.9.8 1.2 1.8 2.5 3.1 2.5 1.2-.1 1.7-.8 3.2-.8 1.5 0 1.9.8 3.2.8 1.3 0 2.2-1.2 3-2.4.9-1.4 1.3-2.7 1.3-2.8 0 0-2.5-1-2.5-4.1zM14 5.3c.7-.8 1.1-1.9 1-3.1-1 0-2.2.7-2.9 1.5-.6.7-1.2 1.9-1 3 1.1.1 2.2-.6 2.9-1.4z"/>',
    "windows": '<g fill="currentColor"><rect x="3" y="3" width="8.4" height="8.4" rx=".6"/><rect x="12.6" y="3" width="8.4" height="8.4" rx=".6"/><rect x="3" y="12.6" width="8.4" height="8.4" rx=".6"/><rect x="12.6" y="12.6" width="8.4" height="8.4" rx=".6"/></g>',
    "linux": '<g fill="currentColor"><ellipse cx="12" cy="7.3" rx="4.3" ry="4.9"/><ellipse cx="12" cy="15.4" rx="6.3" ry="6.6"/><ellipse cx="7.9" cy="21.4" rx="3.3" ry="1.5"/><ellipse cx="16.1" cy="21.4" rx="3.3" ry="1.5"/></g><g fill="var(--cut)"><ellipse cx="12" cy="16" rx="4" ry="5.1"/><ellipse cx="10.4" cy="6.4" rx="1.25" ry="1.55"/><ellipse cx="13.6" cy="6.4" rx="1.25" ry="1.55"/><ellipse cx="12" cy="9.4" rx="2.1" ry="1"/></g><g fill="currentColor"><circle cx="10.65" cy="6.75" r=".6"/><circle cx="13.35" cy="6.75" r=".6"/></g>',
    "download": '<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M12 4v11m-4.5-4.5L12 15l4.5-4.5M5 20h14"/>',
}


def logo(key: str, size: int = 18) -> str:
    return (f'<svg class="logo" width="{size}" height="{size}" viewBox="0 0 24 24" aria-hidden="true" '
            f'focusable="false">{LOGOS[key]}</svg>')


def code_block(lines: list) -> str:
    """Bloc de commande copiable. Aucune ligne de commentaire : l'explication va au-dessus."""
    return (f'<div class="code"><pre><code>{e(chr(10).join(lines))}</code></pre>'
            f'<button type="button" class="copy" aria-label="Copier la commande">Copier</button></div>')


# Distributions Linux : (id, libellé, format, paquet son à installer avec Murmure).
# Le paquet son redirige le micro (ALSA) vers le serveur de son du bureau. Fedora l'a déjà
# (pipewire-alsa est installé d'office et recommandé par le paquet) ; Debian et Ubuntu aussi
# (Recommends du .deb, installé par apt).
DISTROS = [
    ("leap15", "openSUSE Leap 15.6", ".rpm", "alsa-plugins-pulse"),
    ("leap16", "openSUSE Tumbleweed ou Leap 16", ".rpm", "pipewire-alsa"),
    ("fedora", "Fedora", ".rpm", ""),
    ("debian", "Debian ou Ubuntu", ".deb", ""),
    ("appimage", "Autre distribution", ".AppImage", ""),
]
# Choix de la carte graphique : mode → variante.
GPU = {"nvidia": "linux-x64-cuda", "std": "linux-x64", "vulkan": "linux-x64-vulkan"}


def linux_commands(distro: str, sound: str, f: Fichier) -> list:
    """Commande d'installation. Les accolades font lire tout le bloc par le shell avant de
    l'exécuter : sans elles, une fois collées, les lignes suivantes seraient lues comme
    le mot de passe demandé par sudo. Retirer l'ancienne installation permet aussi de
    changer de variante (le paquet s'appelle toujours de la même façon)."""
    launch = f"nohup {APP_CMD} >/dev/null 2>&1 &"
    pkgs = f"{sound} " if sound else ""
    if distro in ("leap15", "leap16"):
        body = [f"sudo zypper --non-interactive remove {APP_CMD}",
                f"sudo zypper --non-interactive install --allow-unsigned-rpm {pkgs}\\",
                f"  {f.url}"]
    elif distro == "fedora":
        body = [f"sudo dnf remove -y {APP_CMD}",
                f"sudo dnf install -y {pkgs}\\", f"  {f.url}"]
    elif distro == "debian":
        tmp = f"/tmp/{f.nom}"
        body = [f"curl -fL -o {tmp} \\", f"  {f.url}",
                f"sudo apt-get remove -y {APP_CMD}",
                f"sudo apt-get install -y {tmp}"]
    else:
        app = f"~/Applications/{APP_NAME}.AppImage"
        body = ["mkdir -p ~/Applications",
                f"curl -fL -o {app} \\", f"  {f.url}",
                f"chmod +x {app}"]
        launch = f"nohup {app} >/dev/null 2>&1 &"
    return ["{", *("  " + l for l in body), "  " + launch, "}"]


def linux_panel(avail: dict) -> str:
    radios, combos, has_gpu = [], [], set()
    for did, label, ext, sound in DISTROS:
        modes = [m for m, v in GPU.items() if (v, ext) in avail]
        if not modes:
            continue
        has_gpu.update(m for m in modes if m != "std")
        radios.append(
            f'<label class="radio"><input type="radio" name="distro" value="{did}" '
            f'data-modes="{" ".join(modes)}"><span>{e(label)}</span></label>')
        for mode in modes:
            f = avail[(GPU[mode], ext)]
            what = {"nvidia": "version NVIDIA", "vulkan": "version Vulkan", "std": "version standard"}[mode]
            if ext == ".AppImage":
                intro = "Ouvrez un terminal, collez la commande et appuyez sur Entrée."
            else:
                intro = ("Ouvrez un terminal (Konsole sous KDE), collez la commande avec "
                         "<kbd>Ctrl</kbd>+<kbd>Maj</kbd>+<kbd>V</kbd> et appuyez sur <kbd>Entrée</kbd>. "
                         "Saisissez ensuite votre mot de passe.")
            combos.append(
                f'<div class="combo" data-distro="{did}" data-gpu="{mode}">'
                f'<p class="combo-title">{e(label)}, {what}</p>'
                f'<p>{intro}</p>{code_block(linux_commands(did, sound, f))}'
                f'<p class="small">La même commande sert à réinstaller {APP_NAME} ou à changer de version. '
                f'Fichier{NB}: <a class="nowrap" href="{e(f.url)}">{e(f.nom)}</a>, {human(f.taille)}.</p></div>')
    if not radios:
        return ""
    vulkan = ""
    if "vulkan" in has_gpu:
        vulkan = (f'<details class="more" id="gpu-more"><summary>Autres options</summary>'
                  f'<label class="radio"><input type="radio" name="gpu" value="vulkan">'
                  f'<span>Carte AMD ou Intel (version Vulkan)</span></label>'
                  f'<p class="small">Choisissez aussi cette version si la version NVIDIA ne démarre pas.</p></details>')
    gpu_q = ""
    if "nvidia" in has_gpu:
        gpu_q = f"""
      <fieldset class="question" id="q-gpu">
        <legend><span class="num">2</span>Avez-vous une carte graphique NVIDIA{NB}?</legend>
        <div class="segmented">
          <label><input type="radio" name="gpu" value="nvidia"><span>Oui</span></label>
          <label><input type="radio" name="gpu" value="std"><span>Non</span></label>
        </div>
        <p class="small" id="gpu-hint">Vous ne savez pas{NB}? Répondez «{NB}Non{NB}»{NB}: cette version fonctionne sur tous les ordinateurs.</p>
        {vulkan}
      </fieldset>"""
    return f"""
    <div class="panel" id="p-linux" data-os="linux" role="tabpanel">
      <h3 class="panel-title">Linux</h3>
      <div class="questions">
        <fieldset class="question">
          <legend><span class="num">1</span>Votre distribution</legend>
          <div class="radios">{"".join(radios)}</div>
          <p class="small">Vous ne savez pas{NB}? Son nom figure dans les paramètres du système, rubrique «{NB}À propos{NB}».</p>
        </fieldset>{gpu_q}
      </div>
      <div class="question run">
        <h4><span class="num">{3 if gpu_q else 2}</span>Lancez l’installation</h4>
        {"".join(combos)}
      </div>
    </div>"""


def download_button(f: Fichier, label: str, big: bool = False) -> str:
    return (f'<a class="button{" button-lg" if big else ""}" href="{e(f.url)}">'
            f'{logo("download", 20)}<span>{e(label)}</span></a>')


def render_page(version: str, date: dt.date, avail: dict, listing: list, sums) -> str:
    tabs, panels, hero = [], [], []
    v = f"Version {e(version)}"

    # macOS
    arm, intel = avail.get(("macos-arm64", ".dmg")), avail.get(("macos-x64", ".dmg"))
    mac_main, mac_alt = (arm, intel) if arm else (intel, None)
    if mac_main:
        kind = "Apple Silicon" if mac_main is arm else "processeur Intel"
        tabs.append(("macos", "macOS", "apple"))
        alt = (f'<p class="small">Mac avec processeur Intel{NB}: <a href="{e(mac_alt.url)}">télécharger cette version</a> '
               f'({human(mac_alt.taille)}). Pour savoir quel Mac vous avez{NB}: menu Pomme › À propos de ce Mac.</p>'
               if mac_alt else "")
        panels.append(f"""
    <div class="panel" id="p-macos" data-os="macos" role="tabpanel">
      <h3 class="panel-title">macOS</h3>
      <div class="split">
        <div class="get">
          {download_button(mac_main, "Télécharger pour Mac")}
          <p class="small">{kind}, macOS 12 ou plus récent, {human(mac_main.taille)}.</p>{alt}
        </div>
        <ol class="steps">
          <li>Ouvrez le fichier téléchargé.</li>
          <li>Glissez {APP_NAME} dans le dossier Applications, puis ouvrez-le.</li>
          <li>Autorisez le micro et l’accessibilité quand macOS vous le demande. L’accessibilité permet à {APP_NAME} d’écrire dans vos autres logiciels.</li>
        </ol>
      </div>
    </div>""")
        hero.append(("macos", mac_main, "Télécharger pour Mac", f"{kind}, {human(mac_main.taille)}"))
        if mac_alt:
            hero.append(("macos-intel", mac_alt, "Télécharger pour Mac", f"processeur Intel, {human(mac_alt.taille)}"))

    # Windows
    exe, msi = avail.get(("windows-x64", "-setup.exe")), avail.get(("windows-x64", ".msi"))
    win = exe or msi
    if win:
        tabs.append(("windows", "Windows", "windows"))
        alt = (f'<p class="small">Pour installer sur plusieurs postes{NB}: <a href="{e(msi.url)}">paquet MSI</a> '
               f'({human(msi.taille)}).</p>' if exe and msi else "")
        panels.append(f"""
    <div class="panel" id="p-windows" data-os="windows" role="tabpanel">
      <h3 class="panel-title">Windows</h3>
      <div class="split">
        <div class="get">
          {download_button(win, "Télécharger pour Windows")}
          <p class="small">Windows 10 ou 11, {human(win.taille)}.</p>{alt}
        </div>
        <ol class="steps">
          <li>Ouvrez le fichier téléchargé et suivez les instructions.</li>
          <li>Si Windows affiche «{NB}Windows a protégé votre ordinateur{NB}», cliquez sur «{NB}Informations complémentaires{NB}», puis sur «{NB}Exécuter quand même{NB}».</li>
          <li>{APP_NAME} démarre. Son icône apparaît près de l’horloge, en bas à droite de l’écran.</li>
        </ol>
      </div>
    </div>""")
        hero.append(("windows", win, "Télécharger pour Windows", f"Windows 10 ou 11, {human(win.taille)}"))

    # Linux
    lp = linux_panel(avail)
    if lp:
        tabs.append(("linux", "Linux", "linux"))
        panels.append(lp)
        hero.append(("linux", None, "Installer sur Linux", "pour openSUSE, Fedora, Debian et Ubuntu"))

    tab_html = "".join(
        f'<button type="button" class="tab" role="tab" id="t-{oid}" data-os="{oid}" aria-selected="false" '
        f'aria-controls="p-{oid}">{logo(lg)}<span>{e(lbl)}</span></button>' for oid, lbl, lg in tabs)

    hero_html = "".join(
        f'<div class="hero-get" data-os="{oid}" hidden>'
        f'<a class="button button-lg" href="{e(f.url) if f else "#linux"}">{logo("download", 22)}<span>{e(lbl)}</span></a>'
        f'<p class="small">{v}, {sub}.</p></div>' for oid, f, lbl, sub in hero)

    files_html = "".join(
        f'<li><a href="{e(u)}">{e(n)}</a><span>{human(s)}</span></li>' for n, u, s in listing)
    sums_html = (f'<p class="small">Pour vérifier l’intégrité des fichiers{NB}: '
                 f'<a href="{e(sums)}">sommes de contrôle SHA-256</a>.</p>' if sums else "")

    date_txt = f"{date.day}{'er' if date.day == 1 else ''} {MOIS[date.month - 1]} {date.year}"
    return (TEMPLATE.replace("{{APP}}", e(APP_NAME)).replace("{{CMD}}", e(APP_CMD))
            .replace("{{VERSION}}", e(version)).replace("{{DATE}}", date_txt)
            .replace("{{HERO}}", hero_html).replace("{{TABS}}", tab_html).replace("{{PANELS}}", "".join(panels))
            .replace("{{FILES}}", files_html).replace("{{SUMS}}", sums_html).replace("{{SOURCE}}", e(SOURCE_URL)).replace("{{NBSP}}", NB))


TEMPLATE = r"""<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Télécharger {{APP}}</title>
<meta name="description" content="{{APP}}, la dictée vocale sur votre ordinateur. Téléchargement pour macOS, Windows et Linux.">
<meta name="color-scheme" content="light dark">
<link rel="icon" href="icon.png">
<style>
/* ---- Couleurs : clair par défaut, sombre si le système le demande ---- */
:root {
  --bg: #ffffff;
  --bg-soft: #f6f7f9;
  --text: #16181d;
  --muted: #5c6270;
  --line: #e4e6eb;
  --line-strong: #cdd1d9;
  --accent: #3b42c4;
  --accent-hover: #2f359f;
  --accent-text: #ffffff;
  --accent-soft: #eef0fd;
  --focus: #3b42c4;
  --ok: #1d7a45;
  --radius: 12px;
  --mono: ui-monospace, "SF Mono", Menlo, Consolas, "DejaVu Sans Mono", "Liberation Mono", monospace;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #111215;
    --bg-soft: #191b1f;
    --text: #ecedf0;
    --muted: #a1a6b2;
    --line: #2a2d34;
    --line-strong: #3b3f48;
    --accent: #a3a8ff;
    --accent-hover: #bcc0ff;
    --accent-text: #111215;
    --accent-soft: #23253c;
    --focus: #a3a8ff;
    --ok: #6fd49b;
  }
}

/* ---- Base ---- */
*, *::before, *::after { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; scroll-padding-top: 24px; }
@media (prefers-reduced-motion: no-preference) { html { scroll-behavior: smooth; } }
body {
  margin: 0; background: var(--bg); color: var(--text);
  font: 17px/1.6 system-ui, -apple-system, "Segoe UI", Roboto, "Noto Sans", Ubuntu, Cantarell, "Helvetica Neue", Arial, sans-serif;
  -webkit-font-smoothing: antialiased;
}
a { color: var(--accent); text-decoration-thickness: 1px; text-underline-offset: 3px; }
a:hover { color: var(--accent-hover); }
:focus-visible { outline: 2px solid var(--focus); outline-offset: 3px; border-radius: 6px; }
p { margin: 0 0 16px; }
h1, h2, h3, h4 { margin: 0; line-height: 1.2; letter-spacing: -0.015em; }
code, kbd, pre { font-family: var(--mono); }
kbd {
  font-size: 0.82em; padding: 1px 6px; border: 1px solid var(--line-strong); border-bottom-width: 2px;
  border-radius: 6px; background: var(--bg); white-space: nowrap;
}
.logo { flex: none; display: block; }
.small { font-size: 15px; color: var(--muted); }
.small:last-child { margin-bottom: 0; }
.nowrap { white-space: nowrap; }

.container { width: 100%; max-width: 1168px; margin: 0 auto; padding: 0 24px; }

/* ---- En-tête du site ---- */
.site-header { border-bottom: 1px solid var(--line); }
.site-header .container { display: flex; align-items: center; justify-content: space-between; height: 64px; gap: 16px; }
.brand { display: flex; align-items: center; gap: 10px; color: var(--text); text-decoration: none; font-weight: 650; font-size: 18px; }
.brand:hover { color: var(--text); }
.brand img { width: 32px; height: 32px; border-radius: 8px; }
.nav { display: flex; gap: 24px; font-size: 15px; }
.nav a { color: var(--muted); text-decoration: none; }
.nav a:hover { color: var(--text); }

/* ---- Accroche ---- */
.hero { padding: 96px 0 88px; }
.hero .container { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: center; gap: 48px 64px; }
.hero h1 { font-size: 56px; font-weight: 700; letter-spacing: -0.03em; }
.hero .lead { margin: 20px 0 0; max-width: 34em; font-size: 21px; line-height: 1.5; color: var(--muted); }
.hero-action { min-width: 320px; }
.hero-action .small { margin: 12px 0 0; }
.hero-action .others { display: inline-block; margin-top: 16px; font-size: 16px; }

/* ---- Boutons ---- */
.button {
  display: inline-flex; align-items: center; justify-content: center; gap: 10px;
  min-height: 48px; padding: 0 24px; border-radius: 10px;
  background: var(--accent); color: var(--accent-text); text-decoration: none;
  font-weight: 600; font-size: 17px; transition: background-color .15s ease;
}
.button:hover { background: var(--accent-hover); color: var(--accent-text); }
.button-lg { min-height: 60px; padding: 0 32px; font-size: 19px; border-radius: 12px; }

/* ---- Sections ---- */
.section { padding: 88px 0; border-top: 1px solid var(--line); }
.section h2 { font-size: 36px; font-weight: 700; letter-spacing: -0.025em; }
.section-intro { margin: 12px 0 0; max-width: 40em; color: var(--muted); }
.section-head { margin-bottom: 40px; }
.section-row { padding: 64px 0; }
.section-row .container { display: grid; grid-template-columns: minmax(0, 5fr) minmax(0, 7fr); gap: 16px 64px; align-items: baseline; }
.section-row p { margin: 0; font-size: 19px; color: var(--muted); max-width: 36em; }

/* ---- Onglets des systèmes ---- */
.tabs { display: inline-flex; gap: 4px; padding: 4px; border-radius: var(--radius); background: var(--bg-soft); border: 1px solid var(--line); }
.tab {
  appearance: none; border: 0; background: none; color: var(--muted); font: inherit; font-size: 16px; font-weight: 600;
  display: inline-flex; align-items: center; gap: 8px; padding: 8px 20px; border-radius: 8px; cursor: pointer;
  transition: color .15s ease, background-color .15s ease;
}
.tab { --cut: var(--bg-soft); }
.tab:hover { color: var(--text); }
.tab[aria-selected="true"] { --cut: var(--bg); background: var(--bg); color: var(--text); box-shadow: 0 1px 2px rgba(16, 18, 24, .08), 0 0 0 1px var(--line); }
.panels { margin-top: 40px; }
.panel-title { display: none; }

/* macOS, Windows : téléchargement à gauche, étapes à droite */
.split { display: grid; grid-template-columns: minmax(0, 5fr) minmax(0, 7fr); gap: 32px 64px; align-items: start; }
.get .small { margin: 12px 0 0; }
.steps { list-style: none; counter-reset: step; margin: 0; padding: 0; }
.steps li { counter-increment: step; position: relative; padding: 0 0 20px 48px; max-width: 36em; }
.steps li:last-child { padding-bottom: 0; }
.steps li::before {
  content: counter(step); position: absolute; left: 0; top: -1px;
  width: 30px; height: 30px; border-radius: 50%; border: 1px solid var(--line-strong);
  display: grid; place-items: center; font-size: 14px; font-weight: 600; color: var(--muted);
}

/* ---- Linux : questions puis commande ---- */
.questions { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 32px 64px; }
.question { border: 0; margin: 0; padding: 0; min-width: 0; }
.question legend, .question h4 { display: flex; align-items: center; gap: 12px; padding: 0; margin-bottom: 16px; font-size: 19px; font-weight: 650; }
.num {
  flex: none; width: 30px; height: 30px; border-radius: 50%; background: var(--accent-soft); color: var(--accent);
  display: grid; place-items: center; font-size: 14px; font-weight: 700;
}
.radios { display: grid; gap: 4px; margin-bottom: 12px; }
.radio { display: flex; align-items: center; gap: 12px; padding: 8px 12px; margin-left: -12px; border-radius: 8px; cursor: pointer; transition: background-color .15s ease; }
.radio:hover { background: var(--bg-soft); }
.radio input { width: 18px; height: 18px; margin: 0; accent-color: var(--accent); flex: none; }

.segmented { display: inline-grid; grid-template-columns: 1fr 1fr; padding: 4px; gap: 4px; border-radius: var(--radius); background: var(--bg-soft); border: 1px solid var(--line); margin-bottom: 16px; }
.segmented label { position: relative; cursor: pointer; }
.segmented input { position: absolute; opacity: 0; inset: 0; margin: 0; cursor: pointer; }
.segmented span { display: block; min-width: 96px; padding: 8px 20px; border-radius: 8px; text-align: center; font-weight: 600; color: var(--muted); transition: color .15s ease, background-color .15s ease; }
.segmented label:hover span { color: var(--text); }
.segmented input:checked + span { background: var(--bg); color: var(--text); box-shadow: 0 1px 2px rgba(16, 18, 24, .08), 0 0 0 1px var(--line); }
.segmented input:focus-visible + span { outline: 2px solid var(--focus); outline-offset: 2px; }

.more { margin-top: 8px; }
.more summary { cursor: pointer; color: var(--accent); font-size: 15px; width: max-content; list-style: none; }
.more summary::-webkit-details-marker { display: none; }
.more summary::after { content: "›"; display: inline-block; margin-left: 6px; transition: transform .15s ease; }
.more[open] summary::after { transform: rotate(90deg); }
.more[open] summary { margin-bottom: 8px; }

.run { margin-top: 48px; padding-top: 40px; border-top: 1px solid var(--line); }
.run p { max-width: 44em; }
.combo-title { display: none; }

/* Bloc de commande */
.code { display: flex; align-items: flex-start; gap: 16px; margin: 0 0 16px; padding: 12px 12px 12px 0; border: 1px solid var(--line); border-radius: var(--radius); background: var(--bg-soft); }
.code pre { flex: 1; min-width: 0; margin: 0; padding: 8px 0 8px 24px; overflow-x: auto; font-size: 14.5px; line-height: 1.7; }
.copy {
  flex: none; appearance: none; font: inherit; font-size: 14px; font-weight: 600;
  color: var(--text); background: var(--bg); border: 1px solid var(--line-strong); border-radius: 8px;
  padding: 4px 14px; cursor: pointer; transition: border-color .15s ease, color .15s ease;
}
.copy:hover { border-color: var(--accent); color: var(--accent); }
.copy.done { color: var(--ok); border-color: var(--ok); }

/* Une seule combinaison visible quand JavaScript est actif */
.js .panel[hidden], .js .combo:not(.on) { display: none; }
html:not(.js) .tabs, html:not(.js) .questions, html:not(.js) .run h4 { display: none; }
html:not(.js) .panel + .panel { margin-top: 64px; }
html:not(.js) .panel-title, html:not(.js) .combo-title { display: block; margin: 0 0 16px; font-size: 22px; }
html:not(.js) .combo-title { font-size: 17px; font-weight: 650; color: var(--text); margin-top: 32px; }
html:not(.js) .run { margin-top: 0; padding-top: 0; border-top: 0; }

/* ---- Premiers pas ---- */
.cards { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 24px; list-style: none; margin: 0; padding: 0; counter-reset: step; }
.cards li { counter-increment: step; padding: 24px; border: 1px solid var(--line); border-radius: var(--radius); }
.cards li::before { content: counter(step); display: grid; place-items: center; width: 30px; height: 30px; margin-bottom: 16px; border-radius: 50%; background: var(--accent-soft); color: var(--accent); font-size: 14px; font-weight: 700; }
.cards h3 { font-size: 19px; margin-bottom: 8px; }
.cards p { margin: 0; color: var(--muted); }
.tip { margin: 32px 0 0; color: var(--muted); }

/* ---- Repliables : dépannage, fichiers ---- */
.fold { border-top: 1px solid var(--line); }
.fold:last-of-type { border-bottom: 1px solid var(--line); }
.fold > summary {
  display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 24px 0;
  cursor: pointer; list-style: none; font-size: 22px; font-weight: 650; letter-spacing: -0.015em;
}
.fold > summary::-webkit-details-marker { display: none; }
.fold > summary::after {
  content: ""; flex: none; width: 10px; height: 10px; margin-right: 6px;
  border-right: 2px solid var(--muted); border-bottom: 2px solid var(--muted);
  transform: rotate(45deg); transition: transform .2s ease;
}
.fold[open] > summary::after { transform: rotate(-135deg); }
.fold-body { padding: 0 0 40px; }
.faq { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 40px 64px; }
.faq h3 { font-size: 17px; font-weight: 650; margin-bottom: 8px; }
.faq p { color: var(--muted); }
.faq .code { margin-top: 8px; }
.files { list-style: none; margin: 0 0 24px; padding: 0; font-size: 15px; columns: 2; column-gap: 64px; }
.files li { display: flex; justify-content: space-between; gap: 16px; padding: 8px 0; border-bottom: 1px solid var(--line); break-inside: avoid; }
.files a { overflow-wrap: anywhere; }
.files span { color: var(--muted); white-space: nowrap; }

/* ---- Pied de page ---- */
.site-footer { padding: 40px 0 56px; font-size: 15px; color: var(--muted); }
.site-footer .container { display: flex; justify-content: space-between; gap: 16px; flex-wrap: wrap; }
.site-footer p { margin: 0; }

/* ---- Écrans moyens et petits ---- */
@media (max-width: 900px) {
  .hero .container, .split, .questions, .faq, .section-row .container { grid-template-columns: minmax(0, 1fr); }
  .cards { grid-template-columns: minmax(0, 1fr); }
  .files { columns: 1; }
  .hero-action { min-width: 0; }
}
@media (max-width: 600px) {
  .container { padding: 0 16px; }
  .nav { display: none; }
  .hero { padding: 56px 0 64px; }
  .hero h1 { font-size: 40px; }
  .hero .lead { font-size: 19px; }
  .section { padding: 64px 0; }
  .section h2 { font-size: 30px; }
  .section-head { margin-bottom: 32px; }
  .tabs { display: flex; }
  .tab { flex: 1; justify-content: center; padding: 8px 8px; }
  .button, .hero .button-lg { width: 100%; }
  .code { gap: 8px; padding: 8px 8px 8px 0; }
  .code pre { padding: 8px 0 8px 16px; font-size: 13px; }
  .fold > summary { font-size: 20px; }
}
</style>
</head>
<body>
<header class="site-header">
  <div class="container">
    <a class="brand" href="#"><img src="icon.png" alt="" width="32" height="32"><span>{{APP}}</span></a>
    <nav class="nav" aria-label="Sur cette page">
      <a href="#installer">Installer</a>
      <a href="#premiers-pas">Premiers pas</a>
      <a href="#aide">Aide</a>
    </nav>
  </div>
</header>

<main>
  <section class="hero">
    <div class="container">
      <div>
        <h1>{{APP}}</h1>
        <p class="lead">La dictée vocale sur votre ordinateur. Maintenez un raccourci, parlez, relâchez{{NBSP}}: le texte s’écrit là où se trouve votre curseur.</p>
      </div>
      <div class="hero-action">
        {{HERO}}
        <div class="hero-get" data-os="none">
          <a class="button button-lg" href="#installer"><span>Télécharger {{APP}}</span></a>
          <p class="small" id="hero-note">Version {{VERSION}}, pour macOS, Windows et Linux.</p>
        </div>
        <a class="others" href="#installer">Autres systèmes</a>
      </div>
    </div>
  </section>

  <section class="section" id="installer">
    <div class="container">
      <div class="section-head">
        <h2>Installer {{APP}}</h2>
        <p class="section-intro">Choisissez le système de l’ordinateur à équiper.</p>
      </div>
      <div class="tabs" role="tablist" aria-label="Système">{{TABS}}</div>
      <div class="panels">{{PANELS}}
      </div>
    </div>
  </section>

  <section class="section" id="premiers-pas">
    <div class="container">
      <div class="section-head">
        <h2>Premiers pas</h2>
      </div>
      <ol class="cards">
        <li>
          <h3>Lancez {{APP}}</h3>
          <p>La première fois, il télécharge son modèle de reconnaissance vocale (environ 550{{NBSP}}Mo). Sous Linux, cliquez sur «{{NBSP}}Autoriser {{APP}}{{NBSP}}» s’il vous le propose.</p>
        </li>
        <li>
          <h3>Maintenez le raccourci</h3>
          <p><kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Espace</kbd>, ou <kbd>⌥ Option</kbd>+<kbd>Espace</kbd> sur Mac. Parlez normalement.</p>
        </li>
        <li>
          <h3>Relâchez</h3>
          <p>Le texte s’écrit là où se trouve votre curseur, dans n’importe quel logiciel.</p>
        </li>
      </ol>
      <p class="tip">Pour la ponctuation, dites «{{NBSP}}virgule{{NBSP}}», «{{NBSP}}point{{NBSP}}» ou «{{NBSP}}point à la ligne{{NBSP}}».</p>
    </div>
  </section>

  <section class="section section-row" id="mises-a-jour">
    <div class="container">
      <h2>Mises à jour</h2>
      <p>{{APP}} se met à jour automatiquement. Sous Linux, il vous demande simplement votre mot de passe.</p>
    </div>
  </section>

  <section class="section" id="aide">
    <div class="container">
      <div class="section-head">
        <h2>Aide</h2>
      </div>
      <details class="fold" id="depannage">
        <summary>Dépannage</summary>
        <div class="fold-body faq">
          <div>
            <h3>Le Mac refuse d’ouvrir {{APP}}</h3>
            <p>Ouvrez Réglages Système › Confidentialité et sécurité, puis cliquez sur «{{NBSP}}Ouvrir quand même{{NBSP}}».</p>
          </div>
          <div>
            <h3>Le raccourci ne fait rien sous Linux</h3>
            <p>Ouvrez les réglages de {{APP}}, rubrique Raccourci, et cliquez sur «{{NBSP}}Autoriser {{APP}}{{NBSP}}».</p>
            <p>Si cela ne suffit pas, créez un raccourci dans les paramètres du système{{NBSP}}: commande <code>{{CMD}} --toggle</code>, touches <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Espace</kbd>. Un appui démarre la dictée, un second l’arrête.</p>
          </div>
          <div>
            <h3>Le micro ne capte rien sous Linux</h3>
            <p>Vérifiez le micro choisi dans les paramètres du son. Ensuite, relancez la commande d’installation{{NBSP}}: elle installe aussi ce dont le son a besoin.</p>
          </div>
          <div>
            <h3>La version NVIDIA ne démarre pas</h3>
            <p>Le pilote NVIDIA doit être installé. Cette commande doit afficher le nom de votre carte{{NBSP}}:</p>
            <div class="code"><pre><code>nvidia-smi</code></pre><button type="button" class="copy" aria-label="Copier la commande">Copier</button></div>
            <p>Sinon, installez la version Vulkan{{NBSP}}: elle se trouve dans «{{NBSP}}Autres options{{NBSP}}».</p>
          </div>
          <div>
            <h3>L’AppImage ne démarre pas</h3>
            <p>Installez le paquet <code>libfuse2</code> avec le gestionnaire de logiciels de votre distribution (sous Ubuntu 24.04{{NBSP}}: <code>libfuse2t64</code>).</p>
          </div>
          <div>
            <h3>Envoyer un rapport au support</h3>
            <p>Sous Linux, cette commande relance {{APP}} et note ce qui se passe. Reproduisez le problème, quittez {{APP}}, puis envoyez le fichier <code>{{CMD}}.log</code> de votre dossier personnel.</p>
            <div class="code"><pre><code>pkill -x {{CMD}}; {{CMD}} 2&gt;&amp;1 | tee ~/{{CMD}}.log</code></pre><button type="button" class="copy" aria-label="Copier la commande">Copier</button></div>
          </div>
        </div>
      </details>
      <details class="fold" id="fichiers">
        <summary>Tous les fichiers de la version {{VERSION}}</summary>
        <div class="fold-body">
          <ul class="files">{{FILES}}</ul>
          {{SUMS}}
        </div>
      </details>
    </div>
  </section>
</main>

<footer class="site-footer">
  <div class="container">
    <p>{{APP}} {{VERSION}}, publiée le {{DATE}}.</p>
    <p>Logiciel libre de RD Agency. <a href="{{SOURCE}}">Code source</a></p>
  </div>
</footer>

<script>
(() => {
  const d = document;
  d.documentElement.classList.add("js");
  const $ = (sel, el = d) => [...el.querySelectorAll(sel)];
  const one = (sel) => d.querySelector(sel);

  // Anciens liens et variantes d'écriture : #linux/opensuse-leap-15/nvidia, #linux/fedora/cuda…
  const ALIAS = {
    "opensuse-leap-15": "leap15", "leap-15": "leap15", "opensuse-leap-16": "leap16", "leap-16": "leap16",
    "opensuse-tumbleweed": "leap16", "tumbleweed": "leap16", ubuntu: "debian",
    cuda: "nvidia", standard: "std", cpu: "std",
  };
  const tabs = $(".tab"), panels = $(".panel"), combos = $(".combo");
  const distros = $('input[name="distro"]'), gpus = $('input[name="gpu"]');
  const state = { os: null, distro: null, gpu: null };

  const modesOf = (id) => {
    const r = distros.find((i) => i.value === id);
    return r ? r.dataset.modes.split(" ") : [];
  };
  const hasOs = (os) => tabs.some((t) => t.dataset.os === os);

  function render() {
    tabs.forEach((t) => t.setAttribute("aria-selected", String(t.dataset.os === state.os)));
    panels.forEach((p) => { p.hidden = p.dataset.os !== state.os; });
    const modes = modesOf(state.distro);
    const gpu = modes.includes(state.gpu) ? state.gpu : "std";
    distros.forEach((i) => { i.checked = i.value === state.distro; });
    gpus.forEach((i) => { i.checked = i.value === gpu; });
    const q = one("#q-gpu");
    if (q) q.hidden = modes.length < 2;
    const more = one("#gpu-more");
    if (more && gpu === "vulkan") more.open = true;
    combos.forEach((c) => c.classList.toggle("on", c.dataset.distro === state.distro && c.dataset.gpu === gpu));
  }

  function save() {
    const h = state.os === "linux" ? `#linux/${state.distro}/${state.gpu}` : `#${state.os}`;
    history.replaceState(null, "", h);
  }

  // Lit #os ou #linux/distribution/carte. Renvoie vrai si l'ancre désigne un système.
  function fromHash() {
    const [os, distro, gpu] = location.hash.slice(1).split("/").map((s) => ALIAS[s] || s);
    if (!hasOs(os)) return false;
    state.os = os;
    if (modesOf(distro).length) state.distro = distro;
    if (gpu in { nvidia: 1, std: 1, vulkan: 1 }) state.gpu = gpu;
    return true;
  }

  // Détection du système, de la distribution et de la carte graphique.
  const ua = navigator.userAgent, uad = navigator.userAgentData;
  const plat = ((uad && uad.platform) || navigator.platform || "") + " " + ua;
  const mobile = /Android|iPhone|iPad|iPod/i.test(ua) || (uad && uad.mobile);
  const detected = mobile ? null : /Win/i.test(plat) ? "windows" : /Mac/i.test(plat) ? "macos"
    : /Linux|X11|CrOS/i.test(plat) ? "linux" : null;
  state.distro = /Fedora/i.test(ua) ? "fedora" : /Ubuntu|Debian|Mint/i.test(ua) ? "debian" : "leap15";
  if (!modesOf(state.distro).length && distros[0]) state.distro = distros[0].value;
  state.gpu = "std";
  let seen = "";
  try {
    const gl = d.createElement("canvas").getContext("webgl");
    const ext = gl && gl.getExtension("WEBGL_debug_renderer_info");
    const r = gl ? gl.getParameter(ext ? ext.UNMASKED_RENDERER_WEBGL : gl.RENDERER) : "";
    if (/nvidia|geforce|rtx|quadro/i.test(r)) { state.gpu = "nvidia"; seen = "NVIDIA"; }
    else if (/radeon|amd|intel.*arc/i.test(r) && gpus.some((i) => i.value === "vulkan")) { state.gpu = "vulkan"; seen = "AMD ou Intel"; }
  } catch (err) { /* WebGL indisponible : rien à détecter */ }
  state.os = hasOs(detected) ? detected : tabs[0] && tabs[0].dataset.os;
  if (seen && one("#gpu-hint")) one("#gpu-hint").textContent = `Carte ${seen} détectée sur cet ordinateur.`;

  // Accroche : gros bouton pour le système détecté.
  function showHero(key) {
    const target = $(".hero-get").find((h) => h.dataset.os === key);
    if (!target) return;
    $(".hero-get").forEach((h) => { h.hidden = h !== target; });
  }
  if (detected) showHero(detected);
  else if (mobile) one("#hero-note").textContent = "Ouvrez cette page sur l’ordinateur à équiper.";
  if (detected === "macos" && uad && uad.getHighEntropyValues) {
    uad.getHighEntropyValues(["architecture"]).then((v) => { if (v.architecture === "x86") showHero("macos-intel"); }, () => {});
  }

  const linked = fromHash();
  render();
  if (linked) one("#installer").scrollIntoView();

  window.addEventListener("hashchange", () => {
    if (location.hash === "#fichiers") one("#fichiers").open = true;
    if (location.hash === "#depannage") one("#depannage").open = true;
    if (fromHash()) { render(); one("#installer").scrollIntoView(); }
  });

  function pick(key, value) { state[key] = value; render(); save(); }
  tabs.forEach((t) => t.addEventListener("click", () => pick("os", t.dataset.os)));
  tabs.forEach((t, i) => t.addEventListener("keydown", (ev) => {
    const step = { ArrowRight: 1, ArrowLeft: -1 }[ev.key];
    if (!step) return;
    const next = tabs[(i + step + tabs.length) % tabs.length];
    next.focus();
    pick("os", next.dataset.os);
  }));
  distros.forEach((i) => i.addEventListener("change", () => pick("distro", i.value)));
  gpus.forEach((i) => i.addEventListener("change", () => pick("gpu", i.value)));

  // Boutons Copier.
  $(".copy").forEach((b) => b.addEventListener("click", async () => {
    const text = b.parentNode.querySelector("pre").textContent;
    let ok = false;
    try { await navigator.clipboard.writeText(text); ok = true; } catch (err) {
      const sel = getSelection(), range = d.createRange();
      range.selectNodeContents(b.parentNode.querySelector("pre"));
      sel.removeAllRanges(); sel.addRange(range);
      try { ok = d.execCommand("copy"); } catch (err2) { ok = false; }
    }
    b.textContent = ok ? "Copié" : "Ctrl+C pour copier";
    b.classList.toggle("done", ok);
    setTimeout(() => { b.textContent = "Copier"; b.classList.remove("done"); }, 2000);
  }));
})();
</script>
</body>
</html>
"""

if __name__ == "__main__":
    main()

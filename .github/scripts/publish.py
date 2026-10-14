"""Prépare la publication S3 d'une release Murmure.

Entrée : le dossier des fichiers d'une version (installeurs + .sig).
Sortie (--out) : latest.json (mise à jour auto Tauri), latest-map.txt (copies « latest »
sans numéro de version), index.html (page de téléchargement) et icon.png.
"""
import argparse
import datetime as dt
import html
import json
import re
import shutil
from pathlib import Path

# Variante → libellé, dans l'ordre d'affichage.
VARIANTS = [
    ("macos-arm64", "macOS", "Apple Silicon (M1 et plus)"),
    ("macos-x64", "macOS", "Intel"),
    ("windows-x64", "Windows", "10 et 11, 64 bits"),
    ("linux-x64", "Linux", "x86_64"),
    ("linux-x64-cuda", "Linux", "x86_64 avec carte NVIDIA (CUDA 12)"),
]
# Extension → (libellé, ordre).
FORMATS = {
    ".dmg": ("Image disque .dmg", 0),
    "-setup.exe": ("Installeur .exe", 0),
    ".msi": ("Paquet .msi", 1),
    ".AppImage": ("AppImage (toutes distributions)", 0),
    ".deb": ("Paquet .deb (Debian, Ubuntu)", 1),
    ".rpm": ("Paquet .rpm (Fedora, openSUSE)", 2),
}
# Clés du manifeste Tauri v2 → (variante, extension). La variante CUDA est exclue :
# la clé linux-x86_64 la ferait « mettre à jour » vers la version sans GPU.
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


def human(n: int) -> str:
    return f"{n / 1_000_000:.0f}\u00a0Mo" if n >= 1_000_000 else f"{n / 1000:.0f}\u00a0ko"


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

    # Manifeste de mise à jour.
    platforms = {}
    for key, (variant, ext) in UPDATER.items():
        n = name(variant, ext)
        if n in files and f"{n}.sig" in files:
            platforms[key] = {"signature": files[f"{n}.sig"].read_text().strip(), "url": f"{rel}/{n}"}
    manifest = {
        "version": version,
        "notes": f"Murmure {version}",
        "pub_date": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    (out / "latest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False))

    # Copies stables + page.
    latest_map, sections = [], []
    for variant, os_name, detail in VARIANTS:
        items = []
        for ext, (label, order) in FORMATS.items():
            n = name(variant, ext)
            if n not in files:
                continue
            stable = f"Murmure-{variant}{ext}"
            latest_map.append(f"{n} {stable}")
            items.append((order, label, f"{a.base}/latest/{stable}", files[n].stat().st_size))
        if items:
            items.sort()
            sections.append((variant, os_name, detail, items))
    (out / "latest-map.txt").write_text("\n".join(latest_map) + "\n")

    cards = []
    for variant, os_name, detail, items in sections:
        primary = items[0]
        others = "".join(
            f'<a class="alt" href="{html.escape(u)}">{html.escape(l)} <span>{human(s)}</span></a>'
            for _, l, u, s in items[1:]
        )
        cards.append(
            f'<section class="card" data-variant="{variant}">'
            f'<div class="os">{html.escape(os_name)}</div><div class="detail">{html.escape(detail)}</div>'
            f'<a class="btn" href="{html.escape(primary[2])}">Télécharger <span>{html.escape(primary[1])} · {human(primary[3])}</span></a>'
            f'{others}</section>'
        )

    page = TEMPLATE.replace("{{VERSION}}", html.escape(version)).replace("{{CARDS}}", "\n".join(cards))
    page = page.replace("{{SUMS}}", f"{rel}/SHA256SUMS.txt").replace("{{DATE}}", dt.date.today().strftime("%d/%m/%Y"))
    (out / "index.html").write_text(page)
    shutil.copy(Path(__file__).resolve().parents[2] / "src-tauri/icons/128x128@2x.png", out / "icon.png")


TEMPLATE = """<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Télécharger Murmure</title>
<link rel="icon" href="icon.png">
<style>
:root { --bg:#f7f7f8; --card:#fff; --text:#1c1c21; --muted:#6b6b76; --line:#e6e6ea; --accent:#5b5fe0; --accent-text:#fff; }
@media (prefers-color-scheme: dark) { :root { --bg:#141417; --card:#1c1c21; --text:#ececf1; --muted:#9a9aa6; --line:#2c2c33; --accent:#8d91ff; --accent-text:#111; } }
* { box-sizing:border-box; }
body { margin:0; background:var(--bg); color:var(--text); font:15px/1.5 -apple-system, "SF Pro Text", Inter, system-ui, sans-serif; -webkit-font-smoothing:antialiased; }
main { max-width:880px; margin:0 auto; padding:64px 16px 48px; }
header { text-align:center; margin-bottom:40px; }
header img { width:88px; height:88px; }
h1 { font-size:32px; letter-spacing:-.02em; margin:12px 0 6px; }
.lead { color:var(--muted); max-width:520px; margin:0 auto; }
.ver { display:inline-block; margin-top:14px; font-size:13px; color:var(--muted); border:1px solid var(--line); border-radius:99px; padding:2px 12px; }
.grid { display:grid; grid-template-columns:repeat(auto-fit, minmax(240px, 1fr)); gap:14px; }
.card { background:var(--card); border:1px solid var(--line); border-radius:16px; padding:20px; display:flex; flex-direction:column; gap:6px; }
.card.mine { border-color:var(--accent); box-shadow:0 0 0 3px color-mix(in srgb, var(--accent) 18%, transparent); order:-1; }
.os { font-weight:600; font-size:17px; }
.detail { color:var(--muted); font-size:13px; margin-bottom:10px; }
.btn { display:flex; flex-direction:column; text-decoration:none; background:var(--accent); color:var(--accent-text); border-radius:10px; padding:10px 14px; font-weight:600; }
.btn span { font-weight:400; font-size:12px; opacity:.85; }
.alt { color:var(--muted); font-size:13px; text-decoration:none; padding:4px 2px; }
.alt:hover { color:var(--text); }
.alt span { opacity:.7; }
.help { margin-top:36px; display:grid; gap:10px; color:var(--muted); font-size:14px; }
.help b { color:var(--text); font-weight:600; }
footer { margin-top:32px; text-align:center; font-size:12px; color:var(--muted); }
footer a { color:inherit; }
</style>
</head>
<body>
<main>
  <header>
    <img src="icon.png" alt="">
    <h1>Murmure</h1>
    <p class="lead">Dictée vocale partout. Maintenez un raccourci, parlez, relâchez : le texte apparaît là où se trouve votre curseur. Hors ligne, ou via votre serveur.</p>
    <span class="ver">Version {{VERSION}} · {{DATE}}</span>
  </header>
  <div class="grid">
{{CARDS}}
  </div>
  <div class="help">
    <div><b>Premier lancement.</b> Murmure télécharge seul le modèle Whisper (environ 550 Mo), puis tout fonctionne hors ligne. Les mises à jour s'installent depuis l'app.</div>
    <div><b>Mac.</b> Autorisez le micro et l'Accessibilité quand Murmure les demande (pour coller le texte dans les autres apps).</div>
    <div><b>Windows.</b> Si SmartScreen s'affiche : « Informations complémentaires » puis « Exécuter quand même ».</div>
    <div><b>Linux.</b> AppImage : rendez le fichier exécutable (<code>chmod +x</code>). Sous Wayland, liez un raccourci du bureau à <code>murmure --toggle</code>.</div>
  </div>
  <footer>RD Agency · <a href="{{SUMS}}">Sommes de contrôle SHA-256</a></footer>
</main>
<script>
(function () {
  var ua = navigator.userAgent, p = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  var v = /Win/i.test(p + ua) ? "windows-x64" : /Linux|X11/i.test(p + ua) && !/Android/i.test(ua) ? "linux-x64" : /Mac/i.test(p + ua) ? "macos-arm64" : null;
  function mark(id) { var el = document.querySelector('[data-variant="' + id + '"]'); if (el) el.classList.add("mine"); }
  if (v === "macos-arm64" && navigator.userAgentData && navigator.userAgentData.getHighEntropyValues) {
    navigator.userAgentData.getHighEntropyValues(["architecture"]).then(function (h) { mark(h.architecture === "x86" ? "macos-x64" : "macos-arm64"); }, function () { mark(v); });
  } else if (v) { mark(v); }
})();
</script>
</body>
</html>
"""

if __name__ == "__main__":
    main()

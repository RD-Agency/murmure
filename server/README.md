# Murmure STT — serveur de transcription auto-hébergé

Serveur de transcription pour l'app **Murmure** (moteur « Serveur »), compatible avec l'API OpenAI
`POST /v1/audio/transcriptions`. Moteur : [faster-whisper](https://github.com/SYSTRAN/faster-whisper)
avec **Whisper large-v3-turbo** (CTranslate2), GPU NVIDIA ou CPU.

Pensé pour des données confidentielles (cabinets d'avocats) : l'audio reste sur votre machine, n'est conservé que
le temps de la requête (fichiers temporaires en mémoire/tmpfs dans les conteneurs), et **les journaux ne contiennent jamais ni le texte transcrit,
ni le prompt, ni l'audio** (uniquement tailles, durées, latences et un identifiant non réversible de clé).

---

## Déploiement en 5 minutes (serveur GPU)

Prérequis : Linux x86_64 avec GPU NVIDIA (pilote **≥ 570**), Docker + Compose v2,
[NVIDIA Container Toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html),
un nom DNS (ex. `stt.exemple.fr`) pointant vers la machine, ports 80/443 ouverts.

```bash
# 1. Récupérer le code
git clone <dépôt murmure> && cd murmure/server

# 2. Configurer
cp .env.example .env
docker compose build stt-gpu            # ou stt-cpu sans GPU
docker compose run --rm --no-deps stt-gpu murmure-server gen-key
#   → mrm_4Kx…   à coller dans MURMURE_API_KEYS (une clé par poste, séparées par des virgules)
#   Renseigner aussi MURMURE_DOMAIN=stt.exemple.fr et ACME_EMAIL dans .env

# 3. Lancer (serveur + HTTPS automatique Let's Encrypt via Caddy)
docker compose --profile gpu --profile proxy up -d

# 4. Vérifier (le 1er démarrage télécharge le modèle, ~1,6 Go, mis en cache dans le volume « models »)
docker compose logs -f stt-gpu          # attendre « modèle prêt »
curl https://stt.exemple.fr/health      # {"status":"ok","model":"large-v3-turbo","device":"cuda","compute_type":"float16",…}
```

Vérifiez que `/health` indique bien `"device": "cuda"` : sans GPU visible, le serveur se rabat sur le CPU.

Test de bout en bout :

```bash
curl https://stt.exemple.fr/v1/audio/transcriptions \
  -H "Authorization: Bearer mrm_…" \
  -F file=@dictee.wav -F model=whisper-1 -F language=fr
# {"text":"Bonjour, ceci est un test de dictée vocale."}
```

Sans GPU : `docker compose --profile cpu --profile proxy up -d` (image `Dockerfile.cpu`, int8).

## Configuration dans Murmure

Réglages → Moteur → **Serveur** :

| Champ | Valeur |
|---|---|
| Endpoint | `https://stt.exemple.fr/v1/audio/transcriptions` |
| Clé API | la clé `mrm_…` générée (stockée dans le trousseau du système, jamais dans `settings.json`) |
| Modèle | `whisper-large-v3-turbo` (n'importe quel nom est accepté : il est mappé sur le modèle chargé) |

Le bouton « Tester » envoie une seconde de silence et doit répondre avec la latence.

**Alternative sans auto-hébergement** — Murmure peut aussi pointer directement vers un fournisseur
compatible (l'audio quitte alors votre infrastructure : à proscrire pour des dossiers clients confidentiels
sans accord contractuel adapté) :

| Fournisseur | Endpoint | Modèle |
|---|---|---|
| OpenAI | `https://api.openai.com/v1/audio/transcriptions` | `whisper-1` (ou `gpt-4o-transcribe`) |
| Groq | `https://api.groq.com/openai/v1/audio/transcriptions` | `whisper-large-v3-turbo` |

## API

| Route | Auth | Description |
|---|---|---|
| `POST /v1/audio/transcriptions` | Bearer | Multipart compatible OpenAI (voir ci-dessous) |
| `GET /v1/models` | Bearer | Modèle chargé + alias (`whisper-1`, `whisper-large-v3-turbo`) |
| `GET /health` | aucune | `status`, `model`, `device`, `compute_type`, `version` |

Champs de `POST /v1/audio/transcriptions` :

- `file` (requis) : tout format courant (WAV, MP3, M4A/AAC, OGG/Opus, WebM, FLAC…), décodé par PyAV et
  rééchantillonné en mono 16 kHz. Taille max : `MURMURE_MAX_UPLOAD_MB` (25 Mo par défaut).
- `model` : ignoré (alias), facultatif.
- `language` : code ISO 639-1 (`fr`, `en`…) ; absent, vide ou `auto` = détection automatique.
  Préciser la langue est plus rapide et plus fiable (la détection coûte une passe supplémentaire).
- `prompt` : vocabulaire / style (noms propres, jargon), transmis comme `initial_prompt`.
- `temperature` : 0 à 1. À 0 (défaut), repli automatique de température comme chez OpenAI.
- `response_format` : `json` (défaut, `{"text": …}`), `text`, `verbose_json` (segments, langue, durée), `srt`, `vtt`.

Erreurs au format OpenAI : `{"error": {"message", "type", "param", "code"}}` — 400 (paramètre / audio
invalide), 401 (`missing_api_key`, `invalid_api_key`), 413 (`file_too_large`), 429 (`rate_limit_exceeded`,
avec `Retry-After`), 503 (`server_busy` si la file d'attente dépasse `MURMURE_QUEUE_TIMEOUT_S`).

Compatible avec le SDK `openai` :

```python
from openai import OpenAI
client = OpenAI(base_url="https://stt.exemple.fr/v1", api_key="mrm_…")
print(client.audio.transcriptions.create(model="whisper-1", file=open("dictee.wav", "rb"), language="fr").text)
```

## Variables d'environnement

Voir `.env.example` (toutes préfixées `MURMURE_`) :

| Variable | Défaut | Rôle |
|---|---|---|
| `API_KEYS` | — | Clés séparées par des virgules |
| `API_KEYS_FILE` | — | Fichier de clés (une par ligne, `#` = commentaire) |
| `ALLOW_NO_AUTH` | `false` | Sans clé, le serveur **refuse de démarrer** sauf si `1` |
| `MODEL` | `large-v3-turbo` | Nom faster-whisper, dépôt HF ou chemin local CTranslate2 |
| `DEVICE` | `auto` | `auto`, `cuda`, `cpu` |
| `COMPUTE_TYPE` | `auto` | `float16` (GPU) / `int8` (CPU) ; `int8_float16` pour GPU à peu de VRAM |
| `BEAM_SIZE` | `5` | 1 = plus rapide, un peu moins précis |
| `VAD_FILTER` | `true` | Filtre de silence Silero (évite les hallucinations sur les blancs) |
| `MAX_CONCURRENCY` | `1` | Transcriptions simultanées (sémaphore + threadpool) |
| `QUEUE_TIMEOUT_S` | `120` | Attente max en file avant 503 |
| `MAX_UPLOAD_MB` | `25` | Taille max du fichier |
| `RATE_LIMIT_PER_MINUTE` | `0` | Requêtes/min/clé, en mémoire (0 = désactivé) |
| `LOG_LEVEL` | `info` | |

`HF_HOME` (défaut `/models` dans les images) contrôle le cache des modèles.

## Sizing indicatif

Le modèle large-v3-turbo pèse ~1,6 Go (float16). Une dictée Murmure typique dure 3 à 20 s.

| Matériel | VRAM / RAM utilisée | Latence pour ~5 s de dictée | Utilisateurs simultanés confortables |
|---|---|---|---|
| RTX 3060 12 Go / RTX 4060 Ti | ~2,5–3 Go (float16) | ~0,3–0,6 s | 10–20 dicteurs (requêtes courtes, file d'attente) |
| RTX 4090 / L4 / A10 | ~3 Go | ~0,2–0,4 s | 30+ (monter `MAX_CONCURRENCY` à 2–4) |
| GPU 4 Go | `int8_float16`, ~1,5 Go | ~0,5–1 s | quelques-uns |
| CPU 8+ cœurs (int8) | ~2 Go de RAM | ≈ temps réel (1×) | 1–3 |

Valeurs GPU indicatives (ordres de grandeur faster-whisper), à valider sur votre matériel.
**Mesuré** (Apple M5 Pro, CPU int8, beam 5, audio FR de 3,3 s, langue fixée) : **~4,3 s** en natif macOS,
**~3,0 s** dans le conteneur `Dockerfile.cpu` (linux/arm64) ; ~4,1 s en natif avec beam 1.
Sur Mac, le moteur **local** de Murmure (whisper.cpp + Metal) est plus rapide : le serveur vise un GPU NVIDIA.

Chaque requête simultanée supplémentaire (`MAX_CONCURRENCY`) consomme un peu de VRAM en plus
(activations + beam) ; laisser 1 tant que la file d'attente reste courte.

## Sécurité

- **Ne jamais exposer le port 8000 sans TLS.** Le compose ne publie le serveur que sur `127.0.0.1` ;
  l'accès externe passe par Caddy (`deploy/Caddyfile`, certificat Let's Encrypt automatique, HSTS).
  Caddy installé hors Docker : remplacer `stt:8000` par `127.0.0.1:8000` dans le Caddyfile.
- Idéalement, restreindre en plus l'accès aux IP du cabinet ou à un VPN (WireGuard/Tailscale) :
  bloc `remote_ip` commenté dans le Caddyfile, ou pare-feu.
- Une clé par poste/utilisateur, pour pouvoir en révoquer une (retirer de `.env`, puis
  `docker compose up -d` pour redémarrer). Les clés sont comparées en temps constant ; les journaux
  n'en contiennent qu'une empreinte tronquée (`key_id`).
- `.env` contient les clés : `chmod 600 .env`, ne jamais le committer. Pour plus de rigueur, utiliser
  `MURMURE_API_KEYS_FILE` avec un secret Docker monté en lecture seule.
- Conteneur durci : utilisateur non-root, système de fichiers en lecture seule, `cap_drop: ALL`,
  `no-new-privileges`, `/tmp` en tmpfs (les fichiers reçus ne touchent pas le disque persistant).
- Pas de documentation interactive (`/docs`, `/openapi.json`) exposée.
- Aucun appel sortant hors téléchargement initial du modèle depuis Hugging Face (télémétrie HF désactivée).
  Une fois le modèle en cache, `HF_HUB_OFFLINE=1` dans `.env` coupe tout accès réseau sortant.

## Mise à jour

```bash
cd murmure/server && git pull
docker compose --profile gpu --profile proxy build --pull
docker compose --profile gpu --profile proxy up -d
docker image prune -f
```

Le cache du modèle (volume `models`) est conservé. Changer de modèle : modifier `MURMURE_MODEL`
puis `up -d` (téléchargement au démarrage).

Versions épinglées : `faster-whisper 1.2.1`, `ctranslate2 4.8.2` (wheels compilées CUDA 12.8 /
cuDNN 9.10), `av < 19` (PyAV 19 casse le décodage de faster-whisper 1.2.1), image
`nvidia/cuda:12.8.1-cudnn-runtime-ubuntu22.04` avec cuDNN aligné sur 9.10.2.21. Toutes les dépendances
Python sont figées dans `uv.lock`. Avant de monter ctranslate2, vérifier la version CUDA/cuDNN de ses wheels.

## Développement

```bash
uv sync                                   # Python ≥ 3.11
uv run pytest                             # moteur factice injecté : aucun modèle requis
MURMURE_API_KEYS=$(uv run murmure-server gen-key) uv run murmure-server serve --port 8000
```

Arborescence :

```
src/murmure_server/
  app.py        routes, middlewares (taille max, journal), sémaphore + threadpool
  engine.py     interface Engine + FasterWhisperEngine (chargé une fois au démarrage)
  auth.py       clés Bearer (temps constant), gen-key
  config.py     variables MURMURE_*
  formats.py    json / text / verbose_json / srt / vtt
  ratelimit.py  seau à jetons par clé, en mémoire
  logs.py       journaux JSON sans contenu
  cli.py        murmure-server serve | gen-key
tests/          pytest (auth, formats, erreurs, limites, décodage)
deploy/Caddyfile
```

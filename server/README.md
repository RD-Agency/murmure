# Serveur de transcription Murmure

Serveur de transcription auto-hébergé pour le moteur « Serveur » de Murmure. Il expose l'API OpenAI
`POST /v1/audio/transcriptions` et transcrit avec [faster-whisper](https://github.com/SYSTRAN/faster-whisper)
et le modèle Whisper large-v3-turbo, sur GPU NVIDIA ou sur processeur.

Il est conçu pour des données confidentielles. L'audio n'est conservé que le temps de la requête, dans des
fichiers temporaires en mémoire (tmpfs) dans les conteneurs. Les journaux ne contiennent jamais l'audio,
le texte transcrit ni le prompt : seulement des tailles, des durées, des latences et une empreinte
tronquée de la clé utilisée.

## Déploiement sur un serveur GPU

Il faut une machine Linux x86_64 avec une carte NVIDIA (pilote 570 ou plus récent), Docker avec Compose v2,
le [NVIDIA Container Toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html),
un nom de domaine (par exemple `stt.exemple.fr`) qui pointe vers la machine, et les ports 80 et 443
ouverts.

```bash
git clone https://github.com/RD-Agency/murmure.git
cd murmure/server

cp .env.example .env
chmod 600 .env
docker compose build stt-gpu
docker compose run --rm --no-deps stt-gpu murmure-server gen-key
```

La dernière commande affiche une clé `mrm_…`. Copiez-la dans `MURMURE_API_KEYS` du fichier `.env` (une clé
par poste, séparées par des virgules), puis renseignez `MURMURE_DOMAIN` et `ACME_EMAIL`. Lancez ensuite le
serveur et le proxy HTTPS (Caddy, certificat Let's Encrypt automatique) :

```bash
docker compose --profile gpu --profile proxy up -d
docker compose logs -f stt-gpu          # attendre « modèle prêt »
curl https://stt.exemple.fr/health
```

Au premier démarrage, le modèle (environ 1,6 Go) est téléchargé, puis gardé dans le volume `models`.
`/health` doit indiquer `"device":"cuda"` : sans GPU visible, le serveur se rabat sur le processeur.

Pour vérifier une transcription de bout en bout :

```bash
curl https://stt.exemple.fr/v1/audio/transcriptions \
  -H "Authorization: Bearer mrm_…" \
  -F file=@dictee.wav -F model=whisper-1 -F language=fr
```

Sans GPU, utilisez le profil `cpu` (image `Dockerfile.cpu`, calcul en int8) :
`docker compose --profile cpu --profile proxy up -d`.

## Configurer Murmure

Dans Réglages, choisissez le moteur « Serveur » et renseignez :

- l'adresse : `https://stt.exemple.fr/v1/audio/transcriptions` (l'adresse de base `https://stt.exemple.fr`
  suffit, Murmure la complète) ;
- la clé API `mrm_…`, rangée dans le trousseau du système ;
- le modèle : `whisper-large-v3-turbo`. Le serveur accepte n'importe quel nom et utilise le modèle chargé.

Le bouton « Tester » envoie une seconde de silence et affiche la latence.

Murmure peut aussi utiliser directement un service tiers compatible, mais l'audio quitte alors votre
infrastructure :

| Service | Adresse | Modèle |
|---|---|---|
| OpenAI | `https://api.openai.com/v1/audio/transcriptions` | `whisper-1` ou `gpt-4o-transcribe` |
| Groq | `https://api.groq.com/openai/v1/audio/transcriptions` | `whisper-large-v3-turbo` |

## API

| Route | Authentification | Rôle |
|---|---|---|
| `POST /v1/audio/transcriptions` | Bearer | transcription, format multipart d'OpenAI |
| `GET /v1/models` | Bearer | modèle chargé et ses alias (`whisper-1`, `whisper-large-v3-turbo`) |
| `GET /health` | aucune | `status`, `model`, `device`, `compute_type`, `version` |

Champs de `POST /v1/audio/transcriptions` :

- `file` (obligatoire) : WAV, MP3, M4A, OGG, WebM, FLAC et autres formats courants, décodés par PyAV puis
  convertis en mono 16 kHz. Taille maximale : `MURMURE_MAX_UPLOAD_MB`, 25 Mo par défaut.
- `model` : facultatif, ignoré.
- `language` : code ISO 639-1 (`fr`, `en`…). Absent, vide ou `auto` : détection automatique, plus lente
  et moins fiable que la langue fixée.
- `prompt` : vocabulaire ou style, transmis à Whisper comme prompt initial.
- `temperature` : de 0 à 1. À 0 (par défaut), la température remonte automatiquement en cas d'échec, comme
  chez OpenAI.
- `response_format` : `json` (par défaut, `{"text": …}`), `text`, `verbose_json`, `srt` ou `vtt`.

Les erreurs suivent le format d'OpenAI, `{"error": {"message", "type", "param", "code"}}` : 400 pour un
paramètre ou un audio invalide, 401 (`missing_api_key`, `invalid_api_key`), 413 (`file_too_large`), 429
(`rate_limit_exceeded`, avec l'en-tête `Retry-After`) et 503 (`server_busy`, quand l'attente dans la file
dépasse `MURMURE_QUEUE_TIMEOUT_S`).

Le SDK `openai` fonctionne tel quel :

```python
from openai import OpenAI

client = OpenAI(base_url="https://stt.exemple.fr/v1", api_key="mrm_…")
with open("dictee.wav", "rb") as f:
    print(client.audio.transcriptions.create(model="whisper-1", file=f, language="fr").text)
```

## Configuration

Toutes les variables sont préfixées par `MURMURE_` ; `.env.example` les présente avec des valeurs
conseillées.

| Variable | Défaut | Rôle |
|---|---|---|
| `MURMURE_API_KEYS` | — | clés acceptées, séparées par des virgules |
| `MURMURE_API_KEYS_FILE` | — | fichier de clés, une par ligne (`#` pour un commentaire) |
| `MURMURE_ALLOW_NO_AUTH` | `false` | sans clé, le serveur refuse de démarrer, sauf avec `1` |
| `MURMURE_HOST`, `MURMURE_PORT` | `0.0.0.0`, `8000` | adresse et port d'écoute |
| `MURMURE_MODEL` | `large-v3-turbo` | nom faster-whisper, dépôt Hugging Face ou chemin d'un modèle CTranslate2 |
| `MURMURE_MODEL_DIR` | — | dossier de cache des modèles (sinon `HF_HOME`) |
| `MURMURE_DEVICE` | `auto` | `auto`, `cuda` ou `cpu` |
| `MURMURE_DEVICE_INDEX` | `0` | numéro du GPU |
| `MURMURE_COMPUTE_TYPE` | `auto` | float16 sur GPU, int8 sur processeur ; `int8_float16` pour un GPU avec peu de mémoire |
| `MURMURE_BEAM_SIZE` | `5` | 1 est plus rapide, un peu moins précis |
| `MURMURE_VAD_FILTER` | `true` | filtre de silence Silero, qui évite les textes inventés sur les blancs |
| `MURMURE_CPU_THREADS` | `0` | threads sur processeur (0 : choix automatique) |
| `MURMURE_MAX_CONCURRENCY` | `1` | transcriptions simultanées |
| `MURMURE_QUEUE_TIMEOUT_S` | `120` | attente maximale dans la file avant une erreur 503 |
| `MURMURE_MAX_UPLOAD_MB` | `25` | taille maximale d'un fichier |
| `MURMURE_RATE_LIMIT_PER_MINUTE` | `0` | requêtes par minute et par clé (0 : pas de limite ; 60 dans `.env.example`) |
| `MURMURE_LOG_LEVEL` | `info` | `debug`, `info`, `warning` ou `error` |

Dans les images Docker, `HF_HOME` vaut `/models`.

## Dimensionnement

Le modèle large-v3-turbo occupe environ 1,6 Go en float16. Une dictée dure en général de 3 à 20 secondes.

| Matériel | Mémoire utilisée | Latence pour 5 s de dictée | Dicteurs simultanés |
|---|---|---|---|
| RTX 3060 12 Go, RTX 4060 Ti | 2,5 à 3 Go | 0,3 à 0,6 s | 10 à 20 |
| RTX 4090, L4, A10 | environ 3 Go | 0,2 à 0,4 s | plus de 30, avec `MAX_CONCURRENCY` entre 2 et 4 |
| GPU de 4 Go | environ 1,5 Go en `int8_float16` | 0,5 à 1 s | quelques-uns |
| Processeur 8 cœurs ou plus | environ 2 Go de RAM | à peu près la durée de l'audio | 1 à 3 |

Ces valeurs GPU sont des ordres de grandeur, à vérifier sur votre matériel. Mesure réelle sur Apple M5 Pro,
processeur, int8, faisceau de 5, audio français de 3,3 s avec la langue fixée : 4,3 s en natif, 3,0 s dans
le conteneur `Dockerfile.cpu` (linux/arm64). Sur Mac, le moteur local de Murmure (Metal) reste plus rapide.

Chaque transcription simultanée supplémentaire consomme un peu plus de mémoire GPU. Gardez
`MAX_CONCURRENCY` à 1 tant que la file d'attente reste courte.

## Sécurité

- N'exposez jamais le port 8000 sans TLS. Le fichier Compose ne le publie que sur `127.0.0.1` ; l'accès
  extérieur passe par Caddy (`deploy/Caddyfile`, Let's Encrypt et HSTS). Si Caddy est installé hors
  Docker, remplacez `stt:8000` par `127.0.0.1:8000` dans le Caddyfile.
- Restreignez si possible l'accès à votre réseau ou à un VPN, avec le bloc `remote_ip` commenté du
  Caddyfile ou un pare-feu.
- Donnez une clé par poste, pour pouvoir en révoquer une : retirez-la de `.env`, puis relancez
  `docker compose --profile gpu --profile proxy up -d`. Les clés sont comparées en temps constant.
- `.env` contient les clés : droits 600, jamais dans Git. Pour aller plus loin, utilisez
  `MURMURE_API_KEYS_FILE` avec un secret Docker monté en lecture seule.
- Le conteneur tourne sans root, avec un système de fichiers en lecture seule, `cap_drop: ALL`,
  `no-new-privileges` et `/tmp` en tmpfs.
- La documentation interactive (`/docs`, `/openapi.json`) n'est pas exposée.
- Le seul accès sortant est le téléchargement initial du modèle depuis Hugging Face, télémétrie
  désactivée. Une fois le modèle en cache, `HF_HUB_OFFLINE=1` dans `.env` coupe tout accès sortant.

## Mise à jour

```bash
cd murmure/server && git pull
docker compose --profile gpu --profile proxy build --pull
docker compose --profile gpu --profile proxy up -d
docker image prune -f
```

Le modèle en cache (volume `models`) est conservé. Pour changer de modèle, modifiez `MURMURE_MODEL` puis
relancez `up -d` : le nouveau modèle est téléchargé au démarrage.

Les versions sont épinglées : faster-whisper 1.2.1 et ctranslate2 4.8.2, dont les wheels sont compilées
pour CUDA 12.8 et cuDNN 9.10 ; PyAV inférieur à 19, car la version 19 casse le décodage de faster-whisper
1.2.1 ; image `nvidia/cuda:12.8.1-cudnn-runtime-ubuntu22.04` avec cuDNN 9.10.2.21. Toutes les dépendances
Python sont figées dans `uv.lock`. Avant de monter ctranslate2, vérifiez la version de CUDA et de cuDNN de
ses wheels.

## Développement

```bash
uv sync
uv run pytest                     # moteur factice : aucun modèle n'est téléchargé
MURMURE_API_KEYS=$(uv run murmure-server gen-key) uv run murmure-server serve --port 8000
```

```
src/murmure_server/
  app.py        routes, middlewares (taille maximale, journal), sémaphore et pool de threads
  engine.py     interface Engine et FasterWhisperEngine, chargé une fois au démarrage
  auth.py       clés Bearer, comparaison en temps constant, génération de clés
  config.py     variables MURMURE_*
  formats.py    formats de réponse json, text, verbose_json, srt, vtt
  ratelimit.py  limitation par clé (seau à jetons, en mémoire)
  logs.py       journaux JSON sans contenu
  cli.py        murmure-server serve | gen-key
tests/          tests pytest : authentification, formats, erreurs, limites, décodage
deploy/         Caddyfile
```

## Licence

Le serveur fait partie de Murmure et suit la même licence, l'AGPL v3 ou ultérieure (voir `LICENSE` à la
racine du dépôt). Si vous modifiez le serveur et le proposez à des utilisateurs par le réseau, vous devez
leur donner accès au code source de votre version.

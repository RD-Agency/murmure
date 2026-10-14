# syntax=docker/dockerfile:1.7
# Image GPU (NVIDIA). ctranslate2 4.8.x est compilé contre CUDA 12.8 + cuDNN 9 :
# les wheels officielles sont construites avec cuda 12.8 et libcudnn9-cuda-12 9.10.2.21
# (cf. python/tools/prepare_build_environment_linux.sh du dépôt CTranslate2, tag v4.8.2).
# L'image runtime CUDA 12.8.1 fournit cuBLAS 12.8 mais cuDNN 9.8 : on aligne cuDNN sur 9.10.2.21.
# Pilote NVIDIA requis sur l'hôte : >= 570 (CUDA 12.8) + NVIDIA Container Toolkit.
ARG CUDA_IMAGE=nvidia/cuda:12.8.1-cudnn-runtime-ubuntu22.04
ARG CUDNN_VERSION=9.10.2.21-1

FROM ghcr.io/astral-sh/uv:0.11.21 AS uv

# --- Étape de construction : Python géré par uv + dépendances figées (uv.lock) ---------------
FROM ${CUDA_IMAGE} AS builder
COPY --from=uv /uv /usr/local/bin/uv
ENV UV_COMPILE_BYTECODE=1 \
    UV_LINK_MODE=copy \
    UV_PYTHON_INSTALL_DIR=/opt/python \
    UV_PYTHON_PREFERENCE=only-managed \
    UV_PROJECT_ENVIRONMENT=/opt/venv
WORKDIR /src
RUN uv python install 3.12
COPY pyproject.toml uv.lock README.md ./
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-dev --no-install-project --python 3.12
COPY src ./src
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-dev --no-editable --python 3.12

# --- Image finale ------------------------------------------------------------------------------
FROM ${CUDA_IMAGE}
ARG CUDNN_VERSION
RUN apt-get update \
    && apt-get install -y --no-install-recommends --allow-change-held-packages \
        ca-certificates "libcudnn9-cuda-12=${CUDNN_VERSION}" \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home murmure \
    && mkdir -p /models && chown murmure:murmure /models
COPY --from=builder /opt/python /opt/python
COPY --from=builder /opt/venv /opt/venv
ENV PATH=/opt/venv/bin:$PATH \
    PYTHONUNBUFFERED=1 \
    HF_HOME=/models \
    HF_HUB_DISABLE_TELEMETRY=1 \
    MURMURE_HOST=0.0.0.0 \
    MURMURE_PORT=8000 \
    MURMURE_DEVICE=auto \
    MURMURE_COMPUTE_TYPE=auto
USER murmure
WORKDIR /home/murmure
VOLUME ["/models"]
EXPOSE 8000
# Premier démarrage : téléchargement du modèle (~1,6 Go), d'où le long start-period.
HEALTHCHECK --interval=30s --timeout=5s --start-period=600s --retries=3 \
    CMD ["python", "-c", "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8000/health', timeout=4)"]
CMD ["murmure-server", "serve"]

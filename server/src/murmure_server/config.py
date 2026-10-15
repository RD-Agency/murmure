# SPDX-License-Identifier: AGPL-3.0-or-later
"""Configuration du serveur, lue depuis les variables d'environnement `MURMURE_*` (ou un `.env`)."""

from __future__ import annotations

from pathlib import Path
from typing import Literal

from pydantic import Field
from pydantic_settings import BaseSettings, SettingsConfigDict


class ConfigError(RuntimeError):
    """Configuration invalide : le serveur refuse de démarrer."""


class Settings(BaseSettings):
    model_config = SettingsConfigDict(
        env_prefix="MURMURE_",
        env_file=".env",
        env_file_encoding="utf-8",
        extra="ignore",
    )

    # --- Réseau -------------------------------------------------------------
    host: str = "0.0.0.0"
    port: int = Field(default=8000, ge=1, le=65535)

    # --- Authentification ---------------------------------------------------
    # Liste de clés séparées par des virgules.
    api_keys: str = ""
    # Fichier optionnel : une clé par ligne, lignes vides et commentaires `#` ignorés.
    api_keys_file: Path | None = None
    # Garde-fou : sans clé, le serveur ne démarre que si ce drapeau est explicitement activé.
    allow_no_auth: bool = False

    # --- Moteur faster-whisper ---------------------------------------------
    # Nom accepté par faster-whisper (« large-v3-turbo » → mobiuslabsgmbh/faster-whisper-large-v3-turbo),
    # identifiant de dépôt Hugging Face, ou chemin local vers un modèle CTranslate2.
    model: str = "large-v3-turbo"
    device: Literal["auto", "cuda", "cpu"] = "auto"
    # « auto » = float16 sur GPU, int8 sur CPU. Sinon toute valeur CTranslate2 (int8_float16, float32…).
    compute_type: str = "auto"
    device_index: int = 0
    beam_size: int = Field(default=5, ge=1, le=20)
    vad_filter: bool = True
    # 0 = laisser CTranslate2 choisir.
    cpu_threads: int = Field(default=0, ge=0)
    # Dossier de cache des modèles (sinon cache Hugging Face standard, cf. HF_HOME).
    model_dir: Path | None = None

    # --- Charge -------------------------------------------------------------
    # Nombre de transcriptions simultanées (1 suffit généralement sur un GPU grand public).
    max_concurrency: int = Field(default=1, ge=1, le=32)
    # Attente maximale dans la file avant de répondre 503.
    queue_timeout_s: float = Field(default=120.0, gt=0)
    max_upload_mb: float = Field(default=25.0, gt=0)
    # Requêtes par minute et par clé ; 0 = désactivé.
    rate_limit_per_minute: int = Field(default=0, ge=0)

    # --- Journalisation -----------------------------------------------------
    log_level: Literal["debug", "info", "warning", "error"] = "info"

    @property
    def max_upload_bytes(self) -> int:
        return int(self.max_upload_mb * 1024 * 1024)

    def load_api_keys(self) -> list[str]:
        """Rassemble les clés (variable + fichier), dédoublonnées, sans espaces parasites."""
        keys: list[str] = [k.strip() for k in self.api_keys.split(",") if k.strip()]
        if self.api_keys_file is not None:
            try:
                content = self.api_keys_file.read_text(encoding="utf-8")
            except OSError as exc:
                raise ConfigError(f"Impossible de lire MURMURE_API_KEYS_FILE ({self.api_keys_file}) : {exc}") from exc
            for line in content.splitlines():
                line = line.split("#", 1)[0].strip()
                if line:
                    keys.append(line)
        unique = list(dict.fromkeys(keys))
        for key in unique:
            if len(key) < 16:
                raise ConfigError("Clé API trop courte (16 caractères minimum) : générez-en une avec `murmure-server gen-key`.")
        return unique

    def validate_auth(self) -> list[str]:
        """Renvoie les clés ; lève ConfigError si aucune clé et que l'accès libre n'est pas explicitement autorisé."""
        keys = self.load_api_keys()
        if not keys and not self.allow_no_auth:
            raise ConfigError(
                "Aucune clé API configurée (MURMURE_API_KEYS ou MURMURE_API_KEYS_FILE). "
                "Générez une clé avec `murmure-server gen-key`, ou définissez MURMURE_ALLOW_NO_AUTH=1 "
                "pour un usage strictement local (déconseillé)."
            )
        return keys

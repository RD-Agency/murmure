"""Journalisation structurée (une ligne JSON par événement).

Confidentialité : on ne journalise JAMAIS le texte transcrit, le prompt, ni l'audio.
Seules des métadonnées (tailles, durées, latences, identifiant non réversible de clé) sont écrites.
"""

from __future__ import annotations

import json
import logging
import sys
from datetime import UTC, datetime

# Attributs standard d'un LogRecord, exclus des champs « extra ».
_STANDARD_ATTRS = frozenset(vars(logging.makeLogRecord({})).keys()) | {"message", "asctime", "taskName", "color_message"}


class JSONFormatter(logging.Formatter):
    def format(self, record: logging.LogRecord) -> str:
        payload: dict[str, object] = {
            "ts": datetime.fromtimestamp(record.created, UTC).isoformat(timespec="milliseconds"),
            "level": record.levelname.lower(),
            "logger": record.name,
            "msg": record.getMessage(),
        }
        for key, value in vars(record).items():
            if key not in _STANDARD_ATTRS and not key.startswith("_"):
                payload[key] = value
        if record.exc_info and record.exc_info[0] is not None:
            # Type d'exception seulement : un message d'erreur pourrait embarquer des données.
            payload["exc_type"] = record.exc_info[0].__name__
        return json.dumps(payload, ensure_ascii=False, default=str)


def setup_logging(level: str = "info") -> None:
    handler = logging.StreamHandler(sys.stdout)
    handler.setFormatter(JSONFormatter())
    root = logging.getLogger()
    root.handlers[:] = [handler]
    root.setLevel(level.upper())
    # Bibliothèques tierces : au minimum WARNING (faster-whisper journalise la langue détectée, etc.).
    for noisy in ("faster_whisper", "httpx", "huggingface_hub", "urllib3"):
        logging.getLogger(noisy).setLevel(logging.WARNING)
    # Le journal d'accès d'uvicorn est remplacé par notre propre événement « request ».
    logging.getLogger("uvicorn.access").disabled = True
    for name in ("uvicorn", "uvicorn.error"):
        lg = logging.getLogger(name)
        lg.handlers[:] = []
        lg.propagate = True

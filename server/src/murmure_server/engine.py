# SPDX-License-Identifier: AGPL-3.0-or-later
"""Moteurs de transcription.

`Engine` est l'interface utilisée par l'API ; `FasterWhisperEngine` est l'implémentation réelle.
Les tests injectent un moteur factice respectant le même protocole.
"""

from __future__ import annotations

import io
import logging
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Protocol, runtime_checkable

if TYPE_CHECKING:
    import numpy as np

    from murmure_server.config import Settings

logger = logging.getLogger("murmure.engine")

SAMPLE_RATE = 16_000
# Repli de température « à la OpenAI » quand temperature=0 : on remonte si le décodage échoue.
DEFAULT_TEMPERATURE_FALLBACK: tuple[float, ...] = (0.0, 0.2, 0.4, 0.6, 0.8, 1.0)


class AudioDecodeError(ValueError):
    """Le fichier reçu n'est pas un audio décodable."""


class InvalidLanguageError(ValueError):
    """Code de langue inconnu du modèle."""


@dataclass(slots=True)
class Segment:
    id: int
    seek: int
    start: float
    end: float
    text: str
    tokens: list[int]
    temperature: float
    avg_logprob: float
    compression_ratio: float
    no_speech_prob: float


@dataclass(slots=True)
class TranscriptionResult:
    text: str
    language: str
    duration: float
    segments: list[Segment] = field(default_factory=list)


@runtime_checkable
class Engine(Protocol):
    """Interface d'un moteur. `transcribe` est bloquant : l'API l'exécute dans un thread."""

    model_name: str
    device: str
    compute_type: str

    def transcribe(
        self,
        audio: bytes,
        *,
        language: str | None,
        prompt: str | None,
        temperature: float,
    ) -> TranscriptionResult: ...


def resolve_device(device: str) -> str:
    """Résout `auto` en `cuda` si un GPU CUDA est visible par CTranslate2, sinon `cpu`."""
    if device != "auto":
        return device
    import ctranslate2

    try:
        return "cuda" if ctranslate2.get_cuda_device_count() > 0 else "cpu"
    except Exception:  # pilote absent, bibliothèque CUDA introuvable…
        return "cpu"


def resolve_compute_type(compute_type: str, device: str) -> str:
    if compute_type != "auto":
        return compute_type
    return "float16" if device == "cuda" else "int8"


def decode(audio: bytes) -> np.ndarray:
    """Décode n'importe quel format courant (WAV, MP3, M4A, OGG/Opus, WebM, FLAC…) en mono 16 kHz float32, via PyAV."""
    from faster_whisper.audio import decode_audio

    try:
        samples = decode_audio(io.BytesIO(audio), sampling_rate=SAMPLE_RATE)
    except Exception as exc:  # PyAV lève des erreurs variées selon le conteneur
        raise AudioDecodeError("Fichier audio illisible ou format non pris en charge.") from exc
    if samples.size == 0:
        raise AudioDecodeError("Le fichier audio ne contient aucun échantillon.")
    return samples


class FasterWhisperEngine:
    """Modèle faster-whisper chargé une seule fois, partagé entre les requêtes."""

    def __init__(self, settings: Settings) -> None:
        from faster_whisper import WhisperModel

        self.model_name = settings.model
        self.device = resolve_device(settings.device)
        self.compute_type = resolve_compute_type(settings.compute_type, self.device)
        self.beam_size = settings.beam_size
        self.vad_filter = settings.vad_filter

        logger.info(
            "chargement du modèle",
            extra={"model": self.model_name, "device": self.device, "compute_type": self.compute_type},
        )
        self._model = WhisperModel(
            self.model_name,
            device=self.device,
            device_index=settings.device_index,
            compute_type=self.compute_type,
            cpu_threads=settings.cpu_threads,
            # Autant de « workers » que de transcriptions simultanées autorisées.
            num_workers=settings.max_concurrency,
            download_root=str(settings.model_dir) if settings.model_dir else None,
        )
        self._languages: frozenset[str] = frozenset(self._model.supported_languages)

    def transcribe(
        self,
        audio: bytes,
        *,
        language: str | None,
        prompt: str | None,
        temperature: float,
    ) -> TranscriptionResult:
        if language is not None and language not in self._languages:
            raise InvalidLanguageError(f"Langue non prise en charge : {language!r}.")

        samples = decode(audio)
        temperatures: float | tuple[float, ...] = (
            DEFAULT_TEMPERATURE_FALLBACK if temperature == 0 else temperature
        )
        segments_iter, info = self._model.transcribe(
            samples,
            language=language,
            task="transcribe",
            beam_size=self.beam_size,
            temperature=temperatures,
            initial_prompt=prompt or None,
            vad_filter=self.vad_filter,
        )
        # Le générateur fait le vrai travail : on le consomme ici, dans le thread de travail.
        segments = [
            Segment(
                id=i,
                seek=s.seek,
                start=round(s.start, 3),
                end=round(s.end, 3),
                text=s.text,
                tokens=list(s.tokens),
                temperature=s.temperature if s.temperature is not None else 0.0,
                avg_logprob=s.avg_logprob,
                compression_ratio=s.compression_ratio,
                no_speech_prob=s.no_speech_prob,
            )
            for i, s in enumerate(segments_iter)
        ]
        text = "".join(s.text for s in segments).strip()
        return TranscriptionResult(
            text=text,
            language=info.language,
            duration=round(info.duration, 3),
            segments=segments,
        )

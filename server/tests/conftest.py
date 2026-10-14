"""Fixtures : moteur factice injecté, application de test, audio WAV synthétique."""

from __future__ import annotations

import io
import struct
import wave
from collections.abc import Callable, Iterator
from dataclasses import dataclass, field

import pytest
from fastapi.testclient import TestClient

from murmure_server.app import create_app
from murmure_server.config import Settings
from murmure_server.engine import AudioDecodeError, InvalidLanguageError, Segment, TranscriptionResult

API_KEY = "mrm_test_cle_valide_0123456789abcdef"
OTHER_KEY = "mrm_autre_cle_valide_9876543210fedcba"


@dataclass
class FakeEngine:
    """Moteur factice : ne charge aucun modèle, enregistre les appels reçus."""

    model_name: str = "large-v3-turbo"
    device: str = "cpu"
    compute_type: str = "int8"
    calls: list[dict] = field(default_factory=list)

    def transcribe(self, audio: bytes, *, language, prompt, temperature) -> TranscriptionResult:
        self.calls.append({"size": len(audio), "language": language, "prompt": prompt, "temperature": temperature})
        if not audio.startswith(b"RIFF"):
            raise AudioDecodeError("Fichier audio illisible ou format non pris en charge.")
        if language not in (None, "fr", "en"):
            raise InvalidLanguageError(f"Langue non prise en charge : {language!r}.")
        segments = [
            Segment(0, 0, 0.0, 1.5, " Bonjour, ceci est un test", [1, 2], 0.0, -0.2, 1.1, 0.01),
            Segment(1, 150, 1.5, 3.25, " de dictée vocale.", [3, 4], 0.0, -0.3, 1.0, 0.02),
        ]
        return TranscriptionResult(
            text="Bonjour, ceci est un test de dictée vocale.",
            language=language or "fr",
            duration=3.25,
            segments=segments,
        )


def make_wav(seconds: float = 1.0, rate: int = 16_000) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(struct.pack("<h", 0) * int(seconds * rate))
    return buf.getvalue()


def make_settings(**overrides) -> Settings:
    base = {"api_keys": f"{API_KEY},{OTHER_KEY}", "max_upload_mb": 1}
    base.update(overrides)
    return Settings(_env_file=None, **base)


@pytest.fixture
def engine() -> FakeEngine:
    return FakeEngine()


@pytest.fixture
def app_factory(engine: FakeEngine) -> Iterator[Callable[..., TestClient]]:
    clients: list[TestClient] = []

    def factory(**overrides) -> TestClient:
        client = TestClient(create_app(make_settings(**overrides), engine=engine))
        client.__enter__()  # déclenche le lifespan
        clients.append(client)
        return client

    yield factory
    for c in clients:
        c.__exit__(None, None, None)


@pytest.fixture
def client(app_factory) -> TestClient:
    return app_factory()


@pytest.fixture
def auth() -> dict[str, str]:
    return {"Authorization": f"Bearer {API_KEY}"}


@pytest.fixture
def wav() -> bytes:
    return make_wav()

"""Décodage audio réel via PyAV (sans modèle) : protège contre une régression de dépendance."""

from __future__ import annotations

import pytest

from murmure_server.engine import SAMPLE_RATE, AudioDecodeError, decode

from .conftest import make_wav


def test_decode_wav_16k():
    samples = decode(make_wav(seconds=1.0))
    assert samples.dtype.name == "float32"
    assert abs(samples.shape[0] - SAMPLE_RATE) < 200


def test_decode_reechantillonne():
    samples = decode(make_wav(seconds=1.0, rate=44_100))
    assert abs(samples.shape[0] - SAMPLE_RATE) < 200


def test_decode_illisible():
    with pytest.raises(AudioDecodeError):
        decode(b"ceci n'est pas de l'audio")

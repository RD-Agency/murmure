# SPDX-License-Identifier: AGPL-3.0-or-later
"""Endpoint de transcription : formats de réponse, paramètres, erreurs, limites."""

from __future__ import annotations

import json
import logging

import pytest

from .conftest import make_wav


def _post(client, auth, wav, **data):
    data.setdefault("model", "whisper-1")
    return client.post("/v1/audio/transcriptions", headers=auth, files={"file": ("a.wav", wav, "audio/wav")}, data=data)


def test_json_par_defaut(client, auth, wav):
    resp = _post(client, auth, wav)
    assert resp.status_code == 200
    assert resp.headers["content-type"].startswith("application/json")
    assert resp.json() == {"text": "Bonjour, ceci est un test de dictée vocale."}
    assert resp.headers["x-request-id"]


def test_text(client, auth, wav):
    resp = _post(client, auth, wav, response_format="text")
    assert resp.headers["content-type"].startswith("text/plain")
    assert resp.text == "Bonjour, ceci est un test de dictée vocale."


def test_verbose_json(client, auth, wav):
    body = _post(client, auth, wav, response_format="verbose_json", language="fr").json()
    assert body["task"] == "transcribe"
    assert body["language"] == "french"
    assert body["duration"] == 3.25
    assert body["text"].startswith("Bonjour")
    assert len(body["segments"]) == 2
    seg = body["segments"][1]
    assert {"id", "seek", "start", "end", "text", "tokens", "temperature",
            "avg_logprob", "compression_ratio", "no_speech_prob"} <= set(seg)
    assert seg["start"] == 1.5 and seg["end"] == 3.25


def test_srt(client, auth, wav):
    resp = _post(client, auth, wav, response_format="srt")
    assert resp.headers["content-type"].startswith("text/plain")
    assert resp.text == (
        "1\n00:00:00,000 --> 00:00:01,500\nBonjour, ceci est un test\n\n"
        "2\n00:00:01,500 --> 00:00:03,250\nde dictée vocale.\n"
    )


def test_vtt(client, auth, wav):
    resp = _post(client, auth, wav, response_format="vtt")
    assert resp.headers["content-type"].startswith("text/vtt")
    assert resp.text.startswith("WEBVTT\n\n00:00:00.000 --> 00:00:01.500\nBonjour, ceci est un test\n")
    assert "00:00:01.500 --> 00:00:03.250\nde dictée vocale." in resp.text


def test_parametres_transmis_au_moteur(client, auth, wav, engine):
    _post(client, auth, wav, language="fr-FR", prompt="Maître Dupont, Cabinet Martin", temperature="0.4")
    call = engine.calls[-1]
    assert call == {"size": len(wav), "language": "fr", "prompt": "Maître Dupont, Cabinet Martin", "temperature": 0.4}


@pytest.mark.parametrize("lang", ["", "auto", "AUTO"])
def test_langue_auto(client, auth, wav, engine, lang):
    _post(client, auth, wav, language=lang)
    assert engine.calls[-1]["language"] is None


@pytest.mark.parametrize("model", ["whisper-1", "whisper-large-v3-turbo", "n-importe-quoi"])
def test_alias_de_modele(client, auth, wav, model):
    assert _post(client, auth, wav, model=model).status_code == 200


def test_model_facultatif(client, auth, wav):
    resp = client.post("/v1/audio/transcriptions", headers=auth, files={"file": ("a.wav", wav, "audio/wav")})
    assert resp.status_code == 200


def _err(resp, status, code, param=None):
    assert resp.status_code == status, resp.text
    err = resp.json()["error"]
    assert err["code"] == code and err["message"] and err["type"]
    if param:
        assert err["param"] == param


def test_fichier_manquant(client, auth):
    resp = client.post("/v1/audio/transcriptions", headers=auth, data={"model": "whisper-1"})
    _err(resp, 400, "missing_file", "file")


def test_fichier_vide(client, auth):
    _err(_post(client, auth, b""), 400, "empty_file", "file")


def test_audio_illisible(client, auth):
    _err(_post(client, auth, b"pas un fichier audio"), 400, "invalid_audio", "file")


def test_format_inconnu(client, auth, wav):
    _err(_post(client, auth, wav, response_format="xml"), 400, "invalid_value", "response_format")


@pytest.mark.parametrize("t", ["-0.1", "1.5", "chaud"])
def test_temperature_invalide(client, auth, wav, t):
    _err(_post(client, auth, wav, temperature=t), 400, "invalid_value", "temperature")


def test_langue_inconnue(client, auth, wav):
    _err(_post(client, auth, wav, language="xx"), 400, "invalid_language", "language")


def test_fichier_trop_gros(client, auth, engine):
    # max_upload_mb=1 dans les fixtures
    big = make_wav(seconds=40)  # ~1,28 Mo
    _err(_post(client, auth, big), 413, "file_too_large", "file")
    assert engine.calls == []


def test_corps_http_trop_gros_rejete_avant_parsing(client, auth):
    resp = client.post(
        "/v1/audio/transcriptions",
        headers={**auth, "content-type": "multipart/form-data; boundary=x"},
        content=b"0" * (3 * 1024 * 1024),
    )
    _err(resp, 413, "file_too_large")


def test_route_inconnue_format_openai(client, auth):
    _err(client.get("/v1/inexistant", headers=auth), 404, "not_found")


def test_methode_non_autorisee(client, auth):
    _err(client.get("/v1/audio/transcriptions", headers=auth), 405, "method_not_allowed")


def test_rate_limit(app_factory, auth, wav):
    client = app_factory(rate_limit_per_minute=2)
    assert _post(client, auth, wav).status_code == 200
    assert _post(client, auth, wav).status_code == 200
    resp = _post(client, auth, wav)
    _err(resp, 429, "rate_limit_exceeded")
    assert int(resp.headers["retry-after"]) >= 1


def test_logs_sans_contenu(client, auth, wav, caplog):
    """Les journaux ne contiennent ni le texte transcrit ni le prompt."""
    from murmure_server.logs import JSONFormatter

    fmt = JSONFormatter()
    with caplog.at_level(logging.INFO, logger="murmure.api"):
        _post(client, auth, wav, prompt="Secret Maître Dupont")
    lines = [fmt.format(r) for r in caplog.records if r.name == "murmure.api"]
    assert lines
    event = json.loads(lines[-1])
    assert event["msg"] == "request" and event["status"] == 200
    assert event["audio_bytes"] == len(wav) and event["audio_duration_s"] == 3.25
    assert "latency_ms" in event and "processing_ms" in event and len(event["key_id"]) == 12
    joined = "\n".join(lines)
    assert "Bonjour" not in joined and "Dupont" not in joined and "mrm_" not in joined

"""Authentification : absence de clé, mauvaise clé, bonne clé, refus de démarrer sans clé."""

from __future__ import annotations

import pytest

from murmure_server.app import create_app
from murmure_server.auth import KeyStore, generate_key, key_id
from murmure_server.cli import main
from murmure_server.config import ConfigError

from .conftest import API_KEY, OTHER_KEY, FakeEngine, make_settings


def _post(client, headers=None, wav=b"RIFF"):
    return client.post(
        "/v1/audio/transcriptions",
        headers=headers or {},
        files={"file": ("a.wav", wav, "audio/wav")},
        data={"model": "whisper-1"},
    )


def _assert_openai_error(resp, status: int, code: str) -> None:
    assert resp.status_code == status
    body = resp.json()
    assert set(body) == {"error"}
    assert {"message", "type", "code"} <= set(body["error"])
    assert body["error"]["code"] == code
    assert body["error"]["message"]


def test_sans_cle_401(client, wav, engine):
    resp = _post(client, wav=wav)
    _assert_openai_error(resp, 401, "missing_api_key")
    assert resp.json()["error"]["type"] == "invalid_request_error"
    assert resp.headers["www-authenticate"] == "Bearer"
    assert engine.calls == []


def test_mauvaise_cle_401(client, wav, engine):
    resp = _post(client, {"Authorization": "Bearer mrm_mauvaise_cle_000000000000"}, wav)
    _assert_openai_error(resp, 401, "invalid_api_key")
    assert engine.calls == []


@pytest.mark.parametrize("header", ["Basic " + API_KEY, API_KEY, "Bearer", "Bearer   "])
def test_entete_mal_forme_401(client, wav, header):
    resp = _post(client, {"Authorization": header}, wav)
    assert resp.status_code == 401


@pytest.mark.parametrize("key", [API_KEY, OTHER_KEY])
def test_bonne_cle_200(client, wav, key):
    resp = _post(client, {"Authorization": f"Bearer {key}"}, wav)
    assert resp.status_code == 200
    assert resp.json() == {"text": "Bonjour, ceci est un test de dictée vocale."}


def test_schema_bearer_insensible_a_la_casse(client, wav):
    assert _post(client, {"Authorization": f"bearer {API_KEY}"}, wav).status_code == 200


def test_models_exige_une_cle(client, auth):
    assert client.get("/v1/models").status_code == 401
    resp = client.get("/v1/models", headers=auth)
    assert resp.status_code == 200
    body = resp.json()
    assert body["object"] == "list"
    ids = [m["id"] for m in body["data"]]
    assert ids[0] == "large-v3-turbo" and "whisper-1" in ids


def test_health_sans_auth(client):
    resp = client.get("/health")
    assert resp.status_code == 200
    assert resp.json()["status"] == "ok"
    assert resp.json()["model"] == "large-v3-turbo"
    assert resp.json()["device"] == "cpu"


def test_cles_depuis_fichier(tmp_path, app_factory, wav):
    f = tmp_path / "keys.txt"
    f.write_text("# clés du cabinet\nmrm_cle_depuis_fichier_abcdefghij  # poste 1\n\n", encoding="utf-8")
    client = app_factory(api_keys="", api_keys_file=f)
    assert _post(client, {"Authorization": "Bearer mrm_cle_depuis_fichier_abcdefghij"}, wav).status_code == 200
    assert _post(client, {"Authorization": f"Bearer {API_KEY}"}, wav).status_code == 401


def test_refus_de_demarrer_sans_cle():
    with pytest.raises(ConfigError, match="Aucune clé API"):
        create_app(make_settings(api_keys=""), engine=FakeEngine())


def test_cle_trop_courte_refusee():
    with pytest.raises(ConfigError, match="trop courte"):
        create_app(make_settings(api_keys="abc"), engine=FakeEngine())


def test_mode_sans_auth_explicite(app_factory, wav):
    client = app_factory(api_keys="", allow_no_auth=True)
    assert _post(client, wav=wav).status_code == 200


def test_cli_serve_refuse_sans_cle(monkeypatch, tmp_path, capsys):
    monkeypatch.chdir(tmp_path)  # pas de .env parasite
    for var in ("MURMURE_API_KEYS", "MURMURE_API_KEYS_FILE", "MURMURE_ALLOW_NO_AUTH"):
        monkeypatch.delenv(var, raising=False)
    assert main(["serve"]) == 2
    assert "Aucune clé API" in capsys.readouterr().err


def test_generate_key_format():
    keys = {generate_key() for _ in range(200)}
    assert len(keys) == 200
    for k in keys:
        assert k.startswith("mrm_") and len(k) == 4 + 43 and k[4:].isalnum()


def test_cli_gen_key(capsys):
    assert main(["gen-key", "-n", "3"]) == 0
    lines = capsys.readouterr().out.split()
    assert len(lines) == 3 and all(line.startswith("mrm_") for line in lines)


def test_keystore_match():
    store = KeyStore([API_KEY, OTHER_KEY])
    assert store.match(API_KEY) == key_id(API_KEY)
    assert store.match(OTHER_KEY) == key_id(OTHER_KEY)
    assert store.match(API_KEY + "x") is None
    assert store.match("") is None

# SPDX-License-Identifier: AGPL-3.0-or-later
"""Authentification par clé API (`Authorization: Bearer <clé>`) et génération de clés."""

from __future__ import annotations

import hashlib
import hmac
import secrets
import string

from fastapi import Request

from murmure_server.errors import APIError

KEY_PREFIX = "mrm_"
_BASE62 = string.digits + string.ascii_letters
# 32 octets = 256 bits → 43 caractères en base 62.
_KEY_BYTES = 32
_KEY_CHARS = 43


def generate_key() -> str:
    """Clé forte : `mrm_` + 32 octets aléatoires (CSPRNG) encodés en base 62, longueur fixe."""
    n = int.from_bytes(secrets.token_bytes(_KEY_BYTES), "big")
    chars: list[str] = []
    for _ in range(_KEY_CHARS):
        n, r = divmod(n, 62)
        chars.append(_BASE62[r])
    return KEY_PREFIX + "".join(reversed(chars))


def key_id(key: str) -> str:
    """Identifiant non réversible d'une clé, utilisable dans les logs et le rate limiting."""
    return hashlib.sha256(key.encode()).hexdigest()[:12]


class KeyStore:
    """Vérifie les clés en temps constant.

    On compare les empreintes SHA-256 (longueur fixe) avec `hmac.compare_digest`, et on parcourt
    toujours toutes les clés : le temps de réponse ne dépend ni de la clé testée ni de sa position.
    """

    def __init__(self, keys: list[str]) -> None:
        self._digests: list[tuple[bytes, str]] = [
            (hashlib.sha256(k.encode()).digest(), key_id(k)) for k in keys
        ]

    @property
    def enabled(self) -> bool:
        return bool(self._digests)

    def __len__(self) -> int:
        return len(self._digests)

    def match(self, candidate: str) -> str | None:
        """Renvoie l'identifiant de la clé si elle est valide, sinon None."""
        digest = hashlib.sha256(candidate.encode()).digest()
        found: str | None = None
        for known, kid in self._digests:
            if hmac.compare_digest(digest, known):
                found = kid
        return found


def _unauthorized(message: str, code: str) -> APIError:
    return APIError(
        401,
        message,
        type="invalid_request_error",
        code=code,
        headers={"WWW-Authenticate": "Bearer"},
    )


def authenticate(request: Request) -> str:
    """Dépendance FastAPI : renvoie l'identifiant de la clé (ou « anonymous » si l'auth est désactivée)."""
    store: KeyStore = request.app.state.keystore
    if not store.enabled:
        return "anonymous"

    header = request.headers.get("authorization", "")
    scheme, _, token = header.partition(" ")
    token = token.strip()
    if scheme.lower() != "bearer" or not token:
        raise _unauthorized(
            "Clé API manquante : ajoutez l'en-tête « Authorization: Bearer <clé> ».", "missing_api_key"
        )
    kid = store.match(token)
    if kid is None:
        raise _unauthorized("Clé API invalide.", "invalid_api_key")
    request.state.key_id = kid
    return kid

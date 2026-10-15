# SPDX-License-Identifier: AGPL-3.0-or-later
"""Application FastAPI : `POST /v1/audio/transcriptions`, `GET /v1/models`, `GET /health`."""

from __future__ import annotations

import asyncio
import logging
import time
import uuid
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Any

from fastapi import Depends, FastAPI, Request
from fastapi.responses import JSONResponse, PlainTextResponse, Response
from pydantic import BaseModel, Field, ValidationError, field_validator
from starlette.concurrency import run_in_threadpool
from starlette.datastructures import UploadFile
from starlette.types import ASGIApp, Message, Receive, Scope, Send

from murmure_server import __version__
from murmure_server.auth import KeyStore, authenticate
from murmure_server.config import Settings
from murmure_server.engine import AudioDecodeError, Engine, InvalidLanguageError
from murmure_server.errors import APIError, error_body, install_error_handlers
from murmure_server.formats import ResponseFormat, to_srt, to_verbose_json, to_vtt
from murmure_server.ratelimit import RateLimiter

logger = logging.getLogger("murmure.api")

# Marge tolérée au niveau HTTP au-delà de la taille du fichier (en-têtes multipart, autres champs).
_MULTIPART_OVERHEAD = 1024 * 1024
# Alias exposés par /v1/models ; tout autre nom de modèle reçu est également accepté.
MODEL_ALIASES = ("whisper-1", "whisper-large-v3-turbo")


# --------------------------------------------------------------------------- middlewares (ASGI pur)


class BodySizeLimitMiddleware:
    """Refuse (413) les corps trop gros, avant tout parsing, y compris en transfert « chunked »."""

    def __init__(self, app: ASGIApp, max_bytes: int, path_prefix: str = "/v1/audio/") -> None:
        self.app = app
        self.max_bytes = max_bytes
        self.path_prefix = path_prefix

    def _too_large(self) -> APIError:
        mb = (self.max_bytes - _MULTIPART_OVERHEAD) / (1024 * 1024)
        return APIError(413, f"Fichier trop volumineux (maximum {mb:g} Mo).", code="file_too_large", param="file")

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http" or not scope["path"].startswith(self.path_prefix):
            await self.app(scope, receive, send)
            return

        for name, value in scope["headers"]:
            if name == b"content-length":
                try:
                    declared = int(value)
                except ValueError:
                    declared = 0
                if declared > self.max_bytes:
                    exc = self._too_large()
                    response = JSONResponse(status_code=413, content=error_body(exc.message, exc.type, exc.code, exc.param))
                    await response(scope, receive, send)
                    return

        received = 0

        async def limited_receive() -> Message:
            nonlocal received
            message = await receive()
            if message["type"] == "http.request":
                received += len(message.get("body", b""))
                if received > self.max_bytes:
                    # Levée pendant le parsing du formulaire, convertie en 413 par le gestionnaire APIError.
                    raise self._too_large()
            return message

        await self.app(scope, limited_receive, send)


class RequestLogMiddleware:
    """Un événement JSON par requête : méthode, chemin, statut, latence, métadonnées audio. Jamais de contenu."""

    def __init__(self, app: ASGIApp) -> None:
        self.app = app

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return

        request_id = uuid.uuid4().hex[:16]
        state: dict[str, Any] = scope.setdefault("state", {})
        state["request_id"] = request_id
        started = time.perf_counter()
        status = 500

        async def send_wrapper(message: Message) -> None:
            nonlocal status
            if message["type"] == "http.response.start":
                status = message["status"]
                headers = list(message.get("headers", []))
                headers.append((b"x-request-id", request_id.encode()))
                message = {**message, "headers": headers}
            await send(message)

        try:
            await self.app(scope, receive, send_wrapper)
        finally:
            path = scope["path"]
            if path != "/health":  # le healthcheck Docker toutes les 30 s n'a pas besoin d'être journalisé
                fields = {
                    k: state[k]
                    for k in ("key_id", "audio_bytes", "audio_duration_s", "processing_ms", "queue_ms",
                              "response_format", "requested_model", "language", "error_code")
                    if k in state
                }
                logger.info(
                    "request",
                    extra={
                        "request_id": request_id,
                        "method": scope["method"],
                        "path": path,
                        "status": status,
                        "latency_ms": round((time.perf_counter() - started) * 1000, 1),
                        **fields,
                    },
                )


# --------------------------------------------------------------------------- paramètres


class TranscriptionParams(BaseModel):
    """Champs texte du formulaire multipart (le fichier est traité à part)."""

    model: str = Field(default="whisper-1", max_length=200)
    language: str | None = Field(default=None, max_length=16)
    prompt: str | None = Field(default=None, max_length=8000)
    response_format: ResponseFormat = ResponseFormat.json
    temperature: float = Field(default=0.0, ge=0.0, le=1.0)

    @field_validator("language")
    @classmethod
    def _normalize_language(cls, v: str | None) -> str | None:
        # "", "auto" → détection automatique ; "fr-FR" → "fr".
        if v is None:
            return None
        v = v.strip().lower().replace("_", "-").split("-", 1)[0]
        return None if v in ("", "auto") else v


def _params_from_form(form: Any) -> TranscriptionParams:
    raw = {
        k: form.get(k)
        for k in ("model", "language", "prompt", "response_format", "temperature")
        if isinstance(form.get(k), str)
    }
    try:
        return TranscriptionParams.model_validate(raw)
    except ValidationError as exc:
        err = exc.errors()[0]
        param = str(err["loc"][0]) if err.get("loc") else None
        if param == "response_format":
            message = "response_format doit valoir json, text, verbose_json, srt ou vtt."
        elif param == "temperature":
            message = "temperature doit être un nombre entre 0 et 1."
        else:
            message = f"Paramètre invalide « {param} » : {err.get('msg', '')}"
        raise APIError(400, message, code="invalid_value", param=param) from None


# --------------------------------------------------------------------------- application


def create_app(settings: Settings, engine: Engine | None = None) -> FastAPI:
    """Construit l'application.

    `engine` permet d'injecter un moteur (tests) ; sinon faster-whisper est chargé au démarrage.
    Lève `ConfigError` si l'authentification n'est pas configurée correctement.
    """
    keystore = KeyStore(settings.validate_auth())

    @asynccontextmanager
    async def lifespan(app: FastAPI) -> AsyncIterator[None]:
        if app.state.engine is None:
            from murmure_server.engine import FasterWhisperEngine

            t0 = time.perf_counter()
            # Chargement (et téléchargement éventuel) hors de la boucle d'événements.
            app.state.engine = await run_in_threadpool(FasterWhisperEngine, settings)
            eng = app.state.engine
            logger.info(
                "modèle prêt",
                extra={"model": eng.model_name, "device": eng.device, "compute_type": eng.compute_type,
                       "load_s": round(time.perf_counter() - t0, 1)},
            )
        if not keystore.enabled:
            logger.warning("authentification DÉSACTIVÉE (MURMURE_ALLOW_NO_AUTH=1) : ne pas exposer ce serveur")
        else:
            logger.info("authentification active", extra={"keys": len(keystore)})
        yield

    app = FastAPI(
        title="Murmure STT",
        version=__version__,
        lifespan=lifespan,
        # Pas de documentation interactive exposée : surface d'attaque minimale.
        docs_url=None,
        redoc_url=None,
        openapi_url=None,
    )
    app.state.settings = settings
    app.state.engine = engine
    app.state.keystore = keystore
    app.state.semaphore = asyncio.Semaphore(settings.max_concurrency)
    app.state.ratelimiter = RateLimiter(settings.rate_limit_per_minute)

    install_error_handlers(app)
    app.add_middleware(BodySizeLimitMiddleware, max_bytes=settings.max_upload_bytes + _MULTIPART_OVERHEAD)
    app.add_middleware(RequestLogMiddleware)

    def get_engine(request: Request) -> Engine:
        eng: Engine | None = request.app.state.engine
        if eng is None:
            raise APIError(503, "Modèle en cours de chargement.", type="server_error", code="model_loading")
        return eng

    def rate_limit(request: Request, kid: str = Depends(authenticate)) -> str:
        request.state.key_id = kid
        delay = request.app.state.ratelimiter.check(kid)
        if delay is not None:
            raise APIError(
                429,
                "Trop de requêtes pour cette clé, réessayez plus tard.",
                type="rate_limit_error",
                code="rate_limit_exceeded",
                headers={"Retry-After": RateLimiter.retry_after_header(delay)},
            )
        return kid

    @app.get("/health")
    async def health(request: Request) -> JSONResponse:
        eng: Engine | None = request.app.state.engine
        if eng is None:
            return JSONResponse(status_code=503, content={"status": "loading"})
        return JSONResponse(
            {
                "status": "ok",
                "model": eng.model_name,
                "device": eng.device,
                "compute_type": eng.compute_type,
                "version": __version__,
            }
        )

    @app.get("/v1/models", dependencies=[Depends(rate_limit)])
    async def list_models(eng: Engine = Depends(get_engine)) -> JSONResponse:
        ids = [eng.model_name, *[a for a in MODEL_ALIASES if a != eng.model_name]]
        return JSONResponse(
            {
                "object": "list",
                "data": [{"id": i, "object": "model", "created": 0, "owned_by": "murmure"} for i in ids],
            }
        )

    @app.post("/v1/audio/transcriptions", dependencies=[Depends(rate_limit)])
    async def transcriptions(request: Request, eng: Engine = Depends(get_engine)) -> Response:
        # Le formulaire n'est lu qu'APRÈS l'authentification (dépendances ci-dessus) :
        # un client non authentifié ne peut pas faire téléverser 25 Mo au serveur.
        async with request.form(max_files=1, max_fields=32) as form:
            upload = form.get("file")
            if not isinstance(upload, UploadFile):
                raise APIError(400, "Champ « file » manquant (multipart/form-data).", code="missing_file", param="file")
            params = _params_from_form(form)
            audio = await upload.read()

        state = request.state
        state.audio_bytes = len(audio)
        state.response_format = params.response_format.value
        state.requested_model = params.model  # tout nom est accepté et mappé sur le modèle chargé
        if len(audio) > settings.max_upload_bytes:
            raise APIError(413, f"Fichier trop volumineux (maximum {settings.max_upload_mb:g} Mo).",
                           code="file_too_large", param="file")
        if not audio:
            raise APIError(400, "Le fichier audio est vide.", code="empty_file", param="file")

        semaphore: asyncio.Semaphore = request.app.state.semaphore
        queued = time.perf_counter()
        try:
            async with asyncio.timeout(settings.queue_timeout_s):
                await semaphore.acquire()
        except TimeoutError:
            raise APIError(503, "Serveur saturé, réessayez dans un instant.", type="server_error",
                           code="server_busy", headers={"Retry-After": "5"}) from None
        try:
            state.queue_ms = round((time.perf_counter() - queued) * 1000, 1)
            t0 = time.perf_counter()
            result = await run_in_threadpool(
                eng.transcribe,
                audio,
                language=params.language,
                prompt=params.prompt,
                temperature=params.temperature,
            )
            state.processing_ms = round((time.perf_counter() - t0) * 1000, 1)
        except AudioDecodeError as exc:
            state.error_code = "invalid_audio"
            raise APIError(400, str(exc), code="invalid_audio", param="file") from None
        except InvalidLanguageError as exc:
            state.error_code = "invalid_language"
            raise APIError(400, str(exc), code="invalid_language", param="language") from None
        finally:
            semaphore.release()

        state.audio_duration_s = result.duration
        state.language = result.language

        match params.response_format:
            case ResponseFormat.json:
                return JSONResponse({"text": result.text})
            case ResponseFormat.text:
                return PlainTextResponse(result.text)
            case ResponseFormat.verbose_json:
                return JSONResponse(to_verbose_json(result))
            case ResponseFormat.srt:
                return PlainTextResponse(to_srt(result))
            case ResponseFormat.vtt:
                return PlainTextResponse(to_vtt(result), media_type="text/vtt")

    return app

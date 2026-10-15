# SPDX-License-Identifier: AGPL-3.0-or-later
"""Erreurs au format OpenAI : `{"error": {"message", "type", "param", "code"}}`."""

from __future__ import annotations

from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from starlette.exceptions import HTTPException as StarletteHTTPException


class APIError(Exception):
    """Erreur applicative convertie telle quelle en réponse JSON OpenAI."""

    def __init__(
        self,
        status_code: int,
        message: str,
        *,
        type: str = "invalid_request_error",
        code: str | None = None,
        param: str | None = None,
        headers: dict[str, str] | None = None,
    ) -> None:
        super().__init__(message)
        self.status_code = status_code
        self.message = message
        self.type = type
        self.code = code
        self.param = param
        self.headers = headers


def error_body(message: str, type: str, code: str | None = None, param: str | None = None) -> dict:
    return {"error": {"message": message, "type": type, "param": param, "code": code}}


def error_response(exc: APIError) -> JSONResponse:
    return JSONResponse(
        status_code=exc.status_code,
        content=error_body(exc.message, exc.type, exc.code, exc.param),
        headers=exc.headers,
    )


def install_error_handlers(app: FastAPI) -> None:
    @app.exception_handler(APIError)
    async def _api_error(_: Request, exc: APIError) -> JSONResponse:
        return error_response(exc)

    @app.exception_handler(RequestValidationError)
    async def _validation_error(_: Request, exc: RequestValidationError) -> JSONResponse:
        # On ne renvoie jamais la valeur reçue (elle pourrait contenir un prompt confidentiel).
        first = exc.errors()[0] if exc.errors() else {}
        loc = [str(p) for p in first.get("loc", ()) if p not in ("body", "query", "header")]
        param = loc[-1] if loc else None
        msg = first.get("msg", "Requête invalide.")
        return JSONResponse(
            status_code=400,
            content=error_body(
                f"Paramètre invalide{f' « {param} »' if param else ''} : {msg}",
                "invalid_request_error",
                "invalid_value" if param != "file" else "missing_file",
                param,
            ),
        )

    @app.exception_handler(StarletteHTTPException)
    async def _http_error(_: Request, exc: StarletteHTTPException) -> JSONResponse:
        messages = {404: "Ressource introuvable.", 405: "Méthode non autorisée."}
        return JSONResponse(
            status_code=exc.status_code,
            content=error_body(
                messages.get(exc.status_code, str(exc.detail)),
                "invalid_request_error",
                {404: "not_found", 405: "method_not_allowed"}.get(exc.status_code),
            ),
            headers=getattr(exc, "headers", None),
        )

    @app.exception_handler(Exception)
    async def _unexpected(_: Request, exc: Exception) -> JSONResponse:
        return JSONResponse(
            status_code=500,
            content=error_body("Erreur interne du serveur.", "server_error", "internal_error"),
        )

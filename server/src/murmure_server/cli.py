"""Ligne de commande : `murmure-server serve` et `murmure-server gen-key`."""

from __future__ import annotations

import argparse
import sys

from murmure_server import __version__


def _cmd_gen_key(args: argparse.Namespace) -> int:
    from murmure_server.auth import generate_key

    for _ in range(args.count):
        print(generate_key())
    return 0


def _cmd_serve(args: argparse.Namespace) -> int:
    import uvicorn

    from murmure_server.app import create_app
    from murmure_server.config import ConfigError, Settings
    from murmure_server.logs import setup_logging

    try:
        settings = Settings()
    except Exception as exc:  # pydantic.ValidationError : variable d'environnement mal typée
        print(f"Configuration invalide :\n{exc}", file=sys.stderr)
        return 2
    setup_logging(settings.log_level)
    try:
        app = create_app(settings)
    except ConfigError as exc:
        print(f"Erreur de configuration : {exc}", file=sys.stderr)
        return 2

    uvicorn.run(
        app,
        host=args.host or settings.host,
        port=args.port or settings.port,
        log_config=None,  # notre journalisation JSON (cf. logs.py)
        access_log=False,
        server_header=False,
        proxy_headers=True,
    )
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="murmure-server", description="Serveur de transcription Murmure.")
    parser.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    sub = parser.add_subparsers(dest="command")

    serve = sub.add_parser("serve", help="Démarrer le serveur (configuration par variables MURMURE_*).")
    serve.add_argument("--host", default=None, help="Adresse d'écoute (défaut : MURMURE_HOST ou 0.0.0.0).")
    serve.add_argument("--port", type=int, default=None, help="Port (défaut : MURMURE_PORT ou 8000).")
    serve.set_defaults(func=_cmd_serve)

    gen = sub.add_parser("gen-key", help="Générer une clé API forte (mrm_…).")
    gen.add_argument("-n", "--count", type=int, default=1, help="Nombre de clés à générer.")
    gen.set_defaults(func=_cmd_gen_key)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if not getattr(args, "func", None):
        parser.print_help()
        return 1
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())

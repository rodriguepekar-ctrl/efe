"""``efe-platform`` CLI — thin wrapper around the programmatic API.

Subcommands:

* ``efe-platform serve`` — start the FastAPI app (uvicorn).
* ``efe-platform answer <prompt>`` — one-shot bot answer, stdout JSON.
* ``efe-platform validate-catalog <path>`` — lint a catalog YAML.
* ``efe-platform version`` — print the version string.

The CLI uses :mod:`argparse` only, no extra dependency. Each subcommand
is a standalone function so it can be unit-tested in isolation.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
from argparse import ArgumentParser, Namespace

from efe_platform.bot.agent import Bot, BotConfig
from efe_platform.catalog.loader import load_catalog
from efe_platform.config import PlatformConfig
from efe_platform.errors import EfePlatformError
from efe_platform.swarm.client import InMemorySwarmTransport, SwarmClient, SwarmEndpoint
from efe_platform.swarm.cone import ConeRegistryClient, InMemoryAdminTransport
from efe_platform.version import VERSION


def main(argv: list[str] | None = None) -> int:
    parser = _build_parser()
    args = parser.parse_args(argv)
    try:
        return int(args.func(args) or 0)
    except EfePlatformError as exc:
        print(f"error: {exc.code}: {exc.message}", file=sys.stderr)
        return 2


# ---------------------------------------------------------------------------
#  Parser
# ---------------------------------------------------------------------------


def _build_parser() -> ArgumentParser:
    p = ArgumentParser(prog="efe-platform", description="EFE Platform CLI")
    p.add_argument("--version", action="store_true", help="Print version and exit.")
    sub = p.add_subparsers(dest="cmd")

    sv = sub.add_parser("serve", help="Start the FastAPI app.")
    sv.add_argument("--host", default=None)
    sv.add_argument("--port", type=int, default=None)
    sv.add_argument(
        "--standalone",
        action="store_true",
        help="Use in-memory swarm transport (no Rust server needed).",
    )
    sv.set_defaults(func=_cmd_serve)

    an = sub.add_parser("answer", help="One-shot bot answer.")
    an.add_argument("prompt")
    an.add_argument("--tenant", default=None)
    an.add_argument("--standalone", action="store_true")
    an.set_defaults(func=_cmd_answer)

    vc = sub.add_parser("validate-catalog", help="Validate a catalog YAML.")
    vc.add_argument("path")
    vc.set_defaults(func=_cmd_validate_catalog)

    vr = sub.add_parser("version", help="Print version and exit.")
    vr.set_defaults(func=_cmd_version)

    # If no subcommand given, show help.
    def _no_cmd(_args: Namespace) -> int:
        p.print_help()
        return 0

    p.set_defaults(func=_no_cmd)
    return p


# ---------------------------------------------------------------------------
#  Subcommands
# ---------------------------------------------------------------------------


def _cmd_version(_args: Namespace) -> int:
    print(VERSION)
    return 0


def _cmd_serve(args: Namespace) -> int:
    try:
        import uvicorn
    except ImportError:
        print(
            "error: uvicorn not installed. `pip install efe-platform[api]`",
            file=sys.stderr,
        )
        return 2
    from efe_platform.api.http_app import create_default_app

    cfg = PlatformConfig.from_env()
    host = args.host or cfg.api.host
    port = args.port or cfg.api.port
    if args.standalone:
        os.environ.setdefault("EFE_PLATFORM_LLM__PROVIDER", "mock")
    app = create_default_app(cfg)
    uvicorn.run(app, host=host, port=port)
    return 0


def _cmd_answer(args: Namespace) -> int:
    return asyncio.run(_run_answer(args))


async def _run_answer(args: Namespace) -> int:
    async with SwarmClient(
        SwarmEndpoint("localhost:50051"),
        transport=InMemorySwarmTransport() if args.standalone else None,
    ) as swarm:
        cones = ConeRegistryClient(InMemoryAdminTransport()) if args.standalone else None
        bot = Bot(BotConfig.from_env(), swarm=swarm, cones=cones)
        reply = await bot.answer(args.prompt, tenant=args.tenant)
        print(json.dumps(reply.to_dict(), ensure_ascii=False, indent=2))
        return 0 if reply.ok else 1


def _cmd_validate_catalog(args: Namespace) -> int:
    cat = load_catalog(args.path)
    print(
        json.dumps(
            {
                "ok": True,
                "version": cat.version,
                "tasks": [t.id for t in cat.tasks],
                "anchors": [a.id for a in cat.anchors],
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())

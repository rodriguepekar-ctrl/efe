"""Minimal CLI — ``efe-inference <subcommand>``."""

from __future__ import annotations

import argparse
import asyncio


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser("efe-inference")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("init-db", help="create DB tables from metadata")
    sub.add_parser("health", help="print backend health snapshot")

    args = parser.parse_args(argv)
    if args.cmd == "init-db":
        return asyncio.run(_init_db())
    if args.cmd == "health":
        return asyncio.run(_health())
    parser.print_help()
    return 2


async def _init_db() -> int:
    from efe_inference.config import InferenceSettings
    from efe_inference.db.engine import build_engine, create_all

    settings = InferenceSettings.from_env()
    engine = build_engine(settings.database)
    await create_all(engine)
    await engine.dispose()
    print(f"DB initialised at {settings.database.url}")
    return 0


async def _health() -> int:
    print("(stub) backend health snapshot — wire registry at runtime")
    return 0


__all__ = ["main"]

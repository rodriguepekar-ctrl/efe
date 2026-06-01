"""CLI entry point: ``python -m efe_gate``."""

from __future__ import annotations

import argparse
import asyncio
import json
import sys

from efe_gate.api.context import GateContext, set_context
from efe_gate.config import GateSettings
from efe_gate.db.engine import get_engine, get_sessionmaker, init_schema
from efe_gate.observability.logging import init_logging


def _parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="efe_gate",
        description="EFE Evaluation Gate (M05) CLI.",
    )
    p.add_argument("--env", default=None)
    p.add_argument("--db-url", default=None)
    sub = p.add_subparsers(dest="command", required=True)
    sub.add_parser("serve", help="Run the HTTP gate server.")
    sub.add_parser("migrate", help="Create the DB schema.")
    sub.add_parser("config", help="Print resolved config as JSON.")
    sub.add_parser("metrics-catalog", help="Print known metrics.")
    return p.parse_args(argv)


def _settings_from_ns(ns: argparse.Namespace) -> GateSettings:
    overrides: dict[str, object] = {}
    if ns.env:
        overrides["environment"] = ns.env
    if ns.db_url:
        overrides["database"] = {"url": ns.db_url}
    return GateSettings.from_env(overrides=overrides)


async def _cmd_migrate(ns: argparse.Namespace) -> int:
    cfg = _settings_from_ns(ns)
    engine = get_engine(cfg.database.url)
    await init_schema(engine)
    print(f"schema created on {cfg.database.url}")
    return 0


async def _cmd_config(ns: argparse.Namespace) -> int:
    cfg = _settings_from_ns(ns)
    print(json.dumps(cfg.model_dump(), indent=2, default=str))
    return 0


def _cmd_metrics_catalog() -> int:
    from efe_gate.metrics.catalog import CATALOG
    rows = [
        {
            "name": md.name, "role": md.role.value,
            "higher_is_better": md.higher_is_better,
            "default_test": md.default_test,
            "task_types": sorted(md.task_types),
        }
        for md in sorted(CATALOG.values(), key=lambda x: x.name)
    ]
    print(json.dumps(rows, indent=2))
    return 0


def _cmd_serve(ns: argparse.Namespace) -> int:
    try:
        import uvicorn
    except ImportError as exc:  # pragma: no cover
        raise RuntimeError(
            "uvicorn is required for 'serve'",
        ) from exc
    from efe_gate.api.http_app import build_app

    cfg = _settings_from_ns(ns)
    sm = get_sessionmaker(cfg.database.url)
    ctx = GateContext.build(cfg, sm)
    set_context(ctx)
    app = build_app(ctx)
    uvicorn.run(
        app, host=cfg.api.host, port=cfg.api.port,
        log_level=cfg.observability.log_level.lower(),
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    ns = _parse_args(argv if argv is not None else sys.argv[1:])
    init_logging(level="INFO", json=True)
    if ns.command == "serve":
        return _cmd_serve(ns)
    if ns.command == "migrate":
        return asyncio.run(_cmd_migrate(ns))
    if ns.command == "config":
        return asyncio.run(_cmd_config(ns))
    if ns.command == "metrics-catalog":
        return _cmd_metrics_catalog()
    raise AssertionError(  # pragma: no cover
        f"unknown command {ns.command!r}",
    )


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())

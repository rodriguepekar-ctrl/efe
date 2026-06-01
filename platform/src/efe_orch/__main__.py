"""CLI entry point for :mod:`efe_orch`.

Usage::

    python -m efe_orch serve          # run the HTTP server
    python -m efe_orch migrate        # create all tables in the configured DB
    python -m efe_orch reap --once    # run one heartbeat-reaper pass
    python -m efe_orch config         # print the resolved configuration

Most production deployments override these commands via a process
manager (systemd, K8s Deployment) that calls ``python -m efe_orch serve``
directly; the subcommand is kept intentionally small so the failure
surface is visible in the CLI help output.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import sys
from typing import Any

from efe_orch.api.context import OrchContext, set_context
from efe_orch.config import OrchSettings
from efe_orch.db.engine import get_sessionmaker, init_schema
from efe_orch.observability.logging import init_logging


def _settings_from_argv(ns: argparse.Namespace) -> OrchSettings:
    overrides: dict[str, Any] = {}
    if ns.env:
        overrides["environment"] = ns.env
    if ns.db_url:
        overrides.setdefault("database", {})["url"] = ns.db_url
    return OrchSettings.from_env(overrides=overrides)


def _build_context(cfg: OrchSettings) -> OrchContext:
    sm = get_sessionmaker(cfg.database.url)
    ctx = OrchContext.build(cfg, sm)
    set_context(ctx)
    return ctx


async def _cmd_migrate(ns: argparse.Namespace) -> int:
    cfg = _settings_from_argv(ns)
    # ``get_engine`` is the supported handle for schema bootstrap;
    # ``get_sessionmaker`` wraps the same cached engine.
    from efe_orch.db.engine import get_engine
    engine = get_engine(cfg.database.url)
    await init_schema(engine)
    print(f"schema created on {cfg.database.url}")
    return 0


async def _cmd_config(ns: argparse.Namespace) -> int:
    cfg = _settings_from_argv(ns)
    print(json.dumps(cfg.model_dump(), indent=2, default=str))
    return 0


async def _cmd_reap_once(ns: argparse.Namespace) -> int:
    cfg = _settings_from_argv(ns)
    ctx = _build_context(cfg)
    from efe_orch.heartbeat.reaper import HeartbeatReaper
    from efe_orch.scheduler.service import SchedulerService
    from efe_orch.state_machine.transitions import StateMachine

    async with ctx.sessionmaker() as s:
        scheduler = SchedulerService(
            s,
            scheduler_settings=cfg.scheduler,
            quota_settings=cfg.quota,
        )
        sm = StateMachine(
            s, lock_backend=ctx.lock_backend, clock=ctx.clock,
        )
        reaper = HeartbeatReaper(
            s, state_machine=sm, settings=cfg.heartbeat,
            scheduler=scheduler, clock=ctx.clock,
        )
        report = await reaper.scan()
        await s.commit()
    print(
        f"reaper: scanned={report.scanned} "
        f"reaped={len(report.reaped)} "
        f"skipped={len(report.skipped)}"
    )
    return 0


def _cmd_serve(ns: argparse.Namespace) -> int:
    try:
        import uvicorn
    except ImportError as exc:  # pragma: no cover — optional dep
        raise RuntimeError(
            "uvicorn is required for 'serve'; "
            "install with: pip install 'efe-platform[api]'",
        ) from exc
    from efe_orch.api.http_app import build_app

    cfg = _settings_from_argv(ns)
    ctx = _build_context(cfg)
    app = build_app(ctx)
    uvicorn.run(
        app,
        host=cfg.api.host,
        port=cfg.api.port,
        log_level=cfg.observability.log_level.lower(),
    )
    return 0


def _parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="efe_orch",
        description="EFE Training Orchestrator (M03) CLI.",
    )
    p.add_argument("--env", default=None, help="Override environment name.")
    p.add_argument("--db-url", default=None, help="Override database URL.")
    sub = p.add_subparsers(dest="command", required=True)
    sub.add_parser("serve", help="Run the HTTP orchestrator server.")
    sub.add_parser("migrate", help="Create the orchestrator DB schema.")
    sub.add_parser("config", help="Print the resolved configuration as JSON.")
    reap = sub.add_parser("reap", help="Run the heartbeat reaper.")
    reap.add_argument("--once", action="store_true", help="One-shot scan.")
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    ns = _parse_args(argv if argv is not None else sys.argv[1:])
    init_logging(level="INFO", json=True)

    if ns.command == "serve":
        return _cmd_serve(ns)
    if ns.command == "migrate":
        return asyncio.run(_cmd_migrate(ns))
    if ns.command == "config":
        return asyncio.run(_cmd_config(ns))
    if ns.command == "reap":
        return asyncio.run(_cmd_reap_once(ns))
    raise AssertionError(f"unknown command {ns.command!r}")  # pragma: no cover


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())

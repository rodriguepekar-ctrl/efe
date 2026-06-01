"""Minimal CLI — ``efe-drift <subcommand>``.

Subcommands :

* ``version`` — print SDK version.
* ``init-db`` — create DB tables on the configured backend.
* ``thresholds`` — print the default severity ladders.

Operational admin commands (``features register``, ``baselines
promote``, ``rules crud``, ``incidents ack``, ``evaluate``) live in
the service container — wiring them here would require a running DB
+ NATS and would duplicate the HTTP surface in §:mod:`efe_drift.api`.
"""

from __future__ import annotations

import argparse
import asyncio
import json

from efe_drift.detection.severity import DEFAULTS
from efe_drift.version import __version__


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser("efe-drift")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("version", help="print SDK version")
    sub.add_parser("init-db", help="create DB tables from metadata")
    sub.add_parser(
        "thresholds",
        help="print the default severity ladder per drift method",
    )

    args = parser.parse_args(argv)
    if args.cmd == "version":
        print(__version__)
        return 0
    if args.cmd == "init-db":
        return asyncio.run(_init_db())
    if args.cmd == "thresholds":
        print(json.dumps(
            {
                method: {
                    "warning": t.warning,
                    "major": t.major,
                    "critical": t.critical,
                    "orientation": t.orientation,
                }
                for method, t in DEFAULTS.items()
            },
            indent=2,
        ))
        return 0
    parser.print_help()
    return 2


async def _init_db() -> int:
    from efe_drift.config import DriftSettings
    from efe_drift.db.engine import build_engine, create_all

    settings = DriftSettings.from_env()
    engine = build_engine(settings.database)
    await create_all(engine)
    await engine.dispose()
    print(f"DB initialised at {settings.database.url}")
    return 0


__all__ = ["main"]

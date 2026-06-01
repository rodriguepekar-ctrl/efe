"""Minimal CLI — ``efe-rollout <subcommand>``.

Subcommands :

* ``init-db`` — materialise the schema on a fresh DB (SQLite or
  Postgres, per :class:`DatabaseSettings`).
* ``power proportions`` — run a sample-size calculation without
  spinning the service.
* ``version`` — print the SDK version.

Operational commands (create experiment, abort, pre-register, etc.)
live in the admin CLI that will be delivered alongside the service
container — out of scope for this entry point.
"""

from __future__ import annotations

import argparse
import asyncio
import json

from efe_rollout.analysis.power import sample_size_proportions
from efe_rollout.version import __version__


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser("efe-rollout")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("init-db", help="create DB tables from metadata")
    sub.add_parser("version", help="print SDK version")

    power = sub.add_parser(
        "power", help="power analysis helpers",
    )
    power_sub = power.add_subparsers(dest="power_cmd", required=True)
    p_prop = power_sub.add_parser(
        "proportions",
        help="sample-size calc for two-proportion A/B",
    )
    p_prop.add_argument("--baseline", type=float, required=True)
    p_prop.add_argument("--relative-lift", type=float, required=True)
    p_prop.add_argument("--alpha", type=float, default=0.05)
    p_prop.add_argument("--power", type=float, default=0.8)
    p_prop.add_argument("--variants", type=int, default=2)

    args = parser.parse_args(argv)
    if args.cmd == "version":
        print(__version__)
        return 0
    if args.cmd == "init-db":
        return asyncio.run(_init_db())
    if args.cmd == "power" and args.power_cmd == "proportions":
        result = sample_size_proportions(
            baseline=args.baseline,
            relative_lift=args.relative_lift,
            alpha=args.alpha,
            power=args.power,
            variants=args.variants,
        )
        print(
            json.dumps(
                {
                    "per_variant_sample_size": result.per_variant_sample_size,
                    "total_sample_size": result.total_sample_size,
                    "detectable_effect_abs": result.detectable_effect_abs,
                    "baseline": result.baseline,
                    "alpha": result.alpha,
                    "power": result.power,
                },
                indent=2,
            ),
        )
        return 0
    parser.print_help()
    return 2


async def _init_db() -> int:
    from efe_rollout.config import RolloutSettings
    from efe_rollout.db.engine import build_engine, create_all

    settings = RolloutSettings.from_env()
    engine = build_engine(settings.database)
    await create_all(engine)
    await engine.dispose()
    print(f"DB initialised at {settings.database.url}")
    return 0


__all__ = ["main"]

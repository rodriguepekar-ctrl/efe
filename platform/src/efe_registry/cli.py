"""Minimal CLI — ``efe-registry <subcommand>``.

Sub-commands:

* ``init-db``          — create tables from the declarative metadata.
* ``render-policy``    — dump ClusterImagePolicy YAML to stdout.
* ``rollback``         — manual rollback (requires DB + URL via env).
* ``verify-artifact``  — verify cosign signature on a given OCI ref.

This is deliberately minimal; heavy ops tooling lives in the
``efe-platform`` CLI that ships with the platform.
"""

from __future__ import annotations

import argparse
import asyncio
import sys


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser("efe-registry")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("init-db", help="create tables")
    sub.add_parser("render-policy", help="dump ClusterImagePolicy YAML")
    rb = sub.add_parser("rollback", help="execute a rollback")
    rb.add_argument("--tenant", required=True)
    rb.add_argument("--task", required=True)
    rb.add_argument("--actor", required=True)
    rb.add_argument("--reason", required=True)
    vr = sub.add_parser("verify-artifact", help="verify cosign signature")
    vr.add_argument("--oci-ref", required=True)

    args = parser.parse_args(argv)

    if args.cmd == "init-db":
        return asyncio.run(_init_db())
    if args.cmd == "render-policy":
        return _render_policy()
    if args.cmd == "rollback":
        return asyncio.run(_rollback(args))
    if args.cmd == "verify-artifact":
        return asyncio.run(_verify(args))

    parser.print_help()
    return 2


async def _init_db() -> int:
    from efe_registry.config import RegistrySettings
    from efe_registry.db.engine import build_engine, create_all
    settings = RegistrySettings.from_env()
    engine = build_engine(settings.database)
    await create_all(engine)
    await engine.dispose()
    print(f"database initialised at {settings.database.url!s}")
    return 0


def _render_policy() -> int:
    from efe_registry.admission.policy import (
        ClusterImagePolicy,
        render_policy_yaml,
    )
    from efe_registry.config import RegistrySettings
    settings = RegistrySettings.from_env()
    policy = ClusterImagePolicy.from_settings(
        admission=settings.admission, signing=settings.signing,
    )
    sys.stdout.write(render_policy_yaml(policy))
    return 0


async def _rollback(args: argparse.Namespace) -> int:
    print(
        f"(stub) would issue rollback tenant={args.tenant!s} "
        f"task={args.task!s} actor={args.actor!s}",
    )
    return 0


async def _verify(args: argparse.Namespace) -> int:
    from efe_registry.config import RegistrySettings
    from efe_registry.signing.models import (
        VerificationInput,
        VerificationLayer,
    )
    from efe_registry.signing.verifier import SignatureVerifier
    settings = RegistrySettings.from_env()
    verifier = SignatureVerifier(settings=settings.signing)
    result = await verifier.verify(VerificationInput(
        artifact_id="cli",
        layer=VerificationLayer.L1_PRE_PUBLISH,
        oci_ref=args.oci_ref,
    ))
    print(f"ok={result.ok} method={result.method.value} "
           f"details={result.details}")
    return 0 if result.ok else 3


__all__ = ["main"]

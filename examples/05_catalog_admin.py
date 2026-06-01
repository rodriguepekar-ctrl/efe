"""Sprint D12 — example 5: server-side catalog admin lifecycle.

Goal: end-to-end walkthrough of the new ``EfeAdminClient`` catalog
operations introduced in v0.1.12. The script exercises every D12
RPC against a live ``efe-server`` :

    1. ``register_catalog``     — atomic v1 import
    2. ``list_catalogs``        — discover what's there
    3. ``list_catalog_versions``— version history per (tenant, name)
    4. ``get_catalog``          — fetch a specific version
    5. ``evolve_catalog``       — append a class, returning v2
    6. ``evolve_catalog``       — update thresholds, returning v3
    7. ``get_catalog``          — verify v3 sees both edits
    8. ``delete_catalog``       — clean up

The example is **idempotent-ish** : it picks a name with a
timestamp suffix so re-running the script doesn't collide with a
previous run.

Pre-requisites:
    * efe-server reachable on EFE_HOST (default 127.0.0.1:50051)
      with the ``CatalogService`` enabled (D12+).
    * EFE_TOKEN env var set, JWT carrying both
      ``admin:catalog:read`` and ``admin:catalog:write`` scopes.

Run:
    EFE_TOKEN=$(cat /tmp/jwt) python examples/05_catalog_admin.py

Exit codes:
    0 — full lifecycle completed.
    2 — operational error (missing token, server unreachable).
    3 — auth failure (token missing required scopes).
    4 — invalid catalog content (schema, embedder mismatch, …).
"""

from __future__ import annotations

import os
import sys
import time
import uuid
from typing import Any

from efe_tool import (
    EfeAdminClient,
    EfeAuthError,
    EfeError,
    EfeInvalidArgumentError,
    EfeNotFoundError,
    EvolveOp,
    RegisteredCatalog,
)


# ─────────────────────────────────────────────────────────────────────
# Catalog content — a realistic 3-class support-triage shape
# ─────────────────────────────────────────────────────────────────────

INITIAL_CONTENT: dict[str, Any] = {
    "embedder": {
        "family": "sentence-transformers",
        "model": "paraphrase-multilingual-MiniLM-L12-v2",
        "dim": 384,
        "normalize": "l2",
    },
    "classes": [
        {
            "name": "billing",
            "description": "Charge / refund / invoicing issues.",
            "canonical_sentences": [
                "I want a refund",
                "my card was charged twice",
                "please cancel my subscription",
                "where is my invoice",
            ],
            "color": "#ef4444",
            "deprecated": False,
        },
        {
            "name": "login",
            "description": "Authentication / access problems.",
            "canonical_sentences": [
                "I cannot log in",
                "my password reset email never arrived",
                "two-factor code keeps failing",
            ],
            "color": "#3b82f6",
            "deprecated": False,
        },
        {
            "name": "feature_request",
            "description": "User wants a new feature.",
            "canonical_sentences": [
                "could you add dark mode",
                "would love a CSV export",
                "please support OAuth",
            ],
            "color": "#10b981",
            "deprecated": False,
        },
    ],
    "cone_policy": {"max_projection_distance": 0.7},
    "thresholds": {"confidence_high": 0.8, "confidence_medium": 0.5},
}

CHURN_CLASS: dict[str, Any] = {
    "name": "churn_signal",
    "description": "User signalling intent to leave.",
    "canonical_sentences": [
        "I am thinking about cancelling",
        "I am going to switch to a competitor",
        "this is the last straw",
    ],
    "color": "#f59e0b",
    "deprecated": False,
}


# ─────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────


def _print_registered(reg: RegisteredCatalog, *, label: str) -> None:
    print(f"  ✓ {label}")
    print(f"    key:           {reg.key}")
    print(f"    tenant/name:   {reg.tenant} / {reg.name}")
    print(f"    version:       {reg.version}")
    print(f"    n_classes:     {len(reg.content.get('classes', []))}")
    print(f"    created_at:    {reg.metadata.created_at.isoformat()}")
    print(f"    created_by:    {reg.metadata.created_by!r}")
    print(f"    evolution_op:  {reg.metadata.evolution_op}")
    print(f"    parent_key:    {reg.metadata.parent_key or '(none — v1)'}")
    print(f"    sha256:        {reg.metadata.content_sha256[:16]}…")


def _env_or_die(name: str) -> str:
    val = os.environ.get(name)
    if not val:
        print(f"FATAL: env var {name} is not set", file=sys.stderr)
        sys.exit(2)
    return val


# ─────────────────────────────────────────────────────────────────────
# Main lifecycle
# ─────────────────────────────────────────────────────────────────────


def main() -> int:
    endpoint = os.environ.get("EFE_HOST", "127.0.0.1:50051")
    jwt = _env_or_die("EFE_TOKEN")
    insecure = os.environ.get("EFE_TLS", "true").lower() in ("0", "false", "no")

    # Pick a unique catalog name so the demo is rerunnable.
    catalog_name = f"d12_demo_{int(time.time())}_{uuid.uuid4().hex[:6]}"

    print(f"Sprint D12 catalog admin demo")
    print(f"  endpoint:  {endpoint}")
    print(f"  insecure:  {insecure}")
    print(f"  catalog:   {catalog_name}")
    print()

    with EfeAdminClient(
        endpoint=endpoint, jwt=jwt, insecure=insecure, timeout_s=30.0,
    ) as admin:
        # ── 1. register v1 ────────────────────────────────────────
        print("[1/8] register v1")
        try:
            v1 = admin.register_catalog(
                tenant=admin._tenant_from_jwt(),  # noqa: SLF001 — demo
                name=catalog_name,
                version="v1",
                content=INITIAL_CONTENT,
                description="Initial demo catalog (3 classes).",
                labels={"env": "demo", "owner": "d12-example"},
            )
            _print_registered(v1, label="v1 registered")
        except EfeInvalidArgumentError as e:
            print(f"  ✗ schema error: {e}", file=sys.stderr)
            return 4
        except EfeAuthError as e:
            print(f"  ✗ auth error: {e}", file=sys.stderr)
            return 3

        # ── 2. list catalogs ──────────────────────────────────────
        print()
        print("[2/8] list_catalogs")
        refs = admin.list_catalogs()
        print(f"  {len(refs)} catalog(s) visible to tenant {v1.tenant!r}")
        for r in refs[:5]:
            print(f"    - {r.tenant}/{r.name} @v{r.version} "
                  f"({r.num_classes} classes, {r.num_anchors} anchors)")
        if len(refs) > 5:
            print(f"    ... ({len(refs) - 5} more)")

        # ── 3. list versions ──────────────────────────────────────
        print()
        print("[3/8] list_catalog_versions")
        keys = admin.list_catalog_versions(tenant=v1.tenant, name=catalog_name)
        print(f"  {len(keys)} version(s): {keys}")

        # ── 4. get v1 ─────────────────────────────────────────────
        print()
        print("[4/8] get_catalog v1")
        fetched = admin.get_catalog(v1.key)
        assert fetched.version == 1
        assert fetched.metadata.content_sha256 == v1.metadata.content_sha256
        print(f"  ✓ same sha256 ({fetched.metadata.content_sha256[:16]}…)")

        # ── 5. evolve: append churn_signal class → v2 ─────────────
        print()
        print("[5/8] evolve APPEND_CLASS → v2")
        v2 = admin.evolve_catalog(
            from_key=v1.key,
            op=EvolveOp.APPEND_CLASS,
            op_payload={"cls": CHURN_CLASS},
            description="Add churn_signal class.",
        )
        _print_registered(v2, label="v2 (after append)")
        assert v2.version == 2
        assert v2.metadata.parent_key == v1.key

        # ── 6. evolve: raise thresholds → v3 ──────────────────────
        print()
        print("[6/8] evolve UPDATE_THRESHOLDS → v3")
        v3 = admin.evolve_catalog(
            from_key=v2.key,
            op=EvolveOp.UPDATE_THRESHOLDS,
            op_payload={
                "thresholds": {
                    "confidence_high": 0.85,    # 0.80 → 0.85
                    "confidence_medium": 0.55,  # 0.50 → 0.55
                },
            },
            description="Tighten confidence thresholds for prod cutover.",
        )
        _print_registered(v3, label="v3 (after threshold update)")

        # ── 7. get v3 — verify both edits visible ─────────────────
        print()
        print("[7/8] get_catalog v3")
        latest = admin.get_catalog(v3.key)
        n_classes_v3 = len(latest.content["classes"])
        ch_v3 = latest.content["thresholds"]["confidence_high"]
        cm_v3 = latest.content["thresholds"]["confidence_medium"]
        assert n_classes_v3 == 4, f"expected 4 classes, got {n_classes_v3}"
        # Float32 round-trip ; allow tolerance
        assert abs(ch_v3 - 0.85) < 1e-5, f"high threshold {ch_v3} ≠ 0.85"
        assert abs(cm_v3 - 0.55) < 1e-5, f"medium threshold {cm_v3} ≠ 0.55"
        print(f"  ✓ v3 sees 4 classes and (0.85, 0.55) thresholds")

        # ── 8. delete v1 (demo cleanup) ───────────────────────────
        print()
        print("[8/8] delete_catalog v1 (force=True to release reference)")
        try:
            admin.delete_catalog(v1.key, force=True)
            print(f"  ✓ deleted {v1.key}")
        except EfeError as e:
            # Some servers may refuse force-delete in this demo flow ;
            # not fatal for the example.
            print(f"  ⚠ delete refused: {e}")

        print()
        print(f"  Demo done. Catalog {catalog_name} v2/v3 still live.")
        print(f"  To clean up further: efe-admin catalog delete "
              f"{v3.key} --force")

    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except EfeNotFoundError as e:
        print(f"FATAL: not found: {e}", file=sys.stderr)
        sys.exit(2)
    except EfeError as e:
        print(f"FATAL: server error: {e}", file=sys.stderr)
        sys.exit(2)
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        sys.exit(130)

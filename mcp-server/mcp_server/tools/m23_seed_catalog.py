"""efe_seed_catalog (D8) — seed catalog from a list of anchors.

WRITE tool, requires `confirm:true`. Idempotent on
(tenant_id, catalog, bundle_version) triple : returns 409 if the
bundle_version already exists for that tenant+catalog combination.

Side effects :
    1. Validate each anchor (anchor_id non-empty, text ≤ 4096, label
       in tenant's existing label space OR new ⇒ implicit add).
    2. Compute embeddings via efe-server gRPC EmbedBatch.
    3. INSERT into catalog_anchors.
    4. Index into Redis Stack HNSW (FT.CREATE / HSET).
    5. INSERT ui_audit_log entry.

Pin tests : tests/test_m23_seed_catalog.py.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


_MAX_ANCHORS_PER_CALL = 50_000  # hard cap — caller streams in batches if more


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeSeedCatalogInput",
    "description": (
        "Seed a catalog with anchors from a structured list. Validates "
        "+ embeds + indexes (HNSW) + audits in a single transaction. "
        "Idempotent on (tenant_id, catalog, bundle_version). WRITE — "
        "requires `confirm:true`. Default OFF via "
        "`EFE_FEATURE_D8_ADMIN_ENABLED`."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100, "default": "default"},
        "bundle_version": {
            "type": "string",
            "pattern": r"^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[A-Za-z0-9-.]+)?$",
        },
        "anchors": {
            "type": "array",
            "minItems": 1,
            "maxItems": _MAX_ANCHORS_PER_CALL,
            "items": {
                "type": "object",
                "properties": {
                    "anchor_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "text": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "label": {"type": "string", "minLength": 1, "maxLength": 100},
                    "metadata": {"type": "object"},
                },
                "required": ["anchor_id", "text", "label"],
                "additionalProperties": False,
            },
        },
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["tenant_id", "bundle_version", "anchors", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "seed_catalog requires confirm:true",
            code="confirm_required",
        )
    if not hasattr(ctx.backend, "seed_catalog"):
        raise ToolError(
            "Backend does not support seed_catalog (EFE 1.6 required)",
            code="not_implemented",
        )

    tenant = args["tenant_id"]
    catalog = args.get("catalog", "default")
    version = args["bundle_version"]
    anchors = args["anchors"]

    # Defensive : reject duplicate anchor_ids in the input array.
    seen_ids: set[str] = set()
    for a in anchors:
        aid = a["anchor_id"]
        if aid in seen_ids:
            raise ToolError(
                f"duplicate anchor_id '{aid}' in input",
                code="invalid_input",
            )
        seen_ids.add(aid)

    try:
        result = await ctx.backend.seed_catalog(
            tenant_id=tenant,
            catalog=catalog,
            bundle_version=version,
            anchors=anchors,
            auth=ctx.auth,
        )
    except getattr(ctx.backend, "ConflictError", Exception) as e:
        raise ToolError(
            f"bundle_version '{version}' already exists for {tenant}/{catalog}",
            code="conflict",
            status_code=409,
        ) from e
    except Exception as e:
        raise ToolError(
            f"seed_catalog failed: {e}",
            code="backend_error",
        ) from e

    return {
        "tenant_id": tenant,
        "catalog": catalog,
        "bundle_version": version,
        "anchors_seeded": result.get("anchors_seeded", len(anchors)),
        "duration_ms": result.get("duration_ms"),
    }


TOOL = Tool(
    name="efe_seed_catalog",
    description=(
        "Seed a catalog with anchors. Validates + embeds + indexes "
        "(HNSW) + audits atomically. Idempotent on "
        "(tenant_id, catalog, bundle_version). WRITE — requires "
        "`confirm:true`."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)

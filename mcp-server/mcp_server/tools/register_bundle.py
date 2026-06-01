"""efe_register_bundle (M18 D50, write).

Register a bundle URI in the backend's known set. The bundle is NOT
downloaded ; this is metadata-only. `verify_before_register=True`
(default) makes the MCP server pull the manifest + verify the Ed25519
signature locally before forwarding the registration call to the
backend — defense-in-depth against a backend that might trust an
unsigned registration.

Confirm-required pattern (M16 carry-over) : `confirm:true` is
mandatory in the schema. Without it, the dispatcher returns
`invalid_arguments`.

Profile : `agent-native-deploy` only (M18-D50 gating).
"""
from __future__ import annotations

import os
from typing import Any

from .inspect_bundle import handler as _inspect_handler
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeRegisterBundleInput",
    "description": (
        "Register a bundle in the backend's known set. Metadata-only "
        "operation ; the bundle is NOT downloaded. WRITE — requires "
        "confirm:true. Profile=agent-native-deploy only."
    ),
    "properties": {
        "slug": {
            "type": "string",
            "pattern": r"^[a-z][a-z0-9\-]{0,99}$",
            "minLength": 1, "maxLength": 100,
        },
        "version": {
            "type": "string",
            "pattern": r"^v\d+\.\d+\.\d+(-[a-z0-9\-]{1,15})?$",
            "minLength": 1, "maxLength": 50,
        },
        "registry_url": {
            "type": "string",
            "minLength": 1, "maxLength": 200,
        },
        "verify_before_register": {
            "type": "boolean", "default": True,
        },
        "idempotency_key": {
            "type": "string",
            "pattern": r"^[A-Za-z0-9_\-]{1,64}$",
            "maxLength": 64,
        },
        "confirm": {
            "type": "boolean",
            "description": (
                "Must be true to actually register. Confirm-required "
                "pattern (M16 carry-over) : the LLM client surfaces "
                "this argument to the user as an explicit approval."
            ),
        },
    },
    "required": ["slug", "version", "registry_url", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "register_bundle requires confirm:true (write operation)",
            code="confirm_required",
        )

    slug = args["slug"]
    version = args["version"]
    registry_url = args["registry_url"]
    verify_before = bool(args.get("verify_before_register", True))
    idempotency_key = args.get("idempotency_key")

    # Defense-in-depth : when verify_before_register is True (default),
    # we call inspect_bundle locally first. This produces the
    # manifest_sha256 we forward to the backend, AND ensures the
    # bundle is actually accessible at the given URL with a valid
    # Ed25519 signature (when crypto + trust store are configured).
    manifest_sha256: str | None = None
    verify_summary: dict[str, Any] = {}
    if verify_before:
        inspect_args = {
            "slug": slug, "version": version, "registry": registry_url,
        }
        target_info = await _inspect_handler(inspect_args, ctx)
        manifest = target_info.get("manifest") or {}
        manifest_sha256 = manifest.get("manifestSha256")
        sig = target_info.get("signature") or {}
        verify_summary = {
            "is_signed": sig.get("is_signed", False),
            "verified": sig.get("verified", False),
            "signed_by_key_id": sig.get("signed_by_key_id"),
            "error": sig.get("error"),
        }

    try:
        result = await ctx.backend.register_bundle(
            slug=slug,
            version=version,
            registry_url=registry_url,
            manifest_sha256=manifest_sha256,
            verify_before_register=verify_before,
            idempotency_key=idempotency_key,
            auth=ctx.auth,
        )
    except NotImplementedError:
        raise ToolError(
            "register_bundle requires an EFE 1.2 backend "
            "(POST /admin/bundles/register not exposed)",
            code="not_supported", status_code=501,
        )

    return {
        "registered": True,
        "bundle_id": (
            result.get("bundle_id")
            or f"{registry_url.rstrip('/')}/{slug}:{version}"
        ),
        "slug": slug,
        "version": version,
        "registry_url": registry_url,
        "manifest_sha256": manifest_sha256 or result.get("manifest_sha256"),
        "verified_at_register": verify_summary.get("verified", False) if verify_before else None,
        "verify_summary": verify_summary if verify_before else None,
        "audit_event_id": result.get("audit_event_id"),
        "next_action_hint": (
            "Bundle registered. To promote it as 'current' for your "
            "tenant, call efe_promote_bundle. To deploy, generate Helm "
            "values via efe_generate_helm_values and apply manually."
        ),
    }


TOOL = Tool(
    name="efe_register_bundle",
    description=(
        "Register a bundle URI in the backend's known set. Metadata-only "
        "(NOT a deploy). Optionally verifies the Ed25519 signature "
        "locally before forwarding to the backend. WRITE — requires "
        "confirm:true. Profile=agent-native-deploy only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
)

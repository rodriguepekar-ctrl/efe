"""Common helpers for the 5 M23 curation MCP tools.

Implements both execution paths :
  - SAMPLING_DELEGATION : ask the MCP client to sample its own LLM via
    ``ctx.sampling.create_message`` (M16 sampling capability).
  - INTERNAL_LLM : call the EFE-side LLMRouter (M21) via the backend
    client.
"""
from __future__ import annotations

import dataclasses
import os
from typing import Any, Optional

from ..types import ToolContext, ToolError


# ─── Execution mode resolution ─────────────────────────────────────────


@dataclasses.dataclass
class LLMResult:
    content: str
    provider: str
    model: str
    input_tokens: int
    output_tokens: int
    cost_cents: int
    latency_ms: int
    audit_id: Optional[str] = None


def _server_internal_llm_enabled() -> bool:
    return os.environ.get("EFE_FEATURE_M23_USE_INTERNAL_LLM", "false").lower() in ("1", "true", "yes")


def _server_m21_enabled() -> bool:
    return os.environ.get("EFE_FEATURE_M21_LLMROUTER", "false").lower() in ("1", "true", "yes")


def assert_tenant_match(ctx: ToolContext, requested_tenant_id: str) -> None:
    """Verify caller's OIDC tenant claim matches the request param.

    Raises ``ToolError`` (mapped to MCP ``isError`` response) on mismatch.
    """
    caller = ctx.tenant_id
    if not caller:
        raise ToolError("no tenant claim in caller context",
                        code="tenant_required", status_code=401)
    if caller != requested_tenant_id:
        raise ToolError(
            f"caller tenant '{caller}' != requested '{requested_tenant_id}'",
            code="tenant_mismatch", status_code=403,
        )


def resolve_execution_mode(requested: str, tenant_flags: dict[str, Any]) -> str:
    """Validate + resolve execution mode against feature flags.

    Returns the canonical mode string ('SAMPLING_DELEGATION' or
    'INTERNAL_LLM'), or raises ToolError.
    """
    if requested == "INTERNAL_LLM":
        if not _server_m21_enabled():
            raise ToolError(
                "internal_llm requires M21 ON server-wide",
                code="failed_precondition", status_code=412,
            )
        if not _server_internal_llm_enabled():
            raise ToolError(
                "EFE_FEATURE_M23_USE_INTERNAL_LLM is false",
                code="failed_precondition", status_code=412,
            )
        if not tenant_flags.get("m23_curation_allow_internal_llm", False):
            raise ToolError(
                "tenant has not opted into internal_llm",
                code="failed_precondition", status_code=412,
            )
        return "INTERNAL_LLM"
    return "SAMPLING_DELEGATION"


# ─── LLM invocation paths ──────────────────────────────────────────────


async def llm_call_via_sampling(
    ctx: ToolContext, prompt: str, *,
    max_tokens: int = 1500, temperature: float = 0.2,
) -> LLMResult:
    """Delegate the LLM call to the MCP client via the sampling capability."""
    if not ctx.has_sampling():
        raise ToolError(
            "MCP client does not advertise sampling capability ; "
            "use INTERNAL_LLM mode or upgrade your client.",
            code="feature_not_supported", status_code=412,
        )
    try:
        result = await ctx.sampling.create_message(
            messages=[{"role": "user", "content": {"type": "text", "text": prompt}}],
            max_tokens=max_tokens, temperature=temperature,
            include_context="thisServer",
        )
    except Exception as e:  # noqa: BLE001 — propagate as ToolError
        raise ToolError(f"sampling failed : {e}",
                        code="sampling_error", status_code=502) from e

    return LLMResult(
        content=getattr(result, "content_text", None) or str(getattr(result, "content", "")),
        provider="client-sampling",
        model=getattr(result, "model", "client-controlled"),
        input_tokens=int(getattr(result, "input_tokens", 0)),
        output_tokens=int(getattr(result, "output_tokens", 0)),
        cost_cents=0,           # tenant-side cost ; not billed by EFE
        latency_ms=int(getattr(result, "latency_ms", 0)),
        audit_id=None,
    )


async def llm_call_via_internal_router(
    ctx: ToolContext, tenant_id: str, prompt: str, *,
    max_tokens: int = 1500, model_pref: Optional[str] = None,
) -> LLMResult:
    """Call the M21 LLMRouter via the backend client (not via sampling)."""
    if not _server_m21_enabled():
        raise ToolError(
            "internal LLM mode requires M21 LLMRouter to be ON server-wide",
            code="failed_precondition", status_code=412,
        )
    if not _server_internal_llm_enabled():
        raise ToolError(
            "EFE_FEATURE_M23_USE_INTERNAL_LLM is false",
            code="failed_precondition", status_code=412,
        )
    try:
        # `ctx.backend` exposes `.llm_router_route(...)` as established in M21.
        # Returns a dict with content, provider, model, tokens, cost, audit_id.
        resp = await ctx.backend.llm_router_route(
            tenant_id=tenant_id, prompt=prompt,
            max_tokens=max_tokens, model_pref=model_pref,
            budget_category="curation",
        )
    except Exception as e:  # noqa: BLE001
        # Map known backend error codes to ToolError categories.
        code = getattr(e, "code", "backend_error")
        status = getattr(e, "status_code", 502)
        if code == "resource_exhausted" or status == 429:
            raise ToolError(
                f"curation budget exceeded : {e}",
                code="resource_exhausted", status_code=429,
            ) from e
        raise ToolError(f"LLM router error : {e}", code=code, status_code=status) from e

    return LLMResult(
        content=resp["content"],
        provider=resp.get("provider", "internal"),
        model=resp.get("model", ""),
        input_tokens=int(resp.get("input_tokens", 0)),
        output_tokens=int(resp.get("output_tokens", 0)),
        cost_cents=int(resp.get("cost_cents", 0)),
        latency_ms=int(resp.get("latency_ms", 0)),
        audit_id=resp.get("audit_id"),
    )


# ─── Backend integration helpers ───────────────────────────────────────


async def fetch_tenant_flags(ctx: ToolContext, tenant_id: str) -> dict[str, Any]:
    """Fetch tenant feature flags via the backend client.

    Returns at least the M23 flags : ``m23_curation_enabled``,
    ``m23_curation_allow_internal_llm``, ``m23_drift_detection_enabled``.
    Falls back to safe defaults if the call fails.
    """
    try:
        return await ctx.backend.get_tenant_features(tenant_id)
    except Exception:  # noqa: BLE001
        return {
            "m23_curation_enabled": False,
            "m23_curation_allow_internal_llm": False,
            "m23_drift_detection_enabled": False,
        }


async def render_prompt(
    ctx: ToolContext, template_name: str, version: int,
    tenant_id: str, context: dict[str, Any],
) -> dict[str, Any]:
    """Render a curation prompt template via the backend client.

    Returns ``{body, template_name, template_version}``.
    """
    return await ctx.backend.render_prompt(
        template_name=template_name, version=version,
        tenant_id=tenant_id, context=context,
    )


async def insert_proposal(
    ctx: ToolContext, *,
    tenant_id: str, catalog_id: str, kind: str,
    llm_output: str, llm_audit_id: Optional[str],
    prompt_template_name: str, prompt_template_version: int,
    execution_mode: str, idempotency_key: Optional[str],
    extra: Optional[dict[str, Any]] = None,
) -> str:
    """Insert a taxonomy_change_proposal via the backend (gRPC bridge)."""
    payload: dict[str, Any] = {
        "tenant_id": tenant_id, "catalog_id": catalog_id, "kind": kind,
        "llm_output": llm_output, "llm_audit_id": llm_audit_id,
        "prompt_template_name": prompt_template_name,
        "prompt_template_version": prompt_template_version,
        "execution_mode": execution_mode,
        "idempotency_key": idempotency_key,
    }
    if extra: payload.update(extra)
    resp = await ctx.backend.curation_insert_proposal(payload)
    return resp["proposal_id"]

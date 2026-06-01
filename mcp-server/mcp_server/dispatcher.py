"""Tool dispatcher (M16) — validation, rate-limit, audit, error mapping.

The dispatcher is the central enforcement point. It :
  1. Looks up the tool by name (404 if unknown) ;
  2. Validates `args` against the tool's JSON Schema ;
  3. Checks the per-(token, tool) rate limit ;
  4. Calls the tool handler ;
  5. Emits a `mcp_call.<tool>` audit event ;
  6. Maps exceptions to a structured error result.

This logic is factored out of `efe_mcp_server.py` so it can be unit-
tested without going through the MCP protocol layer.
"""
from __future__ import annotations

import logging
import time
from dataclasses import dataclass
from typing import Any, Optional

from jsonschema import Draft202012Validator, ValidationError

from .audit_emitter import AuditEmitter, summarize_args_for_audit
from .auth import AuthContext
from .backend_client import (
    BackendBadRequest,
    BackendClient,
    BackendError,
    BackendForbidden,
    BackendNotFound,
    BackendQuotaExceeded,
    BackendUnavailable,
)
from .profile import (
    Profile,
    prompts_enabled as _prompts_enabled,
    resources_enabled as _resources_enabled,
    sampling_enabled as _sampling_enabled,
    streaming_enabled as _streaming_enabled,
)
from .prompts import PROMPT_DESCRIPTORS, get_prompt, render_prompt
from .rate_limiter import RateLimitExceeded, RateLimiter
from .resources import RESOURCE_DESCRIPTORS, read_resource
from .resources.registry import InvalidResourceUri
from .sampling import SamplingClient, SamplingError as _SamplingError
from .state import BundleCache, EmbeddingCache
from .streaming import OperationCancelled as _OperationCancelled, ProgressEmitter, make_null_progress
from .tools import Tool, ToolContext, ToolError


logger = logging.getLogger("efe_mcp.dispatcher")


@dataclass
class DispatchResult:
    """Structured outcome of a tool call.

    `is_error=True` means the dispatcher should report the call as failed
    to the MCP client (`isError: true` in the tool result). The `content`
    dict is what the LLM client sees.
    """

    is_error: bool
    content: dict[str, Any]
    duration_ms: float
    error_code: Optional[str] = None


class Dispatcher:
    """Central enforcement point for tool calls + resources + prompts.

    M17 additions :
      - `profile` (filters tools/resources/prompts on list)
      - `sampling` (optional SamplingClient)
      - `embedding_cache` (always present)
      - tool weights for the rate limiter
    """

    # Per-tool weight in the rate limiter. Defaults to 1 ; bulk_classify
    # is heavier so it costs 10× a regular request.
    TOOL_WEIGHTS: dict[str, float] = {
        "efe_bulk_classify": 10.0,
        "efe_curate_catalog": 5.0,
        "efe_calibrate_thresholds": 5.0,
        "efe_diagnose_catalog": 3.0,
        "efe_suggest_classes": 5.0,
        "efe_propose_examples": 2.0,
        # M18 — OCI calls are heavier than backend RPCs
        "efe_inspect_bundle": 5.0,
        "efe_diff_bundles": 8.0,
        "efe_list_available_bundles": 3.0,
        # M18 D48 — SBOM tool pulls a JSON blob (typically < 1 MB)
        "efe_get_sbom": 4.0,
        # M18 D49 — dry_run_deploy is the heaviest deploy helper because it
        # composes inspect_bundle + (optionally) backend dry-run RPC.
        "efe_dry_run_deploy": 6.0,
        "efe_generate_helm_values": 4.0,
        "efe_get_deployment_status": 1.0,
        "efe_get_deployment_history": 1.0,
        "efe_generate_rollback_plan": 2.0,
        # M18 D50 — write tools.
        # Audit fix M18-FINAL-A2 (MED) : `efe_register_bundle` weight
        # bumped from 3 → 8 because the handler INTERNALLY calls
        # `_inspect_handler` directly (bypassing the dispatcher), which
        # would normally cost 5× via efe_inspect_bundle. 8 = 3 (own
        # work) + 5 (inferred inspect cost) preserves the M18-D-01
        # OCI rate-limit invariant : an adversarial agent cannot
        # circumvent the OCI flood mitigation by triggering registers.
        "efe_register_bundle": 8.0,
        "efe_promote_bundle": 2.0,
        "efe_request_deployment": 2.0,
        # M20 — LLM-helper tools. Each call triggers a sampling round-
        # trip (cost mostly client-side via sampling delegation), but
        # mcp-server still does context fetch + prompt render +
        # validation. Weights tuned so a tenant cannot DoS the helper
        # category (TM-M20-D-01).
        "efe_summarize_classification": 3.0,
        "efe_compare_bundles_quality": 12.0,  # 2× classify_batch + LLM round
        "efe_generate_test_sentences": 4.0,
        "efe_analyze_confusion_matrix": 4.0,
        "efe_draft_runbook": 4.0,
        # M21 — Mode-2 + LLM budget tools. Mode-2 itself is heavier
        # because it triggers a server-side LLM call (cost charged to
        # tenant budget). Per-tenant rate limit + budget cap enforce
        # TM-M21-D-01 (DoS via Mode-2 spam).
        "efe_classify_mode2": 8.0,         # backend LLMRouter round-trip
        "efe_get_llm_budget": 1.0,         # cheap Postgres read
        "efe_set_llm_budget": 2.0,         # write, single Postgres upsert
        "efe_list_llm_providers": 2.0,     # admin RPC, modest
        "efe_invalidate_llm_cache": 5.0,   # may scan Redis namespace
        # M22 — Mode-3 multi-turn tools. Mode-3 is N×Mode-2 calls
        # (default N=3, max 5). Heaviest tool in the registry. Per-
        # tenant rate limit + budget cap enforce TM-M22-D-01 (DoS via
        # max_turns burning).
        "efe_classify_mode3": 24.0,        # 3× Mode-2 average
        "efe_get_mode3_reviews": 2.0,      # cheap Postgres list
        "efe_approve_mode3_review": 2.0,   # write, atomic UPDATE
        "efe_reject_mode3_review": 2.0,    # write, atomic UPDATE
        "efe_get_concept_stack": 1.5,      # single Postgres SELECT
    }

    def __init__(
        self,
        *,
        registry: dict[str, Tool],
        backend: BackendClient,
        auth: AuthContext,
        audit: AuditEmitter,
        rate_limiter: RateLimiter,
        profile: Profile = Profile.CORE,
        sampling: SamplingClient | None = None,
        embedding_cache: EmbeddingCache | None = None,
        bundle_cache: BundleCache | None = None,
        oci_factory: Any | None = None,
    ) -> None:
        self.registry = registry
        self.backend = backend
        self.auth = auth
        self.audit = audit
        self.rate_limiter = rate_limiter
        self.profile = profile
        self.sampling = sampling
        self.embedding_cache = embedding_cache or EmbeddingCache()
        self.bundle_cache = bundle_cache or BundleCache()
        self.oci_factory = oci_factory  # callable -> OCIRegistryClient ; set by EFEMCPServer
        # Pre-compile validators ; reused on every call.
        self._validators: dict[str, Draft202012Validator] = {
            name: Draft202012Validator(t.input_schema) for name, t in registry.items()
        }

    # ─── Public list helpers (for the MCP protocol layer) ─────────

    def list_tools(self) -> list[dict[str, Any]]:
        return [t.to_mcp_dict() for t in self.registry.values()]

    def list_resources(self) -> list[dict[str, Any]]:
        if not _resources_enabled(self.profile):
            return []
        return [r.to_mcp_dict() for r in RESOURCE_DESCRIPTORS]

    def list_prompts(self) -> list[dict[str, Any]]:
        """Return prompts filtered by profile tier (M18-FINAL-A3 fix).

        Pre-fix : every prompt was returned regardless of profile, so
        `deploy_new_bundle` (which references agent-native-deploy
        tools) was visible in agent-native and led to mid-workflow
        tool-not-found errors. Now `profile_tier` gates each prompt.
        """
        if not _prompts_enabled(self.profile):
            return []
        return [
            p.to_mcp_dict()
            for p in PROMPT_DESCRIPTORS
            if _prompt_visible_in_profile(p.profile_tier, self.profile)
        ]

    def supports_sampling(self) -> bool:
        return _sampling_enabled(self.profile) and self.sampling is not None

    def supports_streaming(self) -> bool:
        return _streaming_enabled(self.profile)

    # ─── resources/read ──────────────────────────────────────────

    async def read_resource(self, uri: str) -> dict[str, Any] | None:
        """Resolve a resource URI. Returns None if resources not enabled.

        Audit fix M17-A5 (MED) : every successful + failed read is
        emitted into the D14-bis audit chain so the HMAC-tamper-evident
        trail covers MCP resource access too. Previously, an attacker
        could browse the catalog via resources without leaving a trace
        in the audit log.
        """
        if not _resources_enabled(self.profile):
            return {
                "error": "resources are not exposed on this profile",
                "code": "resources_disabled",
            }
        started = time.time()
        try:
            content = await read_resource(
                uri=uri,
                auth=self.auth,
                backend=self.backend,
                bundle_cache=self.bundle_cache,
                oci_factory=self.oci_factory,
            )
        except InvalidResourceUri as exc:
            await self._emit_safe_resource(
                uri=uri, ok=False, started=started, code="invalid_resource_uri",
            )
            return {
                "error": str(exc),
                "code": "invalid_resource_uri",
                "status_code": 400,
            }
        except BackendError as exc:
            mapped = _map_backend_error(exc)
            await self._emit_safe_resource(
                uri=uri, ok=False, started=started, code=mapped["code"],
            )
            return {
                "error": str(exc),
                "code": mapped["code"],
                "status_code": getattr(exc, "status_code", None),
            }
        except Exception as exc:  # noqa: BLE001
            logger.exception("read_resource failed")
            await self._emit_safe_resource(
                uri=uri, ok=False, started=started, code="internal_error",
            )
            return {"error": f"internal error : {exc!r}", "code": "internal_error"}
        await self._emit_safe_resource(uri=uri, ok=True, started=started, code=None)
        return content

    async def _emit_safe_resource(
        self, *, uri: str, ok: bool, started: float, code: str | None,
    ) -> None:
        """Emit a `mcp_resource_read` audit event ; never raises."""
        try:
            await self.audit.emit(
                tool="mcp_resource_read",
                auth=self.auth,
                duration_ms=(time.time() - started) * 1000.0,
                ok=ok,
                error_code=code,
                arg_summary={
                    "uri": uri[:200],
                    "kind": "mcp_resource",
                    "transport": "mcp",
                },
            )
        except Exception as exc:  # noqa: BLE001
            logger.warning("audit emit raised (suppressed) : %r", exc)

    # ─── prompts/get ──────────────────────────────────────────────

    def get_prompt(
        self, name: str, args: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        if not _prompts_enabled(self.profile):
            return {
                "error": "prompts are not exposed on this profile",
                "code": "prompts_disabled",
            }
        template = get_prompt(name)
        if template is None:
            return {
                "error": f"unknown prompt: {name!r}",
                "code": "unknown_prompt",
            }
        try:
            text = render_prompt(name, args or {})
        except (KeyError, ValueError) as exc:
            return {"error": str(exc), "code": "invalid_arguments"}
        return {
            "description": template.description,
            "messages": [
                {
                    "role": "user",
                    "content": {"type": "text", "text": text},
                }
            ],
        }

    async def dispatch(
        self,
        *,
        tool_name: str,
        args: dict[str, Any],
        progress_token: str | None = None,
        progress_session: Any | None = None,
    ) -> DispatchResult:
        """Dispatch a tool call.

        `progress_token` + `progress_session` together enable streaming
        notifications/progress (M17-A2 fix). Either being None disables
        streaming for this call.
        """
        started = time.time()
        args = dict(args or {})

        # 1. Tool lookup
        tool = self.registry.get(tool_name)
        if tool is None:
            return self._error_result(
                started,
                code="unknown_tool",
                message=(
                    f"unknown tool {tool_name!r}. Available: "
                    f"{sorted(self.registry.keys())}"
                ),
            )

        # 2. Schema validation
        validator = self._validators[tool_name]
        try:
            validator.validate(args)
        except ValidationError as exc:
            return self._error_result(
                started,
                code="invalid_arguments",
                message=f"invalid arguments for {tool_name!r} : {exc.message}",
                tool=tool_name,
            )

        # 3. Rate limit (weighted per tool ; TM-M17-D-01)
        weight = self.TOOL_WEIGHTS.get(tool_name, 1.0)
        try:
            self.rate_limiter.check(
                raw_token=self.auth.raw_token, tool=tool_name, cost=weight,
            )
        except RateLimitExceeded as exc:
            await self._emit_safe(
                tool_name=tool_name, ok=False, code="rate_limit", started=started, args=args
            )
            return self._error_result(
                started,
                code="rate_limit_exceeded",
                message=str(exc),
                extra={"rate_limit": exc.to_dict()},
                tool=tool_name,
            )

        # 4. Token expiry guard (best-effort — backend is source of truth)
        if self.auth.is_expired():
            await self._emit_safe(
                tool_name=tool_name, ok=False, code="token_expired", started=started, args=args
            )
            return self._error_result(
                started,
                code="token_expired",
                message=(
                    "OIDC token has expired ; restart the MCP server with a "
                    "fresh EFE_OIDC_TOKEN environment variable."
                ),
                tool=tool_name,
            )

        # 5. Handler
        # Reset sampling usage for fresh tool call.
        if self.sampling is not None:
            self.sampling.reset_usage()
        # Build progress emitter (M17-A2 fix : wire session through).
        # `progress_session` is supplied by the MCP SDK transport when
        # available ; the JSON-RPC fallback doesn't have a session, so
        # streaming is correctly degraded to no-op.
        progress: ProgressEmitter
        if (
            self.supports_streaming()
            and progress_token
            and progress_session is not None
        ):
            progress = ProgressEmitter(progress_session, progress_token)
        else:
            # M17-A6 fix : fresh per-dispatch emitter to avoid sharing
            # mutable state via the NULL_PROGRESS singleton.
            progress = ProgressEmitter(None, None)
        ctx = ToolContext(
            auth=self.auth,
            backend=self.backend,
            sampling=self.sampling if self.supports_sampling() else None,
            progress=progress,
            embedding_cache=self.embedding_cache,
            tenant_id=self.auth.tenant_id or "",
            bundle_cache=self.bundle_cache,
            oci_factory=self.oci_factory,
        )
        try:
            result = await tool.handler(args, ctx)
        except ToolError as exc:
            await self._emit_safe(
                tool_name=tool_name, ok=False, code=exc.code, started=started, args=args
            )
            return self._error_result(
                started,
                code=exc.code,
                message=str(exc),
                tool=tool_name,
                status_code=exc.status_code,
            )
        except _OperationCancelled as exc:  # type: ignore[misc]
            await self._emit_safe(
                tool_name=tool_name, ok=False, code="cancelled",
                started=started, args=args,
            )
            return self._error_result(
                started, code="cancelled", message=str(exc), tool=tool_name,
            )
        except _SamplingError as exc:  # type: ignore[misc]
            await self._emit_safe(
                tool_name=tool_name, ok=False, code="sampling_failed",
                started=started, args=args,
            )
            return self._error_result(
                started,
                code="sampling_failed",
                message=str(exc),
                tool=tool_name,
            )
        except BackendError as exc:
            mapped = _map_backend_error(exc)
            await self._emit_safe(
                tool_name=tool_name,
                ok=False,
                code=mapped["code"],
                started=started,
                args=args,
            )
            return self._error_result(
                started,
                code=mapped["code"],
                message=str(exc),
                tool=tool_name,
                status_code=getattr(exc, "status_code", None),
            )
        except Exception as exc:  # noqa: BLE001 — final safety net
            logger.exception("unhandled error in tool %s", tool_name)
            await self._emit_safe(
                tool_name=tool_name, ok=False, code="internal_error", started=started, args=args
            )
            return self._error_result(
                started,
                code="internal_error",
                message=f"internal error : {exc!r}",
                tool=tool_name,
            )

        # 6. Success — emit and return
        duration_ms = (time.time() - started) * 1000.0
        await self._emit_safe(
            tool_name=tool_name,
            ok=True,
            code=None,
            started=started,
            args=args,
        )
        return DispatchResult(is_error=False, content=result, duration_ms=duration_ms)

    async def _emit_safe(
        self,
        *,
        tool_name: str,
        ok: bool,
        code: Optional[str],
        started: float,
        args: dict[str, Any],
    ) -> None:
        try:
            await self.audit.emit(
                tool=tool_name,
                auth=self.auth,
                duration_ms=(time.time() - started) * 1000.0,
                ok=ok,
                error_code=code,
                arg_summary=summarize_args_for_audit(tool_name, args),
            )
        except Exception as exc:  # noqa: BLE001
            logger.warning("audit emit raised (suppressed) : %r", exc)

    def _error_result(
        self,
        started: float,
        *,
        code: str,
        message: str,
        tool: Optional[str] = None,
        status_code: Optional[int] = None,
        extra: Optional[dict[str, Any]] = None,
    ) -> DispatchResult:
        duration_ms = (time.time() - started) * 1000.0
        content = {"error": message, "code": code}
        if tool:
            content["tool"] = tool
        if status_code is not None:
            content["status_code"] = status_code
        if extra:
            content.update(extra)
        return DispatchResult(
            is_error=True,
            content=content,
            duration_ms=duration_ms,
            error_code=code,
        )


def _prompt_visible_in_profile(prompt_tier: str, active: Profile) -> bool:
    """Audit fix M18-FINAL-A3 (LOW) : prompt visibility ladder.

    Tiers, in increasing capability :
      "advanced" < "agent-native" < "agent-native-deploy"

    A prompt is visible iff the active profile is at least its tier.
    Unknown tiers are treated as "advanced" (most permissive) so a
    missing `profile_tier` doesn't accidentally hide pre-existing
    prompts.
    """
    tier = prompt_tier or "advanced"
    if tier == "advanced":
        return active in (
            Profile.ADVANCED, Profile.AGENT_NATIVE,
            Profile.AGENT_NATIVE_DEPLOY, Profile.EXPERIMENTAL,
        )
    if tier == "agent-native":
        return active in (
            Profile.AGENT_NATIVE, Profile.AGENT_NATIVE_DEPLOY,
            Profile.EXPERIMENTAL,
        )
    if tier == "agent-native-deploy":
        return active in (
            Profile.AGENT_NATIVE_DEPLOY, Profile.EXPERIMENTAL,
        )
    # Unknown tier — be permissive (advanced+).
    return active != Profile.CORE


def _map_backend_error(exc: BackendError) -> dict[str, Any]:
    """Map BackendError subclass to a stable MCP error code."""
    if isinstance(exc, BackendBadRequest):
        return {"code": "invalid_arguments"}
    if isinstance(exc, BackendForbidden):
        return {"code": "forbidden"}
    if isinstance(exc, BackendNotFound):
        return {"code": "not_found"}
    if isinstance(exc, BackendQuotaExceeded):
        return {"code": "quota_exceeded"}
    if isinstance(exc, BackendUnavailable):
        return {"code": "backend_unavailable"}
    return {"code": getattr(exc, "backend_code", None) or "backend_error"}

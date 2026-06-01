"""Audit emission (M16).

Every MCP tool call results in an audit event being emitted to the EFE
backend's audit log so the D14-bis HMAC chain remains tamper-evident
end-to-end (LLM client → MCP server → EFE backend).

The MCP server itself does NOT keep its own audit log — that would
fragment the chain. Instead, it relies on the backend's audit log and
adds an `mcp_call.<tool_name>` action with metadata about the call
(arguments are scrubbed of PII per D13).
"""
from __future__ import annotations

import logging
import os
import time
from dataclasses import dataclass
from typing import Any, Optional

from .auth import AuthContext
from .backend_client import BackendClient, BackendError


logger = logging.getLogger("efe_mcp.audit")


@dataclass
class AuditEvent:
    """In-memory representation of an MCP tool call event."""

    tool: str
    timestamp: float
    duration_ms: float
    ok: bool
    error_code: Optional[str] = None
    arg_summary: Optional[dict[str, Any]] = None


class AuditEmitter:
    """Best-effort audit emitter.

    On every tool call : POST /audit/emit (REST) or AuditEmit RPC (gRPC).
    Failure to emit is logged but does not fail the user-facing call —
    the *backend's own* internal audit (e.g. catalog.register) is the
    source of truth ; this `mcp_call.*` event is supplementary metadata
    indicating that the call came in via MCP rather than direct gRPC.

    The `EFE_MCP_AUDIT_EMIT=false` toggle disables emission entirely
    (useful in tests and for backends that don't yet expose the
    `/audit/emit` endpoint — e.g. M15 playground today).
    """

    def __init__(
        self,
        backend: BackendClient,
        *,
        enabled: Optional[bool] = None,
    ) -> None:
        self._backend = backend
        if enabled is None:
            enabled = os.environ.get("EFE_MCP_AUDIT_EMIT", "true").strip().lower() != "false"
        self.enabled = enabled
        self._events: list[AuditEvent] = []

    async def emit(
        self,
        *,
        tool: str,
        auth: AuthContext,
        duration_ms: float,
        ok: bool,
        error_code: Optional[str] = None,
        arg_summary: Optional[dict[str, Any]] = None,
    ) -> None:
        event = AuditEvent(
            tool=tool,
            timestamp=time.time(),
            duration_ms=duration_ms,
            ok=ok,
            error_code=error_code,
            arg_summary=arg_summary,
        )
        # Local in-memory ring (last 500 events) — useful for the
        # health endpoint of the MCP server itself.
        self._events.append(event)
        if len(self._events) > 500:
            self._events = self._events[-500:]

        if not self.enabled:
            return

        # Fire-and-forget : we log on failure but don't raise. The
        # primary audit channel is the EFE backend's *own* internal
        # audit chain (e.g. catalog.register event) ; this is purely
        # MCP-call metadata.
        try:
            # The playground does not yet expose /audit/emit ; many
            # production EFE deployments will. We attempt a thin
            # extension call via the underlying httpx client where
            # available.
            await self._post_via_rest(event=event, auth=auth)
        except BackendError as exc:
            logger.warning(
                "audit emit failed (non-fatal): tool=%s err=%s status=%s",
                tool,
                exc,
                getattr(exc, "status_code", None),
            )
        except Exception as exc:  # noqa: BLE001 — best-effort path
            logger.warning("audit emit failed (non-fatal): tool=%s err=%r", tool, exc)

    async def _post_via_rest(self, *, event: AuditEvent, auth: AuthContext) -> None:
        """Best-effort POST to /audit/emit. Skips silently on 404."""
        from .backend_client import RestBackendClient

        if not isinstance(self._backend, RestBackendClient):
            # gRPC path (production) will go through a typed AuditEmit
            # RPC ; not implemented in M16 livraison — covered by M16-A* audit pass.
            return
        client = self._backend._client  # noqa: SLF001 — internal collaboration
        try:
            resp = await client.post(
                "/audit/emit",
                headers={
                    "Authorization": f"Bearer {auth.raw_token}",
                    "Content-Type": "application/json",
                },
                json={
                    "action": f"mcp_call.{event.tool}",
                    "metadata": {
                        "ok": event.ok,
                        "duration_ms": round(event.duration_ms, 2),
                        "error_code": event.error_code,
                        "arg_summary": event.arg_summary or {},
                        "transport": "mcp",
                        "mcp_server_version": "0.1.0",
                    },
                },
            )
        except Exception as exc:  # noqa: BLE001
            logger.debug("audit emit transport error: %r", exc)
            return
        if resp.status_code == 404:
            # Backend doesn't expose /audit/emit (e.g. M15 playground) — fine.
            return
        if resp.status_code >= 400:
            logger.warning(
                "audit emit non-2xx: status=%s body=%s",
                resp.status_code,
                resp.text[:200],
            )

    def recent(self, limit: int = 50) -> list[AuditEvent]:
        return self._events[-limit:]

    def stats(self) -> dict[str, Any]:
        total = len(self._events)
        ok = sum(1 for e in self._events if e.ok)
        return {
            "total": total,
            "ok": ok,
            "errors": total - ok,
            "enabled": self.enabled,
        }


# ─────────────────────────────────────────────────────────────────────
# Argument scrubbing (D13 PII hash mode)
# ─────────────────────────────────────────────────────────────────────


import hashlib  # noqa: E402 — placed after class to keep top imports tight
import re  # noqa: E402

_EMAIL_RE = re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b")
_PHONE_RE = re.compile(r"\b(?:\+?\d{1,3}[\s.-]?)?\(?\d{3}\)?[\s.-]?\d{3}[\s.-]?\d{4}\b")
_SSN_RE = re.compile(r"\b\d{3}-\d{2}-\d{4}\b")
_CC_RE = re.compile(r"\b(?:\d[ -]*?){13,19}\b")


def _hash_token(value: str) -> str:
    return f"<PII_HASH:{hashlib.sha256(value.encode('utf-8')).hexdigest()[:8]}>"


def scrub_text(text: str) -> str:
    """Scrub PII (email/phone/SSN/CC) from a single string. Mirror of
    `playground/backend/middleware.py::scrub_pii` — same regexes."""
    if not text:
        return text
    out = _EMAIL_RE.sub(lambda m: _hash_token(m.group(0)), text)
    out = _SSN_RE.sub(lambda m: _hash_token(m.group(0)), out)
    out = _CC_RE.sub(lambda m: _hash_token(m.group(0)), out)
    out = _PHONE_RE.sub(lambda m: _hash_token(m.group(0)), out)
    return out


def summarize_args_for_audit(tool: str, args: dict[str, Any]) -> dict[str, Any]:
    """Build a small, PII-scrubbed summary of tool args suitable for the
    audit log. Long fields are truncated, not omitted, so operators can
    still spot anomalies.

    Audit fix M18-FINAL-A1 (HIGH) : extended to recognise the bundle/
    deploy keys used by M18 D47-D50 (slug, version, tracker, scope,
    registry_url, environment, target/from/to versions, idempotency_key).
    Pre-fix, write tools' audit events lacked the target context →
    no compliance traceability. All values are length-bounded to defend
    against log injection.
    """
    summary: dict[str, Any] = {}
    for k, v in args.items():
        if k == "text":
            text = v if isinstance(v, str) else str(v)
            summary["text_len"] = len(text)
            summary["text_sample"] = scrub_text(text[:80])
        elif k == "catalog" or k == "name":
            summary[k] = str(v)[:80]
        elif k == "classes":
            if isinstance(v, list):
                summary["class_count"] = len(v)
                summary["class_names"] = [
                    str(c.get("name", ""))[:40] for c in v if isinstance(c, dict)
                ][:20]
        elif k == "additional_examples_per_class":
            if isinstance(v, dict):
                summary["evolve_class_count"] = len(v)
                summary["evolve_added_examples"] = sum(
                    len(x) for x in v.values() if isinstance(x, list)
                )
        elif k == "limit":
            summary["limit"] = int(v) if isinstance(v, (int, float)) else None
        elif k == "top_k":
            summary["top_k"] = int(v) if isinstance(v, (int, float)) else None
        elif k == "confirm":
            summary["confirm"] = bool(v)
        # M18 D47-D50 — bundle / deploy fields. Truncate to bound size,
        # never echo full free-text fields (rationale, reason).
        elif k in ("slug", "version", "scope", "tracker", "environment",
                   "target_environment", "release_name", "namespace",
                   "to_version", "version_a", "version_b", "format"):
            summary[k] = str(v)[:80]
        elif k == "registry" or k == "registry_url":
            summary["registry_url"] = str(v)[:200]
        elif k == "rationale":
            text = v if isinstance(v, str) else str(v)
            summary["rationale_len"] = len(text)
            summary["rationale_sample"] = scrub_text(text[:120])
        elif k == "reason":
            text = v if isinstance(v, str) else str(v)
            summary["reason_len"] = len(text)
            summary["reason_sample"] = scrub_text(text[:120])
        elif k == "idempotency_key":
            # Echo length only — the key may carry deploy ID or
            # ticket-system identifier, useful in audit but not
            # the full value (could exceed practical log size).
            summary["idempotency_key_len"] = len(str(v))
        elif k in ("verify_before_register", "dry_run", "rolling_update",
                   "include_full_sbom", "include_resources",
                   "include_pdb", "autoscaling_enabled", "deep"):
            summary[k] = bool(v)
        elif k in ("filter_severity", "filter_slug", "depth"):
            summary[k] = str(v)[:40]
        elif k == "max_components" or k == "replicas":
            summary[k] = int(v) if isinstance(v, (int, float)) else None
        # M20 — LLM-helper specific fields. Echo lengths/counts for
        # traceability without leaking content.
        elif k == "sentence":
            text = v if isinstance(v, str) else str(v)
            summary["sentence_len"] = len(text)
            summary["sentence_sample"] = scrub_text(text[:80])
        elif k == "classification_result":
            if isinstance(v, dict):
                summary["cls_label"] = str(v.get("label", ""))[:80]
                conf = v.get("confidence")
                summary["cls_confidence"] = (
                    round(float(conf), 3) if isinstance(conf, (int, float))
                    else None
                )
                anchors = v.get("anchors_used", [])
                summary["cls_anchors_count"] = (
                    len(anchors) if isinstance(anchors, list) else 0
                )
        elif k == "test_sentences":
            summary["test_sentences_count"] = (
                len(v) if isinstance(v, list) else 0
            )
        elif k == "sample_size":
            summary["sample_size"] = int(v) if isinstance(v, (int, float)) else None
        elif k == "target_class":
            summary["target_class"] = str(v)[:80]
        elif k == "count":
            summary["count"] = int(v) if isinstance(v, (int, float)) else None
        elif k == "difficulty":
            summary["difficulty"] = str(v)[:20]
        elif k == "language" or k == "language_hint":
            summary["language"] = str(v)[:10]
        elif k == "avoid_redundancy_with_existing":
            summary["avoid_redundancy"] = bool(v)
        elif k == "confusion_matrix":
            if isinstance(v, dict):
                summary["confusion_n_classes"] = len(v)
                summary["confusion_n_cells"] = sum(
                    len(row) for row in v.values()
                    if isinstance(row, dict)
                )
        elif k == "min_confusion_rate":
            summary["min_confusion_rate"] = (
                round(float(v), 3)
                if isinstance(v, (int, float)) else None
            )
        elif k == "max_pairs":
            summary["max_pairs"] = int(v) if isinstance(v, (int, float)) else None
        elif k == "incident_event":
            if isinstance(v, dict):
                summary["incident_slo_id"] = str(v.get("slo_id", ""))[:50]
                summary["incident_has_metric"] = bool(v.get("trigger_metric"))
        elif k == "related_logs":
            summary["related_logs_count"] = (
                len(v) if isinstance(v, list) else 0
            )
        elif k == "context_metadata":
            summary["context_metadata_keys"] = (
                len(v) if isinstance(v, dict) else 0
            )
        # M21 — Mode-2 + LLM budget specific fields.
        elif k == "mode":
            summary["mode"] = str(v)[:30]
        elif k == "top_k_anchors":
            summary["top_k_anchors"] = int(v) if isinstance(v, (int, float)) else None
        elif k == "threshold_override":
            summary["threshold_override"] = (
                round(float(v), 3) if isinstance(v, (int, float)) else None
            )
        elif k == "monthly_cap_usd":
            summary["monthly_cap_usd"] = (
                round(float(v), 4) if isinstance(v, (int, float)) else None
            )
        elif k == "soft_threshold_pct":
            summary["soft_threshold_pct"] = (
                int(v) if isinstance(v, (int, float)) else None
            )
        elif k == "fail_open":
            summary["fail_open"] = bool(v)
        elif k == "rollover_dom":
            summary["rollover_dom"] = (
                int(v) if isinstance(v, (int, float)) else None
            )
        elif k == "scope":
            summary["scope"] = str(v)[:30]
        elif k == "include_health":
            summary["include_health"] = bool(v)
        # M22 — Mode-3 + review queue specific fields.
        elif k == "max_turns":
            summary["max_turns"] = int(v) if isinstance(v, (int, float)) else None
        elif k == "threshold":
            summary["threshold"] = (
                round(float(v), 3) if isinstance(v, (int, float)) else None
            )
        elif k == "review_token":
            # UUID v4 = 36 chars ; truncate is no-op but defensive.
            summary["review_token"] = str(v)[:36]
        elif k == "final_label":
            summary["final_label"] = str(v)[:80]
        elif k == "review_notes":
            text = v if isinstance(v, str) else str(v)
            summary["review_notes_len"] = len(text)
            summary["review_notes_sample"] = scrub_text(text[:80])
        elif k == "reason":
            text = v if isinstance(v, str) else str(v)
            summary["reason_len"] = len(text)
            summary["reason_sample"] = scrub_text(text[:120])
        elif k == "audit_id":
            summary["audit_id"] = int(v) if isinstance(v, (int, float)) else None
        elif k == "include_pii_unredacted":
            summary["include_pii_unredacted"] = bool(v)
        elif k == "status":
            summary["status"] = str(v)[:20]
        # Unknown keys are deliberately *not* echoed — defends against
        # log injection via attacker-controlled arg names.
    summary["_tool"] = tool
    return summary

"""PolicyDecisionPoint — the single place that decides allow/deny.

Every business code path calls :meth:`PolicyDecisionPoint.decide` and
never inspects scopes directly. This concentrates the audit surface
and makes policy changes local.
"""

from __future__ import annotations

import asyncio
import logging
import time as _time
from dataclasses import dataclass, field
from typing import Any

from efe_security.authz.abac import AbacRuleSet, evaluate_rules
from efe_security.authz.rbac import ROLE_ACTION_MAP, RbacRegistry
from efe_security.context import Obligation, Principal, Resource
from efe_security.crypto import DOMAIN_TAG_PDP_CACHE, canonical_json, hmac_seal

_LOG = logging.getLogger("efe_security.authz.pdp")

__all__ = ["PolicyDecision", "PolicyDecisionPoint", "PolicyRequest"]


# ---------------------------------------------------------------------------
# Action → required scopes mapping.
# ---------------------------------------------------------------------------

ACTION_SCOPES: dict[str, frozenset[str]] = {
    # Drift
    "drift.evaluation.read": frozenset({"drift:read"}),
    "drift.evaluation.write": frozenset({"drift:write"}),
    "baseline.retire": frozenset({"baseline:retire"}),
    # Scheduling
    "schedule.read": frozenset({"sched:read"}),
    "schedule.create": frozenset({"sched:write"}),
    "schedule.admin": frozenset({"sched:admin"}),
    "trigger.create": frozenset({"sched:write"}),
    "job.read": frozenset({"job:read"}),
    "job.cancel": frozenset({"job:cancel"}),
    "rule.write": frozenset({"rule:write"}),
    "policy.write": frozenset({"policy:write"}),
    # Audit
    "audit.read": frozenset({"audit:read"}),
    "audit.export": frozenset({"audit:export"}),
    # Consent / DSAR
    "consent.write": frozenset({"consent:write"}),
    "dsar.process": frozenset({"dsar:process"}),
    # Tenancy
    "tenant.onboard": frozenset({"tenant:onboard"}),
    "tenant.offboard": frozenset({"tenant:offboard"}),
    # Crypto
    "key.rotate": frozenset({"key:rotate"}),
    # Break-glass
    "break_glass.activate": frozenset({"break_glass:activate"}),
}


@dataclass(frozen=True, slots=True)
class PolicyRequest:
    principal: Principal
    action: str
    resource: Resource | None = None
    context: dict[str, Any] = field(default_factory=dict)

    def cache_key(self) -> bytes:
        view = {
            "principal": {
                "actor_id": self.principal.actor.actor_id,
                "actor_kind": self.principal.actor.kind.value,
                "tenant_id": self.principal.tenant_id,
                "scopes": sorted(self.principal.scopes),
                "jti": self.principal.jti,
            },
            "action": self.action,
            "resource": (
                {
                    "kind": self.resource.kind,
                    "id": self.resource.id,
                    "tenant_id": self.resource.tenant_id,
                    "owner_id": self.resource.owner_id,
                    "labels": sorted(self.resource.labels),
                }
                if self.resource is not None
                else None
            ),
            "context": dict(sorted(self.context.items())) if self.context else {},
        }
        return hmac_seal(DOMAIN_TAG_PDP_CACHE, view)


@dataclass(frozen=True, slots=True)
class PolicyDecision:
    allow: bool
    reasons: tuple[str, ...] = ()
    obligations: tuple[Obligation, ...] = ()
    eval_ms: float = 0.0
    cache_key: bytes = b""


# ---------------------------------------------------------------------------
# Cache
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class _CacheEntry:
    decision: PolicyDecision
    expires_at: float


class PolicyDecisionPoint:
    """The canonical PDP.

    Flow : RBAC check → tenant check → ABAC rules → final verdict.
    """

    __slots__ = ("_abac", "_cache", "_cache_lock", "_rbac", "_ttl_s")

    def __init__(
        self,
        *,
        rbac: RbacRegistry,
        abac: AbacRuleSet | None = None,
        cache_ttl_s: int = 60,
    ) -> None:
        self._rbac = rbac
        self._abac = abac if abac is not None else AbacRuleSet()
        self._ttl_s = cache_ttl_s
        self._cache: dict[bytes, _CacheEntry] = {}
        self._cache_lock = asyncio.Lock()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    async def decide(self, request: PolicyRequest) -> PolicyDecision:
        key = request.cache_key()
        now_mono = _time.monotonic()
        async with self._cache_lock:
            entry = self._cache.get(key)
            if entry is not None and entry.expires_at > now_mono:
                return entry.decision

        t0 = _time.perf_counter()
        decision = await self._evaluate(request)
        dur = (_time.perf_counter() - t0) * 1000.0
        decision = PolicyDecision(
            allow=decision.allow,
            reasons=decision.reasons,
            obligations=decision.obligations,
            eval_ms=dur,
            cache_key=key,
        )
        async with self._cache_lock:
            self._cache[key] = _CacheEntry(
                decision=decision, expires_at=now_mono + self._ttl_s
            )
        return decision

    async def invalidate(self, *, principal_id: str | None = None) -> int:
        """Flush cache entries. If ``principal_id`` given, flush only
        matching entries (heuristic : compare actor_id substring)."""
        async with self._cache_lock:
            if principal_id is None:
                n = len(self._cache)
                self._cache.clear()
                return n
            # Without parsing the key back, we simply flush everything
            # — simpler and safer than a partial invalidation.
            n = len(self._cache)
            self._cache.clear()
            return n

    # ------------------------------------------------------------------
    # Internals
    # ------------------------------------------------------------------

    async def _evaluate(self, req: PolicyRequest) -> PolicyDecision:
        reasons: list[str] = []

        # 0) Deny anonymous everywhere except explicit public actions.
        if req.principal.actor.kind.value == "anonymous":
            if req.action not in {"health.read", "ready.read"}:
                return PolicyDecision(
                    allow=False,
                    reasons=("authn:anonymous_denied",),
                )
            return PolicyDecision(allow=True, reasons=("authn:public_action",))

        # 1) Tenant match on resource (if both present).
        if (
            req.resource is not None
            and req.resource.tenant_id is not None
            and req.principal.tenant_id is not None
            and req.resource.tenant_id != req.principal.tenant_id
        ):
            return PolicyDecision(
                allow=False,
                reasons=("authz:tenant_mismatch",),
            )

        # 2) RBAC — required scopes.
        required = ACTION_SCOPES.get(req.action)
        if required is None:
            return PolicyDecision(
                allow=False,
                reasons=(f"authz:unknown_action:{req.action}",),
            )
        # Effective scopes : those on the JWT ∪ those granted in the registry.
        granted = await self._rbac.scopes_for(
            tenant_id=req.principal.tenant_id or "",
            principal_id=req.principal.actor.actor_id,
        )
        effective = set(req.principal.scopes) | set(granted)
        missing = required - effective
        if missing:
            return PolicyDecision(
                allow=False,
                reasons=tuple(f"rbac:missing:{s}" for s in sorted(missing)),
            )
        reasons.append("rbac:pass")

        # 3) ABAC.
        abac_allow, abac_reasons, obligations_flat = evaluate_rules(
            self._abac,
            action=req.action,
            principal=req.principal,
            resource=req.resource,
            context=req.context,
        )
        reasons.extend(abac_reasons)
        if not abac_allow:
            return PolicyDecision(allow=False, reasons=tuple(reasons))

        obligations = tuple(Obligation(kind=o) for o in obligations_flat)

        return PolicyDecision(
            allow=True,
            reasons=tuple(reasons),
            obligations=obligations,
        )


_ = canonical_json  # keep import

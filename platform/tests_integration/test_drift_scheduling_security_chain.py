"""End-to-end integration test across M09 → M10 → M11.

Scenario :

1. M09 (:mod:`efe_drift`) produces a ``DriftResult`` on a feature with
   critical severity.
2. M10 (:mod:`efe_scheduling`) consumes it via the
   :class:`ContinuousLearningPipeline` and enqueues a
   ``retrain_decision`` JobSpec.
3. M11 (:mod:`efe_security`) audits the authorisation decision and
   seals the audit chain.
4. We verify the audit chain integrity.

The three modules do not import each other ; they share the wire
format only (canonical JSON + HMAC-tagged fields).
"""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

from efe_scheduling.learning import ContinuousLearningPipeline, LearningConfig
from efe_scheduling.triggers import DriftEvent, DriftSeverity
from efe_security.audit import (
    AuditDraft,
    AuditEmitter,
    AuditOutcome,
    AuditSeverity,
    InMemoryAuditStore,
    verify_range,
)
from efe_security.authz import PolicyDecisionPoint, PolicyRequest, RbacRegistry
from efe_security.context import Actor, ActorKind, Principal, Resource, SecurityContext


def test_drift_to_scheduling_to_security_audit() -> None:
    async def scenario() -> None:
        now = datetime(2026, 4, 23, 12, 0, tzinfo=timezone.utc)

        # ----- M09 produces a drift event -----------------------------
        drift = DriftEvent(
            result_id="dr_42",
            tenant_id="t-integration",
            task_id="task-1",
            feature_name="latency_ms",
            method="psi",
            score=0.62,
            severity=DriftSeverity.critical,
            observed_at=now,
            window_start=now - timedelta(hours=1),
            window_end=now,
        )

        # ----- M10 translates it into JobSpec[] -----------------------
        pipeline = ContinuousLearningPipeline(
            cfg=LearningConfig(retrain_min_critical_features=1),
        )
        jobs = await pipeline.on_drift(drift)
        assert jobs, "expected at least one JobSpec from M10"
        retrain = next((j for j in jobs if j.kind.value == "retrain_decision"), None)
        assert retrain is not None

        # Invariant I1 of M10 : fire_key is deterministic for the same
        # (tenant, task, origin_ref, kind). Re-running the pipeline
        # with the same drift yields the same fire_key.
        pipeline2 = ContinuousLearningPipeline(
            cfg=LearningConfig(retrain_min_critical_features=1),
        )
        jobs2 = await pipeline2.on_drift(drift)
        retrain2 = next(j for j in jobs2 if j.kind.value == "retrain_decision")
        assert retrain.fire_key == retrain2.fire_key

        # ----- M11 audits the authorisation decision -----------------
        rbac = RbacRegistry()
        pdp = PolicyDecisionPoint(rbac=rbac)
        principal = Principal(
            actor=Actor(kind=ActorKind.service, actor_id="spiffe://efe/scheduler"),
            tenant_id="t-integration",
            scopes=frozenset({"sched:admin"}),
        )
        ctx = SecurityContext(principal=principal, request_id="int-req-1")

        req = PolicyRequest(
            principal=principal,
            action="schedule.admin",
            resource=Resource(
                kind="job",
                id=retrain.job_id,
                tenant_id="t-integration",
            ),
        )
        decision = await pdp.decide(req)
        assert decision.allow

        audit = InMemoryAuditStore()
        emitter = AuditEmitter(sink=audit)
        # 1) Audit the authz decision.
        await emitter.authz_decision(ctx=ctx, request=req, decision=decision)
        # 2) Audit the "retrain queued" business action.
        await audit.append(
            AuditDraft(
                occurred_at=now,
                tenant_id="t-integration",
                actor_kind="service",
                actor_id="spiffe://efe/scheduler",
                action="retrain.queued",
                outcome=AuditOutcome.success,
                severity=AuditSeverity.info,
                resource_kind="job",
                resource_id=retrain.job_id,
                reasons=(f"drift_score={drift.score:.2f}",),
                payload={
                    "drift_result_id": drift.result_id,
                    "feature": drift.feature_name,
                },
            )
        )

        # ----- Verify the audit chain integrity ----------------------
        outcome = await verify_range(audit)
        assert outcome.ok
        assert outcome.inspected == 2

        # Both entries must reference the same tenant (RLS invariant).
        rows = await audit.fetch_range(tenant_id="t-integration")
        assert len(rows) == 2
        assert all(r.tenant_id == "t-integration" for r in rows)

    asyncio.run(scenario())


def test_tenant_mismatch_blocks_authorisation() -> None:
    """M11 PDP denies a tenant-mismatched access to a M10 resource."""

    async def scenario() -> None:
        pdp = PolicyDecisionPoint(rbac=RbacRegistry())
        principal = Principal(
            actor=Actor(kind=ActorKind.human, actor_id="u-attacker"),
            tenant_id="tenant-A",
            scopes=frozenset({"sched:admin"}),
        )
        req = PolicyRequest(
            principal=principal,
            action="schedule.admin",
            resource=Resource(kind="job", tenant_id="tenant-B"),
        )
        decision = await pdp.decide(req)
        assert not decision.allow
        assert any("tenant_mismatch" in r for r in decision.reasons)

    asyncio.run(scenario())


def test_all_module_versions_reachable() -> None:
    """Sanity : the three modules expose a ``__version__`` and can be
    imported side-by-side without conflict."""
    import efe_scheduling
    import efe_security

    assert efe_scheduling.__version__
    assert efe_security.__version__
    # No mutable cross-module state leaking between them.
    assert efe_scheduling.DOMAIN_TAG_SCHED_FIRE != efe_security.DOMAIN_TAG_AUDIT_CHAIN


def test_hmac_tag_prefix_discipline() -> None:
    """Every DOMAIN_TAG ends with ``/v1\\x00`` and starts with ``efe-<module>/``."""
    import efe_scheduling
    import efe_security

    tags = [
        efe_scheduling.DOMAIN_TAG_SCHED_FIRE,
        efe_scheduling.DOMAIN_TAG_JOB_SEAL,
        efe_scheduling.DOMAIN_TAG_SCHED_EVENT_V1,
        efe_scheduling.DOMAIN_TAG_DATASET_SEED,
        efe_scheduling.DOMAIN_TAG_DATASET_SEAL,
        efe_scheduling.DOMAIN_TAG_PROMOTION_SEAL,
        efe_scheduling.DOMAIN_TAG_IDEMPOTENCY_TOKEN,
        efe_security.DOMAIN_TAG_USER_ID,
        efe_security.DOMAIN_TAG_AUDIT_CHAIN,
        efe_security.DOMAIN_TAG_PDP_CACHE,
        efe_security.DOMAIN_TAG_CONSENT_SEAL,
        efe_security.DOMAIN_TAG_TENANT_DEK,
        efe_security.DOMAIN_TAG_SESSION_OPAQUE,
        efe_security.DOMAIN_TAG_PII_FIELD_HASH,
        efe_security.DOMAIN_TAG_DSAR_RECEIPT,
        efe_security.DOMAIN_TAG_INCIDENT_SEAL,
        efe_security.DOMAIN_TAG_KEY_ROTATION_V1,
    ]
    for tag in tags:
        assert tag.endswith(b"/v1\x00"), f"bad suffix : {tag!r}"
        assert tag.startswith(b"efe-"), f"bad prefix : {tag!r}"
    # No duplicate tags across modules.
    assert len(set(tags)) == len(tags)

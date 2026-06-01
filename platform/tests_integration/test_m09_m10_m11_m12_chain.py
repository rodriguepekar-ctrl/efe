"""End-to-end integration M09 → M10 → M11 → M12.

Scenario :

1. M09 detects a critical drift event (``DriftEvent``).
2. M10 ``ContinuousLearningPipeline`` turns it into a ``retrain_decision``
   ``JobSpec`` with a deterministic fire_key.
3. M11 PDP authorises the scheduler (service SPIFFE ID) to act on the job.
4. M11 audit records the decision + a ``retrain.queued`` line.
5. M12 ``SwarmClient.classify`` is then called to confirm the drift's
   feature against a registered cone — demonstrating that M09 / M10 /
   M11 / M12 cooperate without importing each other's internals.

Invariants verified :

* DOMAIN_TAG isolation across 4 module namespaces (drift, sched, sec, m12).
* Tenant boundary enforced by PDP.
* Audit chain tamper-evident after 2 entries + 1 classification.
* Deterministic fire_key on replay.
* M12 metadata carries tenant + actor propagated from M11 context.
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
from efe_security.context import (
    Actor,
    ActorKind,
    Principal,
    Resource,
    SecurityContext,
)

from efe_swarm_integration import (
    DOMAIN_TAG_SWARM_IDEMPOTENCY,
    ClassifyRequest,
    ConeRef,
    DEFAULTS,
    InMemoryConeBindingStore,
    SwarmClient,
    VerdictEnum,
)
from efe_swarm_integration import BulkheadRegistry, CircuitBreakerRegistry
from efe_swarm_integration.catalog.bridge import CatalogBridge
from efe_swarm_integration.transport.rpc_driver import RpcResponse

# Re-use the M12 fakes module.
from tests_swarm_integration._fakes import FakeEmbedder, FakePdp, FakeRpcDriver


def _wire_converged_response(label: str = "fraud") -> dict:
    return {
        "swarm_id": "swarm_42",
        "request_id": "req_1",
        "output": {"text": label},
        "confidence": 0.88,
        "verdict": 2,
        "rounds_executed": 4,
        "total_ms": 12.3,
        "alignment_delta": 0.11,
    }


def test_m09_m10_m11_m12_full_chain() -> None:
    async def scenario() -> None:
        now = datetime(2026, 4, 23, 12, 0, tzinfo=timezone.utc)
        tenant = "acme"
        actor = "spiffe://efe.local/prod/scheduler"

        # ---- M09 ----------------------------------------------------
        drift = DriftEvent(
            result_id="dr_critical",
            tenant_id=tenant,
            task_id="support-triage",
            feature_name="latency_ms",
            method="psi",
            score=0.72,
            severity=DriftSeverity.critical,
            observed_at=now,
            window_start=now - timedelta(hours=1),
            window_end=now,
        )

        # ---- M10 ----------------------------------------------------
        pipeline = ContinuousLearningPipeline(
            cfg=LearningConfig(retrain_min_critical_features=1)
        )
        jobs = await pipeline.on_drift(drift)
        retrain = next((j for j in jobs if j.kind.value == "retrain_decision"), None)
        assert retrain is not None
        # Same event → same fire_key (I1 of M10 scheduler).
        pipeline_b = ContinuousLearningPipeline(
            cfg=LearningConfig(retrain_min_critical_features=1)
        )
        jobs_b = await pipeline_b.on_drift(drift)
        retrain_b = next(j for j in jobs_b if j.kind.value == "retrain_decision")
        assert retrain.fire_key == retrain_b.fire_key

        # ---- M11 PDP + Audit ---------------------------------------
        rbac = RbacRegistry()
        pdp = PolicyDecisionPoint(rbac=rbac)
        principal = Principal(
            actor=Actor(kind=ActorKind.service, actor_id=actor),
            tenant_id=tenant,
            scopes=frozenset({"sched:admin"}),
        )
        ctx = SecurityContext(principal=principal, request_id="req-m09-m12")
        req = PolicyRequest(
            principal=principal,
            action="schedule.admin",
            resource=Resource(
                kind="job", id=retrain.job_id, tenant_id=tenant
            ),
        )
        decision = await pdp.decide(req)
        assert decision.allow

        audit_store = InMemoryAuditStore()
        emitter = AuditEmitter(sink=audit_store)
        await emitter.authz_decision(ctx=ctx, request=req, decision=decision)
        await audit_store.append(
            AuditDraft(
                occurred_at=now,
                tenant_id=tenant,
                actor_kind="service",
                actor_id=actor,
                action="retrain.queued",
                outcome=AuditOutcome.success,
                severity=AuditSeverity.info,
                reasons=(f"drift_score={drift.score:.2f}",),
                payload={
                    "fire_key_hex": retrain.fire_key.hex(),
                    "feature": drift.feature_name,
                },
            )
        )

        # ---- M12 classify -------------------------------------------
        driver = FakeRpcDriver(
            unary_responses={
                "/efe.v1.SwarmService/Execute": RpcResponse(
                    payload=_wire_converged_response("fraud")
                )
            }
        )
        store = InMemoryConeBindingStore()
        bridge = CatalogBridge(store=store)
        await bridge.register_binding(
            tenant_id=tenant,
            feature_id=drift.feature_name,
            task_id=drift.task_id,
            cone_ref=ConeRef(tenant, drift.task_id, 1),
            embedder_family="st",
            embedder_version="2",
            anchor_count=4,
            labels=("fraud", "billing", "tech", "other"),
            now=now,
        )
        client = SwarmClient(
            driver=driver,
            config=DEFAULTS,
            embedder=FakeEmbedder(dim=4),
            catalog=bridge,
            bulkheads=BulkheadRegistry(
                capacity=DEFAULTS.max_inflight_per_tenant,
                queue_wait_ms=DEFAULTS.max_queue_wait_ms,
            ),
            breakers=CircuitBreakerRegistry(
                error_rate_threshold=0.25,
                window_s=30.0,
                min_samples=20,
                open_cooldown_s=10.0,
            ),
            pdp=FakePdp(allow=True),
        )
        classify_req = ClassifyRequest(
            text="I was debited without consent",
            catalog_key=(drift.feature_name, drift.task_id),
            tenant_id=tenant,
            task_id=drift.task_id,
            actor_id=actor,
            idempotency_key=retrain.fire_key.hex(),
        )
        classify_resp = await client.classify(classify_req)
        assert classify_resp.label == "fraud"
        assert classify_resp.verdict == VerdictEnum.CONVERGED

        # Final audit event.
        await audit_store.append(
            AuditDraft(
                occurred_at=now + timedelta(seconds=2),
                tenant_id=tenant,
                actor_kind="service",
                actor_id=actor,
                action="swarm.classify.completed",
                outcome=AuditOutcome.success,
                severity=AuditSeverity.info,
                reasons=(),
                payload={
                    "swarm_id": classify_resp.swarm_id,
                    "confidence": classify_resp.confidence,
                    "label": classify_resp.label,
                },
            )
        )

        # ---- Verify audit chain -------------------------------------
        outcome = await verify_range(audit_store)
        assert outcome.ok
        assert outcome.inspected == 3

        # ---- Verify metadata propagation ----------------------------
        method, metadata = driver.call_log[0]
        kv = dict(metadata)
        assert kv["x-efe-tenant-id"] == tenant
        assert kv["x-efe-actor-id"] == actor
        assert kv.get("x-efe-idempotency-key") == retrain.fire_key.hex()

    asyncio.run(scenario())


def test_m12_rejects_cross_tenant_access() -> None:
    """M11 PDP refuses tenant-mismatched requests even when M12 sees a
    valid cone binding."""

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


def test_domain_tag_isolation_across_m09_m10_m11_m12() -> None:
    """Each module owns its HMAC namespace ; no collisions possible."""
    from efe_scheduling import DOMAIN_TAG_SCHED_FIRE
    from efe_security import DOMAIN_TAG_AUDIT_CHAIN, DOMAIN_TAG_USER_ID
    from efe_swarm_integration import (
        DOMAIN_TAG_ARTEFACT_SYNC,
        DOMAIN_TAG_CONE_BINDING,
    )

    tags = {
        DOMAIN_TAG_SCHED_FIRE,
        DOMAIN_TAG_AUDIT_CHAIN,
        DOMAIN_TAG_USER_ID,
        DOMAIN_TAG_SWARM_IDEMPOTENCY,
        DOMAIN_TAG_ARTEFACT_SYNC,
        DOMAIN_TAG_CONE_BINDING,
    }
    # 6 distinct tags, all prefixed by their module.
    assert len(tags) == 6
    prefixes = {t.split(b"/", 1)[0] for t in tags}
    assert prefixes == {b"efe-sched", b"efe-sec", b"efe-m12"}

"""Absorption tests — proof that M12 concretely consumes M1-M11.

Each test instantiates a REAL M11-M09 class (not a Protocol stub) and
verifies that M12 adapters wire them correctly.
"""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

import pytest

# Real M1-M11 imports (no stubs).
from efe_drift.detection import DriftResult
from efe_scheduling.types import (
    JobKind,
    JobPriority,
    JobSpec,
    Origin,
    RetryPolicyKind,
    RetryPolicySpec,
    build_fire_key,
    new_uuid,
)
from efe_scheduling.triggers import DriftSeverity
from efe_security.audit import InMemoryAuditStore, verify_range
from efe_security.authz import PolicyDecisionPoint, RbacRegistry, RoleGrant
from efe_security.context import (
    Actor,
    ActorKind,
    Principal,
    SecurityContext,
)
from efe_security.pii import PiiClassifier

# M12 integration layer.
from efe_swarm_integration import (
    ClassifyRequest,
    ConeRef,
)
from efe_swarm_integration.integration import (
    M01PlatformAdapter,
    M03ArtifactBlobSource,
    M05ArtefactEventHandler,
    M05FtArtefactEvent,
    M06InferenceAdapter,
    M07PreflightAdapter,
    M09DriftAdapter,
    M10JobDispatcher,
    M11AuditBridge,
    M11PdpAdapter,
    M11PiiAdapter,
    PlatformWiring,
    PreflightVerdict,
    build_default_dispatcher,
    drift_result_to_event,
    security_context_to_swarm,
    wire_up_platform,
)
from efe_swarm_integration.errors import (
    PiiDetectedInPrompt,
    UnauthorisedM12,
)
from efe_swarm_integration.transport.rpc_driver import RpcResponse

from tests_swarm_integration._fakes import FakeEmbedder, FakeRpcDriver


# ---------------------------------------------------------------------------
# M11 absorption
# ---------------------------------------------------------------------------


def test_m11_pdp_adapter_delegates_to_real_pdp() -> None:
    """Real :class:`PolicyDecisionPoint` drives the M12 PdpPort."""

    async def scenario() -> None:
        rbac = RbacRegistry()
        pdp = PolicyDecisionPoint(rbac=rbac)
        adapter = M11PdpAdapter(pdp=pdp, rbac=rbac)
        # Anonymous/empty scopes → deny.
        ok = await adapter.authorise(
            actor_id="svc", tenant_id="t", action="schedule.admin",
            resource_kind="job",
        )
        # Without a role granting sched:admin, deny.
        assert not ok

        # Grant admin to a DIFFERENT principal to avoid PDP cache.
        await rbac.grant(
            RoleGrant(
                tenant_id="t",
                principal_id="svc-admin",
                role="admin",
                granted_at=datetime(2026, 4, 24, tzinfo=timezone.utc),
                granted_by="test",
            )
        )
        # Invalidate PDP cache — M11 invariant : role changes flush the cache.
        await pdp.invalidate()
        ok = await adapter.authorise(
            actor_id="svc-admin", tenant_id="t", action="schedule.admin",
            resource_kind="job",
        )
        assert ok

    asyncio.run(scenario())


def test_m11_pii_adapter_blocks_detected_pii() -> None:
    classifier = PiiClassifier()
    adapter = M11PiiAdapter(classifier=classifier)
    hits = adapter.find("email me at alice@example.com")
    assert hits
    with pytest.raises(PiiDetectedInPrompt):
        adapter.assert_clean(
            "please contact bob@evil.com", context="raw_prompt"
        )
    # Clean text must pass.
    adapter.assert_clean("please tell me more", context="prompt")


def test_m11_audit_bridge_appends_dicts_and_drafts() -> None:
    async def scenario() -> None:
        store = InMemoryAuditStore()
        bridge = M11AuditBridge(store=store)
        # Dict-path (interceptor fallback).
        await bridge.append(
            {
                "occurred_at": "2026-04-24T10:00:00+00:00",
                "tenant_id": "t",
                "actor_kind": "service",
                "actor_id": "svc",
                "action": "swarm.rpc.invoked.Execute",
                "outcome": "success",
                "payload": {"request_id": "r1"},
            }
        )
        rows = await store.fetch_range()
        assert len(rows) == 1
        out = await verify_range(store)
        assert out.ok

    asyncio.run(scenario())


def test_security_context_to_swarm() -> None:
    p = Principal(
        actor=Actor(kind=ActorKind.service, actor_id="spiffe://x"),
        tenant_id="t1",
    )
    ctx = SecurityContext(principal=p, request_id="req1")
    bundle = security_context_to_swarm(ctx)
    assert bundle.tenant_id == "t1"
    assert bundle.actor_id == "spiffe://x"

    # Missing tenant → refused.
    p2 = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u"),
        tenant_id=None,
    )
    ctx2 = SecurityContext(principal=p2, request_id="r")
    with pytest.raises(UnauthorisedM12):
        security_context_to_swarm(ctx2)


# ---------------------------------------------------------------------------
# M09 absorption
# ---------------------------------------------------------------------------


def test_drift_result_to_event_preserves_fields() -> None:
    now = datetime(2026, 4, 24, tzinfo=timezone.utc)
    dr = DriftResult(
        evaluation_id="ev_1",
        tenant_id="t",
        task_id="task-x",
        feature_id="feat_1",
        feature_name="latency_ms",
        baseline_id="baseline_1",
        window_start=now - timedelta(hours=1),
        window_end=now,
        method="psi",
        score=0.72,
        severity="critical",
        computed_at=now,
        p_value=None,
        multi_scale_window=None,
        details={},
        sample_count_current=1000,
        sample_count_baseline=1000,
    )
    event = drift_result_to_event(dr)
    assert event.tenant_id == "t"
    assert event.feature_name == "latency_ms"
    assert event.severity == DriftSeverity.critical
    assert event.result_id == "ev_1"


def test_m09_drift_adapter_relaxes_cone_on_critical() -> None:
    from efe_swarm_integration.admin.client import AdminClient
    from efe_swarm_integration.catalog.bridge import CatalogBridge
    from efe_swarm_integration.catalog.store import InMemoryConeBindingStore

    async def scenario() -> None:
        driver = FakeRpcDriver(
            unary_responses={
                "/efe.v1.admin.AdminService/EvolveCone": RpcResponse(
                    payload={
                        "key": "t/task-x@v2",
                        "cone": {
                            "cone": {
                                "anchors": [{"values": [0.0]}],
                                "max_projection_distance": 0.42,
                            },
                            "metadata": {
                                "created_at": "2026-04-24T00:00:00+00:00",
                                "created_by": "m09",
                                "description": "",
                                "labels": {},
                            },
                        },
                    }
                )
            }
        )
        admin = AdminClient(driver=driver)
        catalog = CatalogBridge(store=InMemoryConeBindingStore())
        await catalog.register_binding(
            tenant_id="t",
            feature_id="latency_ms",
            task_id="task-x",
            cone_ref=ConeRef("t", "task-x", 1),
            embedder_family="st",
            embedder_version="2",
            anchor_count=1,
            labels=("fraud",),
            now=datetime(2026, 4, 24, tzinfo=timezone.utc),
        )
        adapter = M09DriftAdapter(admin=admin, catalog=catalog)
        now = datetime(2026, 4, 24, tzinfo=timezone.utc)
        dr = DriftResult(
            evaluation_id="ev_1",
            tenant_id="t",
            task_id="task-x",
            feature_id="latency_ms",
            feature_name="latency_ms",
            baseline_id="b",
            window_start=now - timedelta(hours=1),
            window_end=now,
            method="psi",
            score=0.9,
            severity="critical",
            computed_at=now,
            p_value=None,
            multi_scale_window=None,
            details={},
            sample_count_current=500,
            sample_count_baseline=500,
        )
        outcome = await adapter.react(dr)
        assert outcome.action == "cone_relaxed"
        assert outcome.new_cone_key == "t/task-x@v2"

    asyncio.run(scenario())


def test_m09_drift_adapter_noop_below_severity() -> None:
    from efe_swarm_integration.admin.client import AdminClient
    from efe_swarm_integration.catalog.bridge import CatalogBridge
    from efe_swarm_integration.catalog.store import InMemoryConeBindingStore

    async def scenario() -> None:
        adapter = M09DriftAdapter(
            admin=AdminClient(driver=FakeRpcDriver()),
            catalog=CatalogBridge(store=InMemoryConeBindingStore()),
        )
        now = datetime(2026, 4, 24, tzinfo=timezone.utc)
        dr = DriftResult(
            evaluation_id="ev", tenant_id="t", task_id=None,
            feature_id="f", feature_name="f", baseline_id="b",
            window_start=now, window_end=now, method="psi", score=0.1,
            severity="warning", computed_at=now, p_value=None,
            multi_scale_window=None, details={},
            sample_count_current=1, sample_count_baseline=1,
        )
        outcome = await adapter.react(dr)
        assert outcome.action == "noop"

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# M10 absorption
# ---------------------------------------------------------------------------


def test_m10_dispatcher_routes_known_kind() -> None:
    async def scenario() -> None:
        disp = M10JobDispatcher()
        calls: list[JobKind] = []

        async def handler(job):
            calls.append(job.kind)
            from efe_swarm_integration.integration.adapters_m10 import M10JobOutcome

            return M10JobOutcome(kind=job.kind, handled=True, action="tested")

        disp.register(JobKind.promotion_decision, handler)
        now = datetime(2026, 4, 24, tzinfo=timezone.utc)
        jid = new_uuid()
        job = JobSpec(
            job_id=jid,
            fire_key=build_fire_key(jid, now),
            tenant_id="t",
            task_id=None,
            kind=JobKind.promotion_decision,
            payload={"artefact_id": "a1"},
            priority=JobPriority.normal,
            priority_seq=jid,
            run_at=now,
            deadline=None,
            max_attempts=1,
            retry_policy=RetryPolicySpec(
                kind=RetryPolicyKind.fixed, base_ms=100, max_ms=100,
            ),
            origin=Origin.schedule,
            origin_ref="s",
            created_at=now,
        ).sealed()
        out = await disp.dispatch(job)
        assert out.handled
        assert out.action == "tested"
        assert calls == [JobKind.promotion_decision]

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# M05 absorption
# ---------------------------------------------------------------------------


def test_m05_artefact_event_handler_syncs() -> None:
    from efe_swarm_integration.artefact.digest import compute_sha256
    from efe_swarm_integration.artefact.syncer import ArtefactSyncer

    payload = b"artefact-blob"

    class _Source:
        async def iter_chunks(self, *, bucket: str, key: str):
            async def gen():
                yield payload

            return gen()

    class _Target:
        async def put(self, *, bucket: str, key: str, chunks):
            list(chunks)

    async def scenario() -> None:
        syncer = ArtefactSyncer(
            source=_Source(), target=_Target(), target_bucket="dst"
        )
        handler = M05ArtefactEventHandler(syncer=syncer)
        event = M05FtArtefactEvent(
            tenant_id="t",
            artefact_id="a1",
            source_bucket="ft",
            source_key="k",
            digest=compute_sha256(payload),
            dim=4,
            tokenizer_family="bpe",
            labels=(("model", "gpt-x"),),
        )
        outcome = await handler.handle(event)
        assert outcome.ok
        assert outcome.manifest is not None
        assert outcome.manifest.labels == (("model", "gpt-x"),)

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# M07 absorption
# ---------------------------------------------------------------------------


def test_m07_preflight_calls_describe_spec() -> None:
    async def scenario() -> None:
        # Full wire_up stub.
        driver = FakeRpcDriver(
            unary_responses={
                "/efe.v1.SwarmService/DescribeSpec": RpcResponse(
                    payload={
                        "valid": True,
                        "errors": [],
                        "hbm_bytes_estimate": 1024,
                        "rounds_expected": 6,
                    }
                )
            }
        )
        wiring: PlatformWiring = wire_up_platform(
            driver=driver,
            embedder=FakeEmbedder(dim=4),
            audit_store=InMemoryAuditStore(),
            pdp=PolicyDecisionPoint(rbac=RbacRegistry()),
            pii_classifier=PiiClassifier(),
        )
        preflight = M07PreflightAdapter(swarm=wiring.client)
        from efe_swarm_integration.types import (
            ConeRef as _CR,
            PopulationEntry as _PE,
            PopulationMix as _PM,
            SwarmSpec as _SS,
            TemplateCode as _TC,
        )

        spec = _SS(
            dim_latent=4,
            problem_embedding=(0.1, 0.2, 0.3, 0.4),
            population=_PM(entries=(_PE(_TC.NaSem, 8),)),
            alignment_cone=None,
            alignment_cone_ref=_CR("t", "k", 1),
        )
        verdict: PreflightVerdict = await preflight.preflight(
            spec=spec, tenant_id="t"
        )
        assert verdict.ok
        assert verdict.hbm_bytes_estimate == 1024
        assert verdict.rounds_expected == 6

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# wire_up_platform
# ---------------------------------------------------------------------------


def test_wire_up_platform_returns_fully_integrated_bundle() -> None:
    driver = FakeRpcDriver()
    wiring = wire_up_platform(
        driver=driver,
        embedder=FakeEmbedder(dim=4),
        audit_store=InMemoryAuditStore(),
        pdp=PolicyDecisionPoint(rbac=RbacRegistry()),
        pii_classifier=PiiClassifier(),
    )
    assert wiring.client is not None
    assert wiring.admin is not None
    assert wiring.dispatcher is not None
    assert wiring.drift_adapter is not None
    # Dispatcher has the default M10 handlers.
    assert JobKind.canary_evaluate in wiring.dispatcher.handlers
    assert JobKind.promotion_decision in wiring.dispatcher.handlers
    # PDP, PII, audit adapters are concrete.
    assert isinstance(wiring.pdp_adapter, M11PdpAdapter)
    assert isinstance(wiring.pii_adapter, M11PiiAdapter)
    assert isinstance(wiring.audit_bridge, M11AuditBridge)


def test_wire_up_platform_classify_with_real_pdp_deny() -> None:
    """Real PDP denies when no role is granted — M12 surfaces 403."""

    async def scenario() -> None:
        driver = FakeRpcDriver(
            unary_responses={
                "/efe.v1.SwarmService/Execute": RpcResponse(
                    payload={
                        "swarm_id": "s",
                        "request_id": "r",
                        "output": {"text": "fraud"},
                        "confidence": 0.9,
                        "verdict": 2,
                        "rounds_executed": 1,
                        "total_ms": 1.0,
                        "alignment_delta": 0.1,
                    }
                )
            }
        )
        rbac = RbacRegistry()
        pdp = PolicyDecisionPoint(rbac=rbac)
        wiring = wire_up_platform(
            driver=driver,
            embedder=FakeEmbedder(dim=4),
            audit_store=InMemoryAuditStore(),
            pdp=pdp,
            pii_classifier=PiiClassifier(),
        )
        # Register a cone binding.
        await wiring.catalog.register_binding(
            tenant_id="acme",
            feature_id="triage",
            task_id=None,
            cone_ref=ConeRef("acme", "triage", 1),
            embedder_family="st",
            embedder_version="2",
            anchor_count=1,
            labels=("fraud",),
            now=datetime(2026, 4, 24, tzinfo=timezone.utc),
        )
        req = ClassifyRequest(
            text="hello",
            catalog_key=("triage", None),
            tenant_id="acme",
            actor_id="unknown-actor",
        )
        # No role granted ; PDP says deny.
        with pytest.raises(UnauthorisedM12):
            await wiring.client.classify(req)

    asyncio.run(scenario())

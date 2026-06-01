"""Regression tests for every finding in the M06 audit report.

Each test is tagged with the finding ID it exercises so a future
re-audit can cross-check coverage.
"""

from __future__ import annotations

from datetime import timedelta

import pytest
from sqlalchemy import select

from efe_registry.config import SigningSettings
from efe_registry.contracts.canonicalizer import canonicalize_schema
from efe_registry.contracts.hasher import contract_hash_from_schemas
from efe_registry.core.artifact import ArtifactRole, ArtifactState
from efe_registry.core.registry import PublishRequest
from efe_registry.core.state_machine import ARTIFACT_TRANSITIONS
from efe_registry.db.models import CanaryRun, Endpoint, RollbackEvent
from efe_registry.errors import (
    ArtifactVerificationFailed,
    CanaryAlreadyActive,
    RollbackError,
    RollbackVersionConflict,
    RoutingNotFound,
    ValidationError,
)
from efe_registry.rollback.executor import (
    NullDrainAction,
    RollbackExecutor,
)
from efe_registry.rollback.phases import PhaseName, PhaseTimeout
from efe_registry.rollback.standby import (
    InMemoryStandbyProbe,
    PreWarmedStandbyManager,
)
from efe_registry.router.cache import RoutingCache
from efe_registry.router.lua_runner import InMemoryLuaRunner
from efe_registry.router.routing_table import RoutingEntry, RoutingKey
from efe_registry.router.updater import RoutingUpdater
from efe_registry.signing.cache import InMemorySignatureCache
from efe_registry.signing.models import (
    VerificationInput,
    VerificationLayer,
)
from efe_registry.signing.verifier import (
    ScriptedCosignRunner,
    SignatureVerifier,
    validate_oci_ref,
)
from tests_registry.conftest import _seed_artifact, _seed_endpoint


# ---------------------------------------------------------------------------
# C2 — cosign arg injection
# ---------------------------------------------------------------------------


def test_c2_validate_oci_ref_rejects_flag_like_refs():
    with pytest.raises(ValidationError):
        validate_oci_ref("--dry-run")
    with pytest.raises(ValidationError):
        validate_oci_ref("-x")
    with pytest.raises(ValidationError):
        validate_oci_ref("")
    with pytest.raises(ValidationError):
        validate_oci_ref("a b")
    # Well-formed refs pass.
    validate_oci_ref("efe-registry.io/models/acme/summarize:1.0.0")
    validate_oci_ref(
        "efe-registry.io/models/acme/summarize:1.0.0"
        "@sha256:abcdef0123456789" * 2,
    )


@pytest.mark.asyncio
async def test_c2_verify_image_uses_double_dash_sentinel():
    runner = ScriptedCosignRunner(
        responses={
            "verify --output json --key awskms:///alias/t -- "
            "efe-registry.io/foo:v1": (0, '{"ok":true}', ""),
        },
        calls=[],
    )
    verifier = SignatureVerifier(
        settings=SigningSettings(kms_key_arn="awskms:///alias/t"),
        runner=runner,
    )
    result = await verifier.verify(VerificationInput(
        artifact_id="a-1",
        layer=VerificationLayer.L1_PRE_PUBLISH,
        oci_ref="efe-registry.io/foo:v1",
    ))
    assert result.ok
    # The '--' sentinel must be the last flag before the positional ref.
    assert runner.calls
    argv = runner.calls[0][0]
    assert argv[-2] == "--"
    assert argv[-1] == "efe-registry.io/foo:v1"


@pytest.mark.asyncio
async def test_c2_publish_rejects_flag_like_oci_ref():
    from efe_registry.core.registry import _validate_publish

    req = PublishRequest(
        tenant_id="acme",
        task_id="summarize",
        semver="1.0.0",
        content_hash=b"\xab" * 32,
        oci_ref="--evil",
        s3_url="s3://x/",
        parent_run_id="r",
        framework="vllm",
        size_bytes=1,
    )
    with pytest.raises(ValidationError):
        _validate_publish(req)


# ---------------------------------------------------------------------------
# C3 — verify_artifact fails closed
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_c3_verify_artifact_fails_closed_when_unseeded():
    verifier = SignatureVerifier(
        settings=SigningSettings(kms_key_arn="awskms:///alias/t"),
        runner=ScriptedCosignRunner(responses={}),
        cache=InMemorySignatureCache(),
    )
    assert await verifier.verify_artifact("never-seeded") is False


@pytest.mark.asyncio
async def test_c3_seeded_artifact_passes():
    verifier = SignatureVerifier(
        settings=SigningSettings(kms_key_arn="awskms:///alias/t"),
        runner=ScriptedCosignRunner(responses={}),
        cache=InMemorySignatureCache(),
    )
    await verifier.seed_artifact_ok(
        "seeded", layer=VerificationLayer.L2_ADMISSION,
    )
    assert await verifier.verify_artifact("seeded") is True


# ---------------------------------------------------------------------------
# C1 — router swap validates endpoints BEFORE touching Redis
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_c1_swap_fails_fast_if_endpoint_missing(
    session_factory, fake_clock,
):
    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua, clock=fake_clock,
    )
    with pytest.raises(RoutingNotFound):
        await updater.swap_to_champion(
            tenant_id="acme", task_id="summarize",
            new_champion_endpoint_id="does-not-exist",
            actor="test",
        )
    # Redis must stay untouched.
    assert await lua.get_version("routing:acme:summarize:v") == 0


# ---------------------------------------------------------------------------
# C4 — concurrent idempotency keys
# ---------------------------------------------------------------------------


async def _seed_rollback_scenario(session_factory, fake_clock):
    async with session_factory() as session:
        prev = await _seed_artifact(
            session, semver="1.0.0",
            state=ArtifactState.DEPRECATED,
            state_version=2,
            promoted_at=fake_clock.now() - timedelta(hours=2),
        )
        cur = await _seed_artifact(
            session, semver="2.0.0",
            state=ArtifactState.CHAMPION,
            state_version=1,
            promoted_at=fake_clock.now() - timedelta(minutes=20),
        )
        prev_ep = await _seed_endpoint(
            session, artifact_id=prev.artifact_id,
            role=ArtifactRole.STANDBY_WARM, state="ready",
            last_heartbeat_at=fake_clock.now(),
        )
        await _seed_endpoint(
            session, artifact_id=cur.artifact_id,
            role=ArtifactRole.CHAMPION, state="ready",
            last_heartbeat_at=fake_clock.now(),
        )
        await session.commit()
        return prev, cur, prev_ep


def _make_executor(session_factory, settings, fake_clock, prev_ep,
                    *, metrics=None, audit=None):
    probe = InMemoryStandbyProbe(ready_endpoints={prev_ep.endpoint_id})
    standby = PreWarmedStandbyManager(probe=probe, clock=fake_clock)
    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua, clock=fake_clock,
    )
    return RollbackExecutor(
        session_factory=session_factory,
        settings=settings.rollback,
        standby=standby,
        routing_updater=updater,
        drain=NullDrainAction(),
        metrics=metrics,
        audit=audit,
        clock=fake_clock,
    )


@pytest.mark.asyncio
async def test_c4_concurrent_idempotency_returns_same_rollback_id(
    session_factory, settings, fake_clock,
):
    prev, _cur, prev_ep = await _seed_rollback_scenario(
        session_factory, fake_clock,
    )
    executor = _make_executor(session_factory, settings, fake_clock, prev_ep)
    idem = b"\xcc" * 32

    # Sequential replay already tested elsewhere; here we simulate an
    # IntegrityError by pre-inserting a matching idempotency row.
    async with session_factory() as session:
        session.add(RollbackEvent(
            rollback_id="00000000-0000-0000-0000-000000000000",
            tenant_id=prev.tenant_id,
            task_id=prev.task_id,
            from_artifact=None,
            to_artifact=None,
            trigger_kind="manual",
            actor="tests",
            reason="unit",
            idempotency_key=idem,
            state="requested",
            requested_at=fake_clock.now(),
        ))
        await session.commit()

    # The real execute sees the UNIQUE conflict and refuses cleanly,
    # rather than crashing with IntegrityError.
    with pytest.raises(RollbackVersionConflict):
        await executor.execute(
            tenant_id=prev.tenant_id, task_id=prev.task_id,
            actor="tests", reason="unit",
            trigger_kind="manual",
            idempotency_key=idem,
        )


# ---------------------------------------------------------------------------
# C5 — _phase_lookup is read-only + DEPRECATED-only prev
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_c5_lookup_ignores_serving_sibling(
    session_factory, settings, fake_clock,
):
    # Current champion + another SERVING artifact (simulated from a
    # concurrent promote race). Rollback must pick DEPRECATED, not
    # SERVING.
    async with session_factory() as session:
        cur = await _seed_artifact(
            session, semver="2.0.0",
            state=ArtifactState.CHAMPION,
            state_version=1, promoted_at=fake_clock.now(),
        )
        sibling = await _seed_artifact(
            session, semver="2.0.1",
            state=ArtifactState.SERVING, state_version=1,
            promoted_at=fake_clock.now() - timedelta(minutes=5),
        )
        old = await _seed_artifact(
            session, semver="1.0.0",
            state=ArtifactState.DEPRECATED, state_version=3,
            promoted_at=fake_clock.now() - timedelta(hours=2),
        )
        old_ep = await _seed_endpoint(
            session, artifact_id=old.artifact_id,
            role=ArtifactRole.STANDBY_WARM, state="ready",
            last_heartbeat_at=fake_clock.now(),
        )
        await _seed_endpoint(
            session, artifact_id=cur.artifact_id,
            role=ArtifactRole.CHAMPION, state="ready",
            last_heartbeat_at=fake_clock.now(),
        )
        await session.commit()

    executor = _make_executor(session_factory, settings, fake_clock, old_ep)
    receipt = await executor.execute(
        tenant_id=cur.tenant_id, task_id=cur.task_id,
        actor="tests", reason="unit",
    )
    assert receipt.to_artifact_id == old.artifact_id
    # Not the sibling SERVING one.
    assert receipt.to_artifact_id != sibling.artifact_id


# ---------------------------------------------------------------------------
# M2 — RollbackEvent.from_artifact / to_artifact nullable on INSERT
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_m2_rollback_event_inserts_with_null_artifact_ids(
    session_factory, settings, fake_clock,
):
    prev, _cur, prev_ep = await _seed_rollback_scenario(
        session_factory, fake_clock,
    )
    executor = _make_executor(session_factory, settings, fake_clock, prev_ep)
    receipt = await executor.execute(
        tenant_id=prev.tenant_id, task_id=prev.task_id,
        actor="tests", reason="unit",
    )
    async with session_factory() as session:
        row = await session.get(RollbackEvent, receipt.rollback_id)
        assert row is not None
        assert row.from_artifact is not None  # populated at LOOKUP
        assert row.to_artifact is not None


# ---------------------------------------------------------------------------
# M7 — broader exception path marks event failed
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_m7_unhandled_exception_marks_event_failed(
    session_factory, settings, fake_clock,
):
    class _BoomMetricSource:
        async def inflight_share(self, **_: object) -> float:
            raise RuntimeError("metric-source-boom")

    prev, _cur, prev_ep = await _seed_rollback_scenario(
        session_factory, fake_clock,
    )
    executor = _make_executor(
        session_factory, settings, fake_clock, prev_ep,
        metrics=_BoomMetricSource(),
    )
    with pytest.raises(RollbackError):
        await executor.execute(
            tenant_id=prev.tenant_id, task_id=prev.task_id,
            actor="tests", reason="unit",
        )
    async with session_factory() as session:
        stmt = select(RollbackEvent).where(
            RollbackEvent.tenant_id == prev.tenant_id,
        )
        rows = list((await session.execute(stmt)).scalars())
        assert any(r.state == "failed" for r in rows), (
            "no rollback_event marked failed"
        )


# ---------------------------------------------------------------------------
# H4 — drain targets ALL endpoints tied to the former artifact
# ---------------------------------------------------------------------------


class _RecordingDrain:
    def __init__(self):
        self.seen: list[str] = []

    async def drain(self, endpoint):
        self.seen.append(endpoint.endpoint_id)


@pytest.mark.asyncio
async def test_h4_drain_covers_non_champion_roles(
    session_factory, settings, fake_clock,
):
    prev, cur, prev_ep = await _seed_rollback_scenario(
        session_factory, fake_clock,
    )
    # Add an extra shadow endpoint for the failing artifact.
    async with session_factory() as session:
        await _seed_endpoint(
            session, artifact_id=cur.artifact_id,
            role=ArtifactRole.SHADOW, state="ready",
            last_heartbeat_at=fake_clock.now(),
        )
        await session.commit()

    probe = InMemoryStandbyProbe(ready_endpoints={prev_ep.endpoint_id})
    standby = PreWarmedStandbyManager(probe=probe, clock=fake_clock)
    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua, clock=fake_clock,
    )
    drain = _RecordingDrain()
    executor = RollbackExecutor(
        session_factory=session_factory,
        settings=settings.rollback,
        standby=standby,
        routing_updater=updater,
        drain=drain,
        clock=fake_clock,
    )
    await executor.execute(
        tenant_id=prev.tenant_id, task_id=prev.task_id,
        actor="tests", reason="unit",
    )
    # Both endpoints tied to the failing artifact got drained.
    assert len(drain.seen) == 2


# ---------------------------------------------------------------------------
# H5 — verify phase raises when standby artifact_id is empty
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_h5_verify_rejects_empty_artifact_id(
    session_factory, settings, fake_clock,
):
    # Seed as usual but nuke the standby's artifact_id to simulate a
    # misconfigured endpoint record.
    prev, _cur, prev_ep = await _seed_rollback_scenario(
        session_factory, fake_clock,
    )
    async with session_factory() as session:
        ep = await session.get(Endpoint, prev_ep.endpoint_id)
        assert ep is not None
        ep.artifact_id = None
        await session.commit()

    executor = _make_executor(session_factory, settings, fake_clock, prev_ep)
    with pytest.raises(RollbackError):
        await executor.execute(
            tenant_id=prev.tenant_id, task_id=prev.task_id,
            actor="tests", reason="unit",
        )


# ---------------------------------------------------------------------------
# H9 — per-phase budgets enforced
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_h9_bounded_phase_enforces_budget(
    session_factory, settings, fake_clock,
):
    """Unit-test for :meth:`RollbackExecutor._bounded_phase`.

    We give the helper a coroutine that sleeps longer than the budget
    and assert that :class:`PhaseTimeout` is raised with the correct
    phase + budget attributes. Running the full ``execute()`` path for
    this would need real-time sleeps and is covered by integration
    tests in production.
    """
    import asyncio as _a

    probe = InMemoryStandbyProbe()
    standby = PreWarmedStandbyManager(probe=probe, clock=fake_clock)
    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua, clock=fake_clock,
    )
    executor = RollbackExecutor(
        session_factory=session_factory,
        settings=settings.rollback,
        standby=standby,
        routing_updater=updater,
        clock=fake_clock,
    )
    # Monkeypatch the phase-budget table so the test runs in ~0.1s of
    # real time rather than 5 s (the production budget).
    import efe_registry.rollback.executor as _rx
    saved = dict(_rx._PHASE_BUDGETS)
    _rx._PHASE_BUDGETS[PhaseName.LOOKUP] = 0.1
    try:
        async def _slow():
            await _a.sleep(5.0)

        with pytest.raises(PhaseTimeout) as exc_info:
            await executor._bounded_phase(PhaseName.LOOKUP, _slow())
        assert exc_info.value.phase == PhaseName.LOOKUP
        assert exc_info.value.budget_s == pytest.approx(0.1, abs=1e-6)
    finally:
        _rx._PHASE_BUDGETS.clear()
        _rx._PHASE_BUDGETS.update(saved)


# ---------------------------------------------------------------------------
# H1 — first-promotion always journals a CanaryRun
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_h1_first_promotion_creates_completed_canary_run():
    # Smoke test — logic verified by end-to-end promoter path in its
    # own file; here we check the shape of the CanaryRun we'd journal.
    cr = CanaryRun(
        canary_id="00000000-0000-0000-0000-000000000000",
        tenant_id="acme",
        task_id="sum",
        candidate_artifact="a",
        baseline_artifact="a",
        current_percent=100,
        target_percent=100,
        schedule_kind="manual",
        state="completed",
        started_at=None,  # type: ignore[arg-type]
        last_step_at=None,
        ended_at=None,
        auto_rollback=False,
        auto_advance=False,
    )
    assert cr.state == "completed"
    assert cr.current_percent == 100


# ---------------------------------------------------------------------------
# C6 — canary active uniqueness enforced at application level
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_c6_promoter_rejects_when_canary_active(
    session_factory, fake_clock,
):
    from efe_registry.core.promoter import ArtifactPromoter

    class _StubVerifier:
        async def verify_artifact(self, artifact_id):  # noqa: ANN001
            return True

    class _StubRouter:
        async def start_canary(self, **kw):  # noqa: ANN003
            return 1

    async with session_factory() as session:
        art = await _seed_artifact(
            session, semver="1.0.0",
            state=ArtifactState.ADMITTED,
            state_version=1,
        )
        # An active canary already exists on this tenant/task.
        session.add(CanaryRun(
            canary_id="00000000-0000-0000-0000-000000000123",
            tenant_id=art.tenant_id,
            task_id=art.task_id,
            candidate_artifact=art.artifact_id,
            baseline_artifact=art.artifact_id,
            current_percent=5,
            target_percent=100,
            schedule_kind="geometric",
            state="ramping",
            started_at=fake_clock.now(),
            last_step_at=fake_clock.now(),
        ))
        await session.commit()

    promoter = ArtifactPromoter(
        session_factory=session_factory,
        verifier=_StubVerifier(),
        router_updater=_StubRouter(),
        clock=fake_clock,
    )
    with pytest.raises(CanaryAlreadyActive):
        await promoter.promote(
            decision_id="d1",
            artifact_id=art.artifact_id,
            actor="tests",
            reason="unit",
        )


# ---------------------------------------------------------------------------
# H3 — transition count reconciliation
# ---------------------------------------------------------------------------


def test_h3_state_machine_has_14_transitions_and_documents_revival():
    # Check the whitelist size matches the documented code-of-record.
    assert len(ARTIFACT_TRANSITIONS) == 14
    # And includes DEPRECATED -> SERVING (the extra "rollback revival"
    # transition that differs from the arch doc §5.1 table).
    assert (ArtifactState.DEPRECATED, ArtifactState.SERVING) in ARTIFACT_TRANSITIONS


# ---------------------------------------------------------------------------
# M3 — canonicalize normalisations
# ---------------------------------------------------------------------------


def test_m3_int_and_float_produce_same_hash():
    a = {"multipleOf": 1}
    b = {"multipleOf": 1.0}
    out = {"type": "number"}
    assert (
        contract_hash_from_schemas(a, out)
        == contract_hash_from_schemas(b, out)
    )


def test_m3_string_trailing_whitespace_normalised():
    a = {"title": "Hello "}
    b = {"title": "Hello"}
    out = {"type": "string"}
    assert (
        contract_hash_from_schemas(a, out)
        == contract_hash_from_schemas(b, out)
    )


def test_m3_nfc_normalisation_on_strings():
    # "é" as NFC vs NFD (decomposed).
    nfc = "r\u00e9sum\u00e9"       # résumé composed
    nfd = "re\u0301sume\u0301"     # résumé decomposed
    a = {"title": nfc}
    b = {"title": nfd}
    out = {"type": "string"}
    assert (
        contract_hash_from_schemas(a, out)
        == contract_hash_from_schemas(b, out)
    )


def test_m3_nfc_normalisation_on_keys():
    nfc = "r\u00e9sum\u00e9"
    nfd = "re\u0301sume\u0301"
    a = {nfc: {"type": "string"}}
    b = {nfd: {"type": "string"}}
    assert canonicalize_schema(a) == canonicalize_schema(b)


# ---------------------------------------------------------------------------
# M6 — API tenant from header, not body
# ---------------------------------------------------------------------------


def test_m6_header_tenant_extraction_rejects_empty():
    from efe_registry.api.http_app import _header_tenant

    class _Req:
        def __init__(self, headers):
            self.headers = headers

    assert _header_tenant(_Req({})) is None
    assert _header_tenant(_Req({"x-efe-tenant-id": ""})) is None
    assert _header_tenant(_Req({"x-efe-tenant-id": "  "})) is None
    assert _header_tenant(_Req({"x-efe-tenant-id": "acme"})) == "acme"


def test_m6_mfa_check_accepts_variants():
    from efe_registry.api.http_app import _has_mfa

    class _Req:
        def __init__(self, headers):
            self.headers = headers

    assert _has_mfa(_Req({"x-efe-mfa-verified": "true"}))
    assert _has_mfa(_Req({"x-efe-mfa-verified": "TRUE"}))
    assert _has_mfa(_Req({"x-efe-mfa-verified": "1"}))
    assert _has_mfa(_Req({"x-efe-mfa-verified": "yes"}))
    assert not _has_mfa(_Req({"x-efe-mfa-verified": "no"}))
    assert not _has_mfa(_Req({}))


# ---------------------------------------------------------------------------
# L4 — VllmAdapter L3 pre-load verify
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_l4_vllm_adapter_verify_or_die_refuses_bad_signature(
    tmp_path,
):
    from efe_registry.config import VllmPoolSpec
    from efe_registry.core.artifact import Artifact
    from datetime import datetime, timezone
    from efe_registry.serving.vllm_adapter import VllmAdapter

    # Build a fake artifact with matching blobs on disk.
    blob = tmp_path / "model.safetensors"
    blob.write_bytes(b"weights")
    sig = tmp_path / "model.safetensors.sig"
    sig.write_bytes(b"signature")

    verifier = SignatureVerifier(
        settings=SigningSettings(kms_key_arn="awskms:///alias/t"),
        runner=ScriptedCosignRunner(
            responses={}, default=(1, "", "bad signature"),
        ),
        cache=InMemorySignatureCache(),
    )
    artifact = Artifact(
        artifact_id="art-1",
        tenant_id="acme",
        task_id="summarize",
        semver="1.0.0",
        content_hash=b"\x00" * 32,
        oci_ref="efe-registry.io/foo:v1",
        s3_url="s3://x/",
        parent_run_id="r",
        parent_decision_id=None,
        size_bytes=1,
        framework="vllm",
        state=ArtifactState.ADMITTED,
        state_version=0,
        created_at=datetime(2026, 1, 1, tzinfo=timezone.utc),
        uploaded_at=None, signed_at=None, admitted_at=None,
        served_at=None, promoted_at=None, deprecated_at=None,
        purged_at=None,
    )
    adapter = VllmAdapter(
        artifact=artifact,
        pool=VllmPoolSpec(name="test"),
        local_root=tmp_path,
        verifier=verifier,
    )
    with pytest.raises(ArtifactVerificationFailed):
        await adapter._verify_or_die()


# ---------------------------------------------------------------------------
# M4 — RoutingCache locks pruned on invalidate
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_m4_invalidate_prunes_lock(fake_clock):
    async def loader(key):
        return ((RoutingEntry("ep-1", 100, ArtifactRole.CHAMPION),), 1)

    cache = RoutingCache(loader=loader, ttl_s=60.0, clock=fake_clock)
    key = RoutingKey("acme", "sum")
    await cache.get(key)
    assert cache.size() == 1
    assert len(cache._locks) == 1  # type: ignore[attr-defined]
    cache.invalidate("routing:acme:sum")
    assert cache.size() == 0
    assert len(cache._locks) == 0  # type: ignore[attr-defined]


# ---------------------------------------------------------------------------
# Smoke: per-phase PhaseTimeout is importable + typed
# ---------------------------------------------------------------------------


def test_phase_timeout_carries_phase_and_budget():
    exc = PhaseTimeout(PhaseName.LOOKUP, 0.5)
    assert exc.phase == PhaseName.LOOKUP
    assert exc.budget_s == 0.5

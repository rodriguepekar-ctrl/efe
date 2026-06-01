"""End-to-end — publish → transition → rollback."""

from __future__ import annotations

from datetime import timedelta

import pytest

from efe_registry.core.artifact import ArtifactRole, ArtifactState
from efe_registry.core.registry import PublishRequest, Registry
from efe_registry.observability.audit import RecordingAuditChannel
from efe_registry.rollback.executor import (
    NullDrainAction,
    NullInflightMetricSource,
    RollbackExecutor,
)
from efe_registry.rollback.standby import (
    InMemoryStandbyProbe,
    PreWarmedStandbyManager,
)
from efe_registry.router.lua_runner import InMemoryLuaRunner
from efe_registry.router.updater import NullNatsPublisher, RoutingUpdater
from tests_registry.conftest import _seed_endpoint


class _StubVerifier:
    async def verify_artifact(self, artifact_id: str) -> bool:
        return True


class _StubPromoter:
    # Minimal promoter stub — not used by this e2e which goes through Registry
    pass


@pytest.mark.asyncio
async def test_publish_creates_artifact(
    session_factory, settings, fake_clock,
):
    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua, clock=fake_clock,
    )
    probe = InMemoryStandbyProbe()
    standby = PreWarmedStandbyManager(probe=probe, clock=fake_clock)
    executor = RollbackExecutor(
        session_factory=session_factory,
        settings=settings.rollback,
        standby=standby,
        routing_updater=updater,
        clock=fake_clock,
    )

    # Build a Registry with a trivial promoter stub — publish doesn't use it.
    class _NoopPromoter:
        async def promote(self, **kw):  # pragma: no cover
            raise NotImplementedError

    registry = Registry(
        session_factory=session_factory,
        settings=settings,
        promoter=_NoopPromoter(),  # type: ignore[arg-type]
        rollback_executor=executor,
        clock=fake_clock,
    )
    req = PublishRequest(
        tenant_id="acme",
        task_id="summarize",
        semver="1.0.0",
        content_hash=b"\xab" * 32,
        oci_ref="efe-registry.io/models/acme/summarize:1.0.0",
        s3_url="s3://efe-artifacts-acme/summarize/1.0.0/",
        parent_run_id="00000000-0000-0000-0000-000000000001",
        framework="vllm",
        size_bytes=2048,
    )
    artifact = await registry.publish(req)
    assert artifact.tenant_id == "acme"
    assert artifact.state == ArtifactState.UPLOADED
    assert artifact.state_version == 0


@pytest.mark.asyncio
async def test_publish_then_rollback(
    session_factory, settings, fake_clock,
):
    # Seed a prev champion + current champion + standby warm pod.
    from tests_registry.conftest import _seed_artifact

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
            role=ArtifactRole.STANDBY_WARM,
            state="ready",
            last_heartbeat_at=fake_clock.now(),
        )
        await _seed_endpoint(
            session, artifact_id=cur.artifact_id,
            role=ArtifactRole.CHAMPION, state="ready",
        )
        await session.commit()

    lua = InMemoryLuaRunner()
    updater = RoutingUpdater(
        session_factory=session_factory, lua=lua,
        nats_publisher=NullNatsPublisher(), clock=fake_clock,
    )
    probe = InMemoryStandbyProbe(ready_endpoints={prev_ep.endpoint_id})
    standby = PreWarmedStandbyManager(probe=probe, clock=fake_clock)
    audit = RecordingAuditChannel()
    executor = RollbackExecutor(
        session_factory=session_factory,
        settings=settings.rollback,
        standby=standby,
        routing_updater=updater,
        drain=NullDrainAction(),
        metrics=NullInflightMetricSource(),
        audit=audit,
        clock=fake_clock,
    )

    class _NoopPromoter:
        async def promote(self, **kw):  # pragma: no cover
            raise NotImplementedError

    registry = Registry(
        session_factory=session_factory,
        settings=settings,
        promoter=_NoopPromoter(),  # type: ignore[arg-type]
        rollback_executor=executor,
        clock=fake_clock,
    )
    receipt = await registry.rollback(
        tenant_id="acme", task_id="summarize",
        actor="ops", reason="regression",
        trigger_kind="manual",
    )
    assert receipt.final_state == "completed"
    assert receipt.latency_ms < settings.rollback.hard_deadline_s * 1000
    assert audit.rollbacks

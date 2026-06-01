"""Shared fixtures for efe_registry tests."""

from __future__ import annotations

import asyncio
import uuid
from collections.abc import AsyncIterator, Iterator
from datetime import datetime, timezone

import pytest
import pytest_asyncio
from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    AsyncSession,
    async_sessionmaker,
)

from efe_registry.clock import FakeClock
from efe_registry.config import (
    AdmissionSettings,
    CanarySettings,
    ContractSettings,
    DatabaseSettings,
    RegistrySettings,
    RollbackSettings,
    RouterSettings,
    SigningSettings,
)
from efe_registry.core.artifact import ArtifactRole, ArtifactState
from efe_registry.db.engine import build_engine, build_sessionmaker, create_all
from efe_registry.db.models import (
    Artifact,
    Endpoint,
)


@pytest.fixture(scope="session")
def event_loop() -> Iterator[asyncio.AbstractEventLoop]:
    loop = asyncio.new_event_loop()
    yield loop
    loop.close()


@pytest.fixture
def fake_clock() -> FakeClock:
    return FakeClock(datetime(2026, 4, 20, 10, 0, 0, tzinfo=timezone.utc))


@pytest.fixture
def settings() -> RegistrySettings:
    return RegistrySettings(
        environment="test",
        database=DatabaseSettings(
            url="sqlite+aiosqlite:///:memory:",
        ),
        signing=SigningSettings(
            kms_key_arn="awskms:///alias/efe-test",
        ),
        router=RouterSettings(cache_ttl_s=0.05),
        canary=CanarySettings(
            step_duration_s=30,
            auto_rollback=True,
            slo_breach_abort_threshold=3,
        ),
        rollback=RollbackSettings(
            hard_deadline_s=10.0,
            drain_deadline_s=2.0,
            swap_timeout_s=0.5,
            verify_deadline_s=0.5,
            verify_inflight_share_threshold=0.9,
            idempotency_window_s=60,
        ),
        admission=AdmissionSettings(),
        contracts=ContractSettings(
            deprecation_grace_days=90,
            alert_days_before_sunset=(30, 7, 1),
        ),
    )


@pytest_asyncio.fixture
async def engine(settings: RegistrySettings) -> AsyncIterator[AsyncEngine]:
    eng = build_engine(settings.database)
    await create_all(eng)
    yield eng
    await eng.dispose()


@pytest_asyncio.fixture
async def session_factory(
    engine: AsyncEngine,
) -> async_sessionmaker[AsyncSession]:
    return build_sessionmaker(engine)


async def _seed_artifact(
    session: AsyncSession,
    *,
    artifact_id: str | None = None,
    tenant: str = "acme",
    task: str = "summarize",
    semver: str = "1.0.0",
    state: ArtifactState = ArtifactState.UPLOADED,
    state_version: int = 0,
    promoted_at: datetime | None = None,
) -> Artifact:
    ar = Artifact(
        artifact_id=artifact_id or str(uuid.uuid4()),
        tenant_id=tenant,
        task_id=task,
        semver=semver,
        content_hash=bytes.fromhex(
            f"{abs(hash((tenant, task, semver))):064x}"[-64:],
        ),
        oci_ref=f"efe-registry.io/models/{tenant}/{task}:{semver}",
        s3_url=f"s3://efe-artifacts-{tenant}/{task}/{semver}/",
        parent_run_id=str(uuid.uuid4()),
        parent_decision_id=None,
        size_bytes=1024,
        framework="vllm",
        state=state.value,
        state_version=state_version,
        created_at=datetime(2026, 4, 1, tzinfo=timezone.utc),
        uploaded_at=datetime(2026, 4, 1, tzinfo=timezone.utc),
        promoted_at=promoted_at,
    )
    session.add(ar)
    await session.flush()
    return ar


async def _seed_endpoint(
    session: AsyncSession,
    *,
    endpoint_id: str | None = None,
    pool_id: str = "gpu_a100_shared",
    pod_name: str | None = None,
    url: str | None = None,
    artifact_id: str | None = None,
    tenant_id: str = "acme",
    task_id: str = "summarize",
    role: ArtifactRole = ArtifactRole.CHAMPION,
    state: str = "ready",
    last_heartbeat_at: datetime | None = None,
) -> Endpoint:
    eid = endpoint_id or str(uuid.uuid4())
    ep = Endpoint(
        endpoint_id=eid,
        pool_id=pool_id,
        pod_name=pod_name or f"pod-{eid[:8]}",
        namespace="efe-test",
        url=url or f"http://pod-{eid[:8]}:8000",
        tenant_id=tenant_id,
        task_id=task_id,
        artifact_id=artifact_id,
        role=role.value,
        state=state,
        tensor_parallel=1,
        lora_capacity=0,
        readiness_since=datetime(2026, 4, 1, tzinfo=timezone.utc),
        last_heartbeat_at=last_heartbeat_at
        or datetime(2026, 4, 20, 9, 59, 59, tzinfo=timezone.utc),
    )
    session.add(ep)
    await session.flush()
    return ep


@pytest_asyncio.fixture
async def seeded_session(
    session_factory: async_sessionmaker[AsyncSession],
) -> AsyncIterator[AsyncSession]:
    async with session_factory() as session:
        yield session

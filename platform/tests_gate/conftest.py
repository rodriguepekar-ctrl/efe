"""Shared fixtures for tests_gate."""

from __future__ import annotations

import hashlib
import os
import tempfile
from collections.abc import AsyncIterator
from typing import Any

import numpy as np
import pytest
import pytest_asyncio
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_gate.champion.audit import AuditChain
from efe_gate.champion.registry import ChampionRegistry
from efe_gate.clock import FakeClock
from efe_gate.config import GateSettings, SignatureSettings
from efe_gate.db.engine import (
    drop_schema,
    get_engine,
    get_sessionmaker,
    init_schema,
)
from efe_gate.db.models import GoldenSet, PromotionGate
from efe_gate.decision.rule import (
    FairnessRuleSpec,
    MetricRuleSpec,
    PromotionRule,
    ReviewPolicy,
)
from efe_gate.runner import FakeScorer
from efe_gate.services import EvalService
from efe_gate.state_machine import StateMachine


@pytest.fixture(scope="function")
def anyio_backend() -> str:
    return "asyncio"


@pytest_asyncio.fixture
async def sqlite_url() -> AsyncIterator[str]:
    fd, path = tempfile.mkstemp(suffix="_gate.sqlite")
    os.close(fd)
    url = f"sqlite+aiosqlite:///{path}"
    yield url
    try:
        os.unlink(path)
    except OSError:
        pass


@pytest_asyncio.fixture
async def sessionmaker(
    sqlite_url: str,
) -> AsyncIterator[async_sessionmaker[AsyncSession]]:
    engine = get_engine(sqlite_url)
    await init_schema(engine)
    sm = get_sessionmaker(sqlite_url)
    yield sm
    await drop_schema(engine)
    await engine.dispose()


@pytest_asyncio.fixture
async def session(
    sessionmaker: async_sessionmaker[AsyncSession],
) -> AsyncIterator[AsyncSession]:
    async with sessionmaker() as s:
        yield s


@pytest.fixture
def fake_clock() -> FakeClock:
    return FakeClock()


@pytest.fixture
def signing_settings() -> SignatureSettings:
    return SignatureSettings(
        key_id="test", key_hex="a" * 64, algorithm="hmac-sha256",
    )


@pytest.fixture
def settings(
    sqlite_url: str, signing_settings: SignatureSettings,
) -> GateSettings:
    return GateSettings.from_env(overrides={
        "environment": "test",
        "database": {"url": sqlite_url, "enable_rls": False},
        "signature": signing_settings.model_dump(),
        "stats": {"bootstrap_resamples": 200, "rng_seed": 42},
        "rollback": {
            "window_minutes": 5, "tick_seconds": 30,
            "min_live_samples": 10,
            "auto_rollback_min_delta": 0.05,
            "auto_rollback_threshold_p_value": 0.9,
        },
    })


@pytest.fixture
def fake_scorer() -> FakeScorer:
    return FakeScorer()


@pytest_asyncio.fixture
async def audit_chain(
    session: AsyncSession, signing_settings: SignatureSettings,
    fake_clock: FakeClock,
) -> AuditChain:
    return AuditChain(
        session, tenant_id="acme",
        signing=signing_settings, clock=fake_clock,
    )


@pytest_asyncio.fixture
async def state_machine(
    session: AsyncSession, fake_clock: FakeClock,
) -> StateMachine:
    return StateMachine(session, clock=fake_clock)


@pytest_asyncio.fixture
async def registry(
    session: AsyncSession, audit_chain: AuditChain,
    fake_clock: FakeClock,
) -> ChampionRegistry:
    return ChampionRegistry(
        session, audit_chain=audit_chain, clock=fake_clock,
    )


@pytest_asyncio.fixture
async def golden_set(
    session: AsyncSession, fake_clock: FakeClock,
) -> GoldenSet:
    gs = GoldenSet(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///snap/gold",
        content_hash=hashlib.sha256(b"golden").digest(),
        n_samples=100,
        label_balance={"a": 50, "b": 50},
        fairness_attrs={},
        locked_at=fake_clock.now(),
        created_by="curator",
        deprecated=False,
    )
    session.add(gs)
    await session.commit()
    await session.refresh(gs)
    return gs


@pytest_asyncio.fixture
async def promotion_gate(
    session: AsyncSession, fake_clock: FakeClock,
) -> PromotionGate:
    rule = PromotionRule(
        primary=(
            MetricRuleSpec(
                metric="accuracy", min_equivalence_margin=0.01,
            ),
        ),
        budget=(
            MetricRuleSpec(metric="p99_latency_ms", hard_cap=100.0),
        ),
        fairness=FairnessRuleSpec(
            groups=(), metric="accuracy",
            max_per_group_drop=0.1, min_samples_per_group=5,
        ),
        review_policy=ReviewPolicy(
            require_dual_signoff=False, timeout_hours=1,
        ),
    )
    row = PromotionGate(
        tenant_id="acme", task_id="routing",
        rule=rule.as_dict(),
        updated_by="ops", updated_at=fake_clock.now(),
    )
    session.add(row)
    await session.commit()
    return row


@pytest_asyncio.fixture
async def eval_service(
    session: AsyncSession,
    state_machine: StateMachine,
    registry: ChampionRegistry,
    audit_chain: AuditChain,
    fake_scorer: FakeScorer,
    settings: GateSettings,
    fake_clock: FakeClock,
) -> EvalService:
    return EvalService(
        session,
        state_machine=state_machine,
        registry=registry,
        audit_chain=audit_chain,
        scorer=fake_scorer,
        decision_settings=settings.decision,
        stats_settings=settings.stats,
        rollback_settings=settings.rollback,
        review_settings=settings.review,
        clock=fake_clock,
    )


def _bernoulli(p: float, n: int, seed: int) -> np.ndarray:
    rng = np.random.default_rng(seed)
    return (rng.random(n) < p).astype(np.int64)


@pytest.fixture
def bern() -> Any:
    return _bernoulli

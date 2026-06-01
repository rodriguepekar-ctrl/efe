"""DB models — create_all succeeds on SQLite."""

from __future__ import annotations

import pytest

from efe_rollout.config import DatabaseSettings
from efe_rollout.db import Base, build_engine, create_all


@pytest.mark.asyncio
async def test_create_all_creates_all_tables():
    engine = build_engine(
        DatabaseSettings(url="sqlite+aiosqlite:///:memory:"),
    )
    await create_all(engine)
    tables = set(Base.metadata.tables)
    expected = {
        "experiments",
        "variants",
        "experiment_preregistrations",
        "exposures",
        "outcomes",
        "analyses",
        "bandit_state",
        "guardrails",
        "forced_assignments",
        "layer_allocations",
    }
    assert expected <= tables
    await engine.dispose()

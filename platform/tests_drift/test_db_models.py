"""DB models — create_all succeeds on SQLite."""

from __future__ import annotations

import pytest

from efe_drift.config import DatabaseSettings
from efe_drift.db import Base, build_engine, create_all


@pytest.mark.asyncio
async def test_create_all_creates_all_tables():
    engine = build_engine(
        DatabaseSettings(url="sqlite+aiosqlite:///:memory:"),
    )
    await create_all(engine)
    tables = set(Base.metadata.tables)
    expected = {
        "feature_catalog",
        "baselines",
        "baseline_state_transitions",
        "feature_snapshots",
        "raw_events",
        "drift_evaluations",
        "alert_incidents",
    }
    assert expected <= tables
    await engine.dispose()

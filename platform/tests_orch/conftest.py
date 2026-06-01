"""Shared pytest fixtures for tests_orch."""

from __future__ import annotations

import os
import tempfile
from collections.abc import AsyncIterator

import pytest
import pytest_asyncio
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_orch.api.context import OrchContext, clear_context, set_context
from efe_orch.clock import FakeClock
from efe_orch.config import OrchSettings
from efe_orch.db.engine import (
    drop_schema,
    get_engine,
    get_sessionmaker,
    init_schema,
)
from efe_orch.dispatcher import FakeKubernetesClient
from efe_orch.state_machine.locks import InMemoryLockBackend


@pytest.fixture(scope="function")
def anyio_backend() -> str:
    return "asyncio"


@pytest_asyncio.fixture
async def sqlite_url() -> AsyncIterator[str]:
    fd, path = tempfile.mkstemp(suffix="_orch.sqlite")
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
def fake_k8s() -> FakeKubernetesClient:
    return FakeKubernetesClient()


@pytest.fixture
def lock_backend() -> InMemoryLockBackend:
    return InMemoryLockBackend()


@pytest_asyncio.fixture
async def orch_context(
    sqlite_url: str,
    sessionmaker: async_sessionmaker[AsyncSession],
    fake_k8s: FakeKubernetesClient,
    fake_clock: FakeClock,
    lock_backend: InMemoryLockBackend,
) -> AsyncIterator[OrchContext]:
    settings = OrchSettings.from_env(overrides={
        "environment": "test",
        "database": {"url": sqlite_url, "enable_rls": False},
        "k8s": {"dry_run": True},
    })
    ctx = OrchContext(
        settings=settings,
        sessionmaker=sessionmaker,
        k8s_client=fake_k8s,
        clock=fake_clock,
        lock_backend=lock_backend,
    )
    set_context(ctx)
    try:
        yield ctx
    finally:
        clear_context()

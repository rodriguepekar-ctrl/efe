"""Shared pytest fixtures for tests_ft."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator, Iterator
from pathlib import Path

import pytest
import pytest_asyncio
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_ft.api.dependencies import FtContext, clear_context, set_context
from efe_ft.config import FtSettings
from efe_ft.db.engine import (
    drop_schema,
    get_engine,
    get_sessionmaker,
    init_schema,
)
from efe_ft.ingest.dedup import InMemoryDeduper
from efe_ft.labeling.session import InMemorySessionStore
from efe_ft.storage import LocalBlobStore


@pytest.fixture(scope="function")
def anyio_backend() -> str:
    return "asyncio"


@pytest_asyncio.fixture
async def sqlite_url() -> str:
    # Use per-test file so migrations can run/reset clean.
    import tempfile
    import os
    fd, path = tempfile.mkstemp(suffix=".sqlite")
    os.close(fd)
    url = f"sqlite+aiosqlite:///{path}"
    yield url
    try:
        os.unlink(path)
    except OSError:
        pass


@pytest_asyncio.fixture
async def sessionmaker(sqlite_url: str) -> async_sessionmaker[AsyncSession]:
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
def deduper() -> InMemoryDeduper:
    return InMemoryDeduper(num_perm=128, threshold=0.90)


@pytest.fixture
def session_store() -> InMemorySessionStore:
    return InMemorySessionStore()


@pytest.fixture
def blob_store(tmp_path: Path) -> LocalBlobStore:
    return LocalBlobStore(tmp_path / "blobs")


CLASS_SET = frozenset([
    "billing",
    "authentication",
    "shipping",
    "refund",
    "technical_bug",
    "account_management",
    "complaint",
    "general_inquiry",
])


@pytest_asyncio.fixture
async def ft_context(
    sqlite_url: str,
    sessionmaker: async_sessionmaker[AsyncSession],
    deduper: InMemoryDeduper,
    blob_store: LocalBlobStore,
    session_store: InMemorySessionStore,
) -> AsyncIterator[FtContext]:
    settings = FtSettings.from_env(overrides={
        "environment": "test",
        "database": {"url": sqlite_url, "enable_rls": False},
    })
    ctx = FtContext(
        settings=settings,
        sessionmaker=sessionmaker,
        deduper=deduper,
        blob_store=blob_store,
        session_store=session_store,
        class_set=CLASS_SET,
    )
    set_context(ctx)
    try:
        yield ctx
    finally:
        clear_context()

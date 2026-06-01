"""Async SQLAlchemy engine + session factory + RLS helper.

* ``get_engine(url)`` — cached async engine singleton per URL.
* ``get_sessionmaker()`` — async sessionmaker bound to the engine.
* ``session_for_tenant(session, tenant_id)`` — context manager that
  sets ``app.tenant_id`` on the Postgres session (no-op on SQLite).

Tests use ``init_schema(engine)`` to create all tables on an
in-memory SQLite URL; production runs Alembic.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Any

from sqlalchemy import text
from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)

from efe_ft.db.models import Base

_ENGINE_CACHE: dict[str, AsyncEngine] = {}
_SESSIONMAKER_CACHE: dict[str, async_sessionmaker[AsyncSession]] = {}


def get_engine(url: str, **kwargs: Any) -> AsyncEngine:
    """Return (and cache) the async engine for ``url``."""
    if url in _ENGINE_CACHE:
        return _ENGINE_CACHE[url]
    # SQLite doesn't support real connection pooling; drop pool kwargs.
    if url.startswith("sqlite"):
        kwargs.pop("pool_size", None)
        kwargs.pop("max_overflow", None)
        kwargs.pop("pool_timeout", None)
    engine = create_async_engine(url, **kwargs)
    _ENGINE_CACHE[url] = engine
    return engine


def get_sessionmaker(url: str, **engine_kwargs: Any) -> async_sessionmaker[AsyncSession]:
    if url in _SESSIONMAKER_CACHE:
        return _SESSIONMAKER_CACHE[url]
    engine = get_engine(url, **engine_kwargs)
    sm = async_sessionmaker(engine, expire_on_commit=False)
    _SESSIONMAKER_CACHE[url] = sm
    return sm


async def init_schema(engine: AsyncEngine) -> None:
    """Create all tables — **tests only**. Prod uses Alembic."""
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)


async def drop_schema(engine: AsyncEngine) -> None:
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.drop_all)


@asynccontextmanager
async def session_for_tenant(
    sessionmaker: async_sessionmaker[AsyncSession],
    tenant_id: str,
    *,
    enable_rls: bool = True,
) -> AsyncIterator[AsyncSession]:
    """Yield a session with ``app.tenant_id`` set for RLS.

    On SQLite (tests) RLS is a no-op. On Postgres we set a GUC that
    RLS policies reference in ``USING (tenant_id = current_setting('app.tenant_id'))``.
    """
    async with sessionmaker() as session:
        if enable_rls:
            dialect = session.bind.dialect.name if session.bind else ""
            if dialect == "postgresql":
                # set_config returns value, use LOCAL so it's transaction-scoped
                await session.execute(
                    text("SELECT set_config('app.tenant_id', :t, true)"),
                    {"t": tenant_id},
                )
        yield session


async def dispose_all() -> None:
    """Disposes every cached engine — call on service shutdown."""
    for e in list(_ENGINE_CACHE.values()):
        await e.dispose()
    _ENGINE_CACHE.clear()
    _SESSIONMAKER_CACHE.clear()


__all__ = [
    "dispose_all",
    "drop_schema",
    "get_engine",
    "get_sessionmaker",
    "init_schema",
    "session_for_tenant",
]

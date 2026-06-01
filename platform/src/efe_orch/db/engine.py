"""Async SQLAlchemy engine + session factory for :mod:`efe_orch`.

* ``get_engine(url)`` — cached async engine singleton per URL.
* ``get_sessionmaker()`` — async sessionmaker bound to the engine.
* ``session_for_tenant(sm, tenant_id)`` — yields a session with
  ``app.tenant_id`` set for Postgres RLS (no-op on SQLite).

Tests use :func:`init_schema` to create all tables on a per-test
SQLite file; production runs Alembic.
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

from efe_orch.db.models import Base

_ENGINE_CACHE: dict[str, AsyncEngine] = {}
_SESSIONMAKER_CACHE: dict[str, async_sessionmaker[AsyncSession]] = {}


def get_engine(url: str, **kwargs: Any) -> AsyncEngine:
    """Return (and cache) the async engine for ``url``.

    SQLite ignores pool kwargs. For Postgres, any kwarg accepted by
    :func:`create_async_engine` flows through.
    """
    if url in _ENGINE_CACHE:
        return _ENGINE_CACHE[url]
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
    """Create all orchestrator tables — tests only. Prod uses Alembic."""
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
    """Yield an AsyncSession scoped to ``tenant_id``.

    On Postgres we ``SELECT set_config('app.tenant_id', …)`` so any
    RLS policy on the tables sees the current tenant. SQLite gets a
    no-op. The context manager always commits on success; callers can
    force rollback by raising.
    """
    async with sessionmaker() as session:
        if enable_rls:
            dialect = session.bind.dialect.name if session.bind else ""
            if dialect == "postgresql":
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

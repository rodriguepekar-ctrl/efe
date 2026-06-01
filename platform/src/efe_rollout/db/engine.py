"""Async SQLAlchemy engine + sessionmaker — mirrors :mod:`efe_registry.db.engine`.

SQLite receives a minimal pool (async drivers serialise anyway and
larger pools cause flaky tests). Postgres receives the tuned pool.
"""

from __future__ import annotations

from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)

from efe_rollout.config import DatabaseSettings
from efe_rollout.db.models import Base


def build_engine(settings: DatabaseSettings) -> AsyncEngine:
    url = settings.url
    if url.startswith("sqlite"):
        return create_async_engine(
            url, echo=settings.echo, future=True,
        )
    return create_async_engine(
        url,
        echo=settings.echo,
        future=True,
        pool_size=settings.pool_size,
        max_overflow=settings.pool_max_overflow,
        pool_timeout=settings.pool_timeout_s,
    )


def build_sessionmaker(
    engine: AsyncEngine,
) -> async_sessionmaker[AsyncSession]:
    return async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession,
    )


async def create_all(engine: AsyncEngine) -> None:
    """Create tables from :class:`Base.metadata`.

    Used by unit tests (SQLite) and by the ``init-db`` CLI subcommand.
    Production runs Alembic migrations.
    """
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)


__all__ = ["build_engine", "build_sessionmaker", "create_all"]

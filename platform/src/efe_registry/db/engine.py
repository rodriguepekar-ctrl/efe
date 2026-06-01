"""Async SQLAlchemy engine + sessionmaker — mirrors :mod:`efe_gate.db.engine`."""

from __future__ import annotations

from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)

from efe_registry.config import DatabaseSettings
from efe_registry.db.models import Base


def build_engine(settings: DatabaseSettings) -> AsyncEngine:
    """Build an AsyncEngine from DatabaseSettings.

    SQLite gets a minimal pool because async drivers serialise anyway
    and larger pools cause test noise. Postgres gets the configured
    pool.
    """
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

    Used by tests and CLI ``init-db``. In production migrations are
    handled by Alembic (see ``platform/alembic/`` when present).
    """
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)


__all__ = ["build_engine", "build_sessionmaker", "create_all"]

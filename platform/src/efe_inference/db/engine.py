"""Async SQLAlchemy engine + sessionmaker — mirrors sibling modules."""

from __future__ import annotations

from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)

from efe_inference.config import DatabaseSettings
from efe_inference.db.models import Base


def build_engine(settings: DatabaseSettings) -> AsyncEngine:
    if settings.url.startswith("sqlite"):
        return create_async_engine(
            settings.url, echo=settings.echo, future=True,
        )
    return create_async_engine(
        settings.url,
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
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)


__all__ = ["build_engine", "build_sessionmaker", "create_all"]

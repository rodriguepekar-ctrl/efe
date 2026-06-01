"""Shared dependency-injection context for the HTTP apps."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

if TYPE_CHECKING:
    from efe_ft.config import FtSettings
    from efe_ft.ingest.dedup import Deduper
    from efe_ft.labeling.session import SessionStore
    from efe_ft.storage import BlobStore


@dataclass
class FtContext:
    """Everything the HTTP handlers need — wired at app construction."""

    settings: FtSettings
    sessionmaker: async_sessionmaker[AsyncSession]
    deduper: Deduper
    blob_store: BlobStore
    session_store: SessionStore
    class_set: frozenset[str] = field(default_factory=frozenset)


_CTX: FtContext | None = None


def set_context(ctx: FtContext) -> None:
    global _CTX
    _CTX = ctx


def get_context() -> FtContext:
    if _CTX is None:
        raise RuntimeError(
            "FtContext not initialised — call set_context() at app startup",
        )
    return _CTX


def clear_context() -> None:
    global _CTX
    _CTX = None


__all__ = ["FtContext", "clear_context", "get_context", "set_context"]

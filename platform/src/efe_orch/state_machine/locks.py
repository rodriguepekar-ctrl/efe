"""Portable advisory-locking for the state machine.

The state machine needs *exclusive* access to one run's row while it
computes the next state and writes the transition row. In production
we use **Postgres advisory locks** (``pg_try_advisory_xact_lock``)
because they are session-local, automatically released at transaction
end, and keyed by a 64-bit int we derive from the run UUID.

Because we also want tests to run on SQLite (no advisory locks at
all), the locking is abstracted through :class:`LockBackend`:

* :class:`PostgresLockBackend` — ``pg_try_advisory_xact_lock``.
* :class:`SqliteLockBackend` — falls back to an in-process
  :class:`asyncio.Lock` keyed by run_id.
* :class:`InMemoryLockBackend` — identical to Sqlite backend but
  dialect-agnostic (useful when no session is available).

The public API is deliberately narrow: callers call
``backend.acquire(session, run_id)`` and get an :class:`AdvisoryLock`
context manager. The inner block sees either:

* a real Postgres xact-scoped lock (released on COMMIT/ROLLBACK), or
* a local mutex.

The 64-bit lock key is derived as the SHA-256 of the run_id, taking
the first 8 bytes interpreted as a signed big-endian integer. This
gives ~2^-32 collision probability between any two active runs —
safe in practice at our concurrency ceiling.
"""

from __future__ import annotations

import asyncio
import hashlib
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Protocol

from sqlalchemy import text
from sqlalchemy.ext.asyncio import AsyncSession

# ---------------------------------------------------------------------------
#  Key derivation — stable across processes
# ---------------------------------------------------------------------------


def lock_key_for(run_id: str) -> int:
    """Deterministic 64-bit signed int from a run_id string.

    Postgres ``pg_advisory_xact_lock(bigint)`` takes a signed 8-byte
    key. We want the same key across pods, so we compute it from the
    SHA-256 of the run_id. Top bit is forced off to keep the int
    positive (avoids ambiguity with the 2-arg form of the lock fn).
    """
    h = hashlib.sha256(run_id.encode("utf-8")).digest()
    # Take 8 bytes, big-endian, mask MSB so it fits in Postgres bigint.
    unsigned = int.from_bytes(h[:8], "big", signed=False) & 0x7FFF_FFFF_FFFF_FFFF
    return unsigned


# ---------------------------------------------------------------------------
#  Lock type + backend protocol
# ---------------------------------------------------------------------------


class AdvisoryLock:
    """Marker type returned by :meth:`LockBackend.acquire`.

    Its only job is to carry the ``(backend, key)`` pair so the
    context manager can release on exit. Callers don't instantiate
    this directly.
    """

    __slots__ = ("_released", "backend", "key", "run_id")

    def __init__(self, backend: LockBackend, key: int, run_id: str) -> None:
        self.backend = backend
        self.key = key
        self.run_id = run_id
        self._released = False

    @property
    def released(self) -> bool:
        return self._released

    async def release(self) -> None:
        if not self._released:
            await self.backend._release(self)
            self._released = True


class LockBackend(Protocol):
    """Backend used to acquire / release :class:`AdvisoryLock`."""

    def acquire(
        self, session: AsyncSession | None, run_id: str,
    ) -> _AcquireCM:  # pragma: no cover — protocol
        ...

    async def _release(self, lock: AdvisoryLock) -> None:  # pragma: no cover
        ...


class _AcquireCM:
    """Duck-typed async context manager returned by ``acquire()``."""

    def __init__(
        self, backend: LockBackend, session: AsyncSession | None, run_id: str,
    ) -> None:
        self._backend = backend
        self._session = session
        self._run_id = run_id
        self._lock: AdvisoryLock | None = None

    async def __aenter__(self) -> AdvisoryLock:
        # Concrete backends implement ``_do_acquire``.
        do_acquire = self._backend._do_acquire  # type: ignore[attr-defined]
        lock: AdvisoryLock = await do_acquire(self._session, self._run_id)
        self._lock = lock
        return lock

    async def __aexit__(self, *exc: object) -> None:
        if self._lock is not None:
            await self._lock.release()


# ---------------------------------------------------------------------------
#  Concrete backends
# ---------------------------------------------------------------------------


class InMemoryLockBackend:
    """Process-local mutex keyed by run_id.

    Safe for single-process tests. **Never** use in production with
    more than one orchestrator replica — there's no cross-process
    coordination.
    """

    def __init__(self) -> None:
        self._locks: dict[str, asyncio.Lock] = {}
        self._registry_lock = asyncio.Lock()
        self._held_by: dict[str, AdvisoryLock] = {}

    def _lock_for(self, run_id: str) -> asyncio.Lock:
        lk = self._locks.get(run_id)
        if lk is None:
            lk = asyncio.Lock()
            self._locks[run_id] = lk
        return lk

    def acquire(
        self, session: AsyncSession | None, run_id: str,
    ) -> _AcquireCM:
        return _AcquireCM(self, session, run_id)

    async def _do_acquire(
        self, session: AsyncSession | None, run_id: str,
    ) -> AdvisoryLock:
        # Register under a short metaregistry lock, then await the
        # per-run mutex without holding the registry lock so we don't
        # serialise the whole orchestrator.
        async with self._registry_lock:
            lk = self._lock_for(run_id)
        await lk.acquire()
        adv = AdvisoryLock(self, lock_key_for(run_id), run_id)
        self._held_by[run_id] = adv
        return adv

    async def _release(self, lock: AdvisoryLock) -> None:
        lk = self._locks.get(lock.run_id)
        if lk is not None and lk.locked():
            lk.release()
        self._held_by.pop(lock.run_id, None)


class SqliteLockBackend(InMemoryLockBackend):
    """Alias for :class:`InMemoryLockBackend` — SQLite has no advisory locks."""


class PostgresLockBackend:
    """Real Postgres xact-scoped advisory lock.

    The caller must supply a session with an *open* transaction. The
    lock is acquired via ``pg_advisory_xact_lock`` (blocks until
    available — we rely on fast TX + short hold times) and is
    automatically released by the DB when the transaction commits or
    rolls back. We therefore intentionally do nothing in
    :meth:`_release` — the DB will do it.
    """

    def acquire(
        self, session: AsyncSession | None, run_id: str,
    ) -> _AcquireCM:
        if session is None:
            raise RuntimeError(
                "PostgresLockBackend.acquire() requires an AsyncSession",
            )
        return _AcquireCM(self, session, run_id)

    async def _do_acquire(
        self, session: AsyncSession | None, run_id: str,
    ) -> AdvisoryLock:
        if session is None:
            raise RuntimeError(
                "PostgresLockBackend.acquire() requires an AsyncSession",
            )
        key = lock_key_for(run_id)
        await session.execute(
            text("SELECT pg_advisory_xact_lock(:k)"),
            {"k": key},
        )
        return AdvisoryLock(self, key, run_id)

    async def _release(self, lock: AdvisoryLock) -> None:
        # xact-scoped lock — DB releases on commit/rollback.
        return None


# ---------------------------------------------------------------------------
#  Dialect-sniffing factory
# ---------------------------------------------------------------------------


@asynccontextmanager
async def lock_for_dialect(
    session: AsyncSession, run_id: str,
    *, fallback: LockBackend | None = None,
) -> AsyncIterator[AdvisoryLock]:
    """Choose a backend based on the session's dialect.

    Convenience wrapper — most callers prefer to stash a backend at
    service construction time and reuse it.
    """
    dialect = session.bind.dialect.name if session.bind else ""
    backend: LockBackend = (
        PostgresLockBackend()
        if dialect == "postgresql"
        else (fallback or SqliteLockBackend())
    )
    async with backend.acquire(session, run_id) as lock:
        yield lock


__all__ = [
    "AdvisoryLock",
    "InMemoryLockBackend",
    "LockBackend",
    "PostgresLockBackend",
    "SqliteLockBackend",
    "lock_for_dialect",
    "lock_key_for",
]

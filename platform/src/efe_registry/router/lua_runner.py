"""LuaRunner — abstracts Redis EVAL calls.

The :class:`InMemoryLuaRunner` mirrors the semantics of
``ATOMIC_SWAP_LUA`` and ``QUOTA_BUCKET_LUA`` faithfully so unit tests
can run without a Redis process. Production uses
:class:`RedisLuaRunner` (aioredis-based) — defined here but only
imported lazily to keep tests lightweight.
"""

from __future__ import annotations

import asyncio
from collections import defaultdict
from dataclasses import dataclass
from typing import Any, Protocol, cast

from efe_registry.errors import (
    RoutingInvariantBroken,
    RoutingVersionConflict,
)


@dataclass(frozen=True, slots=True)
class LuaSwapResult:
    ok: bool
    new_version: int
    error: str | None = None


class LuaRunner(Protocol):
    async def atomic_swap(
        self,
        *,
        version_key: str,
        routing_key: str,
        pubsub_channel: str,
        expected_version: int,
        new_version: int,
        entries: tuple[tuple[str, int, str], ...],
    ) -> LuaSwapResult:  # pragma: no cover
        ...

    async def quota_check(
        self,
        *,
        quota_key: str,
        now_ms: int,
        capacity: float,
        refill_per_s: float,
        cost: float,
    ) -> bool:  # pragma: no cover
        ...

    async def get_version(
        self, version_key: str,
    ) -> int:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# In-memory runner (tests)
# ---------------------------------------------------------------------------


@dataclass
class _QuotaState:
    tokens: float
    last_ts_ms: int


class InMemoryLuaRunner:
    """Faithful implementation of atomic_swap + quota_bucket semantics."""

    def __init__(self) -> None:
        self._versions: dict[str, int] = defaultdict(int)
        self._routing: dict[str, dict[str, str]] = defaultdict(dict)
        self._quotas: dict[str, _QuotaState] = {}
        self.published: list[tuple[str, str]] = []
        self._lock = asyncio.Lock()

    async def atomic_swap(
        self,
        *,
        version_key: str,
        routing_key: str,
        pubsub_channel: str,
        expected_version: int,
        new_version: int,
        entries: tuple[tuple[str, int, str], ...],
    ) -> LuaSwapResult:
        async with self._lock:
            cur = self._versions[version_key]
            if cur != expected_version:
                return LuaSwapResult(
                    ok=False, new_version=cur,
                    error=f"version_mismatch (current={cur})",
                )
            if new_version != cur + 1:
                return LuaSwapResult(
                    ok=False, new_version=cur,
                    error="version_not_sequential",
                )
            # Validate weight sums.
            weight_sum = 0
            for _endpoint_id, weight, role in entries:
                if role in ("champion", "candidate"):
                    weight_sum += int(weight)
            if weight_sum != 100:
                return LuaSwapResult(
                    ok=False, new_version=cur,
                    error=f"weights_not_100 (sum={weight_sum})",
                )
            # Commit.
            self._routing[routing_key] = {
                endpoint_id: f"{weight}:{role}"
                for endpoint_id, weight, role in entries
            }
            self._versions[version_key] = new_version
            self.published.append((
                pubsub_channel,
                f'{{"tenant_key":"{routing_key}","version":{new_version}}}',
            ))
            return LuaSwapResult(ok=True, new_version=new_version)

    async def quota_check(
        self,
        *,
        quota_key: str,
        now_ms: int,
        capacity: float,
        refill_per_s: float,
        cost: float,
    ) -> bool:
        async with self._lock:
            st = self._quotas.get(quota_key)
            if st is None:
                st = _QuotaState(tokens=capacity, last_ts_ms=now_ms)
            elapsed = max(0.0, (now_ms - st.last_ts_ms) / 1000.0)
            new_tokens = min(capacity, st.tokens + elapsed * refill_per_s)
            if new_tokens < cost:
                self._quotas[quota_key] = _QuotaState(
                    tokens=new_tokens, last_ts_ms=now_ms,
                )
                return False
            self._quotas[quota_key] = _QuotaState(
                tokens=new_tokens - cost, last_ts_ms=now_ms,
            )
            return True

    async def get_version(self, version_key: str) -> int:
        return self._versions[version_key]

    # Test-only helpers
    def snapshot(self, routing_key: str) -> dict[str, str]:
        return dict(self._routing.get(routing_key, {}))


# ---------------------------------------------------------------------------
# Real Redis runner (lazy import)
# ---------------------------------------------------------------------------


class RedisLuaRunner:
    """Production runner — issues EVAL / EVALSHA via aioredis.

    Imported lazily so tests without aioredis still pass.

    NOSCRIPT recovery: Redis flushes its script cache on restart; an
    ``EVALSHA`` call after that will raise ``NOSCRIPT``. We handle this
    by reloading both scripts and retrying once.
    """

    def __init__(self, *, redis_client: Any, pubsub_channel: str) -> None:
        self._r = redis_client
        self._ch = pubsub_channel
        self._swap_sha: str | None = None
        self._quota_sha: str | None = None

    async def _ensure_scripts(self, *, force_reload: bool = False) -> None:
        if force_reload or self._swap_sha is None or self._quota_sha is None:
            from efe_registry.router.scripts import (
                ATOMIC_SWAP_LUA,
                QUOTA_BUCKET_LUA,
            )
            self._swap_sha = await self._r.script_load(ATOMIC_SWAP_LUA)
            self._quota_sha = await self._r.script_load(QUOTA_BUCKET_LUA)

    @staticmethod
    def _is_noscript(exc: BaseException) -> bool:
        """Return True if ``exc`` indicates the Lua script is missing.

        redis-py raises ``NoScriptError`` / ``ResponseError`` with
        ``NOSCRIPT`` in the message. We match by substring so we don't
        need a hard dependency on the redis exception types.
        """
        msg = str(exc)
        return "NOSCRIPT" in msg or "No matching script" in msg

    async def atomic_swap(
        self,
        *,
        version_key: str,
        routing_key: str,
        pubsub_channel: str,
        expected_version: int,
        new_version: int,
        entries: tuple[tuple[str, int, str], ...],
    ) -> LuaSwapResult:
        await self._ensure_scripts()
        argv: list[str] = [
            str(expected_version), str(new_version),
        ]
        for endpoint_id, weight, role in entries:
            argv.append(endpoint_id)
            argv.append(f"{weight}:{role}")
        try:
            result = await self._r.evalsha(
                cast(str, self._swap_sha), 3,
                version_key, routing_key, pubsub_channel,
                *argv,
            )
        except Exception as exc:  # pragma: no cover
            if self._is_noscript(exc):
                # Redis flushed its script cache; reload once and retry.
                await self._ensure_scripts(force_reload=True)
                try:
                    result = await self._r.evalsha(
                        cast(str, self._swap_sha), 3,
                        version_key, routing_key, pubsub_channel,
                        *argv,
                    )
                except Exception as exc2:
                    raise RoutingInvariantBroken(
                        f"Lua swap failed after NOSCRIPT reload: {exc2}",
                    ) from exc2
            else:
                raise RoutingInvariantBroken(f"Lua swap failed: {exc}") from exc
        if isinstance(result, (bytes, str)):
            text = (
                result.decode() if isinstance(result, bytes) else result
            )
            if text == "ok":
                return LuaSwapResult(ok=True, new_version=new_version)
        # Lua returned {err=...}
        if isinstance(result, dict) and "err" in result:
            err = str(result["err"])
            if err.startswith("version_mismatch"):
                raise RoutingVersionConflict(err)
            raise RoutingInvariantBroken(err)
        raise RoutingInvariantBroken(f"unexpected Lua result: {result!r}")

    async def quota_check(
        self,
        *,
        quota_key: str,
        now_ms: int,
        capacity: float,
        refill_per_s: float,
        cost: float,
    ) -> bool:
        await self._ensure_scripts()
        try:
            res = await self._r.evalsha(
                cast(str, self._quota_sha), 1, quota_key,
                str(now_ms), str(capacity), str(refill_per_s), str(cost),
            )
        except Exception as exc:  # pragma: no cover
            if self._is_noscript(exc):
                await self._ensure_scripts(force_reload=True)
                try:
                    res = await self._r.evalsha(
                        cast(str, self._quota_sha), 1, quota_key,
                        str(now_ms), str(capacity),
                        str(refill_per_s), str(cost),
                    )
                except Exception as exc2:
                    raise RoutingInvariantBroken(
                        f"quota Lua failed after NOSCRIPT reload: {exc2}",
                    ) from exc2
            else:
                raise RoutingInvariantBroken(f"quota Lua failed: {exc}") from exc
        return bool(int(res))

    async def get_version(self, version_key: str) -> int:
        raw = await self._r.get(version_key)
        if raw is None:
            return 0
        if isinstance(raw, bytes):
            raw = raw.decode()
        return int(raw)


__all__ = [
    "InMemoryLuaRunner",
    "LuaRunner",
    "LuaSwapResult",
    "RedisLuaRunner",
]

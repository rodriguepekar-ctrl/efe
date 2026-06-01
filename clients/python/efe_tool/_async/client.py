"""Async variant of :class:`efe_tool.EfeClient` — Sprint D10 skeleton.

The full async implementation is scheduled for v0.2.0 (post-D11).
For v0.1.10, this skeleton :

  * Exposes the same public surface as ``EfeClient`` (constructor
    args, methods) so partner code can be authored against the
    async API today and "just work" when the real impl lands.
  * Falls back internally to a thread-pool wrapping the sync
    :class:`EfeClient` — slower than native asyncio under
    concurrent load (the sync calls block grpcio threads), but
    correct and unblocks early adopters.
  * Emits a ``DeprecationWarning`` once-per-process when
    instantiated, alerting callers that a non-trivial perf change
    is coming in v0.2.0 (replace thread-pool with grpc.aio
    native).

Architecture note
─────────────────

This staged delivery (skeleton in 0.1.x, native in 0.2.x) is the
"facade-then-real" pattern used by httpx (sync first → AsyncClient
added in 0.7.0 with thread-pool, native asyncio in 0.13.0). It
lets us ship the SDK contract early without committing to a
specific concurrency model before partner usage data is in.
"""

from __future__ import annotations

import asyncio
import warnings
from concurrent.futures import ThreadPoolExecutor
from contextlib import AsyncExitStack
from typing import Any

from efe_tool.catalog import AnchorCatalog
from efe_tool.client import EfeClient
from efe_tool.embedder import Embedder, LocalSTEmbedder
from efe_tool.types import ClassifyResult


_SKELETON_WARNING_EMITTED = False


def _warn_skeleton_once() -> None:
    """Emit a once-per-process warning that AsyncEfeClient is a
    thread-pool skeleton in v0.1.x, native in v0.2.x.

    Use module-level state (vs ``warnings.simplefilter('default')``)
    so callers can suppress with ``-W ignore::DeprecationWarning``
    or via standard logging filters.
    """
    global _SKELETON_WARNING_EMITTED
    if _SKELETON_WARNING_EMITTED:
        return
    _SKELETON_WARNING_EMITTED = True
    warnings.warn(
        "AsyncEfeClient is a thread-pool skeleton in efe-tool v0.1.x. "
        "Native grpc.aio backend lands in v0.2.0. The public surface "
        "is stable; only the internal implementation will change.",
        DeprecationWarning,
        stacklevel=3,
    )


class AsyncEfeClient:
    """Asyncio-aware EFE client — same public surface as
    :class:`EfeClient`, but every method is `async`.

    Usage::

        async with AsyncEfeClient(host="127.0.0.1:50051", insecure=True) as client:
            result = await client.classify(text, catalog)

    See :class:`efe_tool.EfeClient` for the full method documentation —
    this class delegates to a sync client running in a dedicated
    thread-pool. All thread-pool overhead is amortised across
    many concurrent calls; a single awaited call has comparable
    latency to a sync ``EfeClient.classify`` call (one extra GIL
    hop).

    Constructor argument naming
    ───────────────────────────
    ``host`` is accepted as the gRPC-canonical name (matches ghz,
    grpcurl, gRPC docs). Internally it maps to the sync
    :class:`EfeClient`'s ``endpoint`` parameter. Same for
    ``token`` (mapped to ``jwt`` on the sync client). This keeps
    the async surface idiomatic to gRPC users without forcing them
    to remember which arg is named what (audit fix A4).
    """

    def __init__(
        self,
        *,
        host: str,
        embedder: Embedder | None = None,
        insecure: bool = False,
        token: str | None = None,
        tenant: str | None = None,
        timeout_s: float = 30.0,
        max_workers: int = 8,
    ) -> None:
        _warn_skeleton_once()
        self._host = host
        # If no embedder is supplied, fall back to the env-driven
        # local sentence-transformers embedder. This matches the
        # behaviour of `EfeClient.from_env()` and lets quickstarts
        # work with `AsyncEfeClient(host=...)` only.
        self._embedder: Embedder = embedder if embedder is not None \
            else LocalSTEmbedder.from_env()
        self._insecure = insecure
        self._token = token
        self._tenant = tenant
        self._timeout_s = timeout_s
        self._max_workers = max_workers
        self._executor: ThreadPoolExecutor | None = None
        self._sync_client: EfeClient | None = None
        self._aexit_stack: AsyncExitStack | None = None

    # ─── Lifecycle ───────────────────────────────────────────────

    async def __aenter__(self) -> "AsyncEfeClient":
        self._executor = ThreadPoolExecutor(
            max_workers=self._max_workers,
            thread_name_prefix="efe-async",
        )
        loop = asyncio.get_running_loop()
        # Build the sync client off the event loop — its connect path
        # is blocking (TCP handshake, possibly TLS).
        self._sync_client = await loop.run_in_executor(
            self._executor,
            self._build_sync_client,
        )
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.close()

    def _build_sync_client(self) -> EfeClient:
        """Translate the async constructor's kwargs to the sync
        :class:`EfeClient` constructor's signature.

        Audit fix A4: the previous version passed ``host=`` and
        ``token=`` to ``EfeClient(...)``, but the sync constructor
        expects ``endpoint=`` and ``jwt=``. It also missed the
        REQUIRED ``embedder=`` argument. This implementation
        translates names + always provides an embedder.
        """
        kwargs: dict[str, Any] = {
            "endpoint": self._host,           # gRPC-style host:port
            "embedder": self._embedder,
            "insecure": self._insecure,
        }
        if self._token is not None:
            kwargs["jwt"] = self._token       # async `token` → sync `jwt`
        if self._tenant is not None:
            kwargs["tenant"] = self._tenant
        if self._timeout_s != 30.0:
            kwargs["timeout_s"] = self._timeout_s
        return EfeClient(**kwargs)

    async def close(self) -> None:
        loop = asyncio.get_running_loop()
        if self._sync_client is not None:
            await loop.run_in_executor(
                self._executor, self._sync_client.close
            )
            self._sync_client = None
        if self._executor is not None:
            self._executor.shutdown(wait=False)
            self._executor = None

    # ─── API ─────────────────────────────────────────────────────

    async def classify(
        self,
        text: str,
        catalog: AnchorCatalog,
        **kwargs: Any,
    ) -> ClassifyResult:
        """Async wrapper for :meth:`EfeClient.classify`.

        Same signature — see the sync client for details.
        """
        if self._sync_client is None:
            raise RuntimeError(
                "AsyncEfeClient is not entered — use `async with` or "
                "call `await client.__aenter__()` explicitly."
            )
        loop = asyncio.get_running_loop()
        return await loop.run_in_executor(
            self._executor,
            lambda: self._sync_client.classify(text, catalog, **kwargs),
        )

    async def healthcheck(self) -> bool:
        """Async wrapper for :meth:`EfeClient.healthcheck`."""
        if self._sync_client is None:
            return False
        loop = asyncio.get_running_loop()
        return await loop.run_in_executor(
            self._executor, self._sync_client.healthcheck,
        )

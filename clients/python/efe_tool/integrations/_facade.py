"""The Letter-D façade: ``EfeFacade``.

Heart of the integration layer. Wraps :class:`efe_tool.client.EfeClient`,
:class:`efe_tool.catalog.AnchorCatalog` and
:class:`efe_tool.embedder.LocalSTEmbedder` behind a stable, typed,
thread-safe API.

See LETTER_D_ARCHITECTURE.md §5 and §6 for the specification.

Invariants
----------

* One ``EfeFacade`` per process (enforced by the framework adapters,
  not by the façade itself — nothing prevents tests from constructing
  several).
* Thread-safe hot path: ``classify()`` and ``classify_async()`` take
  NO lock. Internal mutation (``close``, ``reload_catalog``) uses a
  :class:`threading.Lock` on a short critical section only.
* Frozen configuration: :attr:`EfeFacade.config` is immutable.
* Lifecycle-typed errors: every exception the host receives is a
  subclass of :class:`EfeFacadeError`.
"""

from __future__ import annotations

import asyncio
import contextvars
import functools
import hashlib
import logging
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import replace
from pathlib import Path
from typing import Any, Mapping, Optional

from ._config import EfeConfig
from ._errors import (
    EfeFacadeCatalogError,
    EfeFacadeClosedError,
    EfeFacadeConfigError,
    EfeFacadeError,
    EfeFacadeInitError,
    EfeFacadeRejectedError,
    EfeFacadeTransientError,
    EfeFacadeUsageError,
    from_bridge_error,
)
from ._types import FacadeStats, HealthStatus, Verdict


_log = logging.getLogger("efe_tool.integrations")


#: Context variable that hosts can set in middleware to propagate a
#: request id into every classify call without passing it explicitly.
request_id_var: contextvars.ContextVar[Optional[str]] = contextvars.ContextVar(
    "efe_request_id", default=None,
)


_INSTANCE_COUNT: int = 0         # best-effort process-wide guard
_INSTANCE_LOCK = threading.Lock()


# ---------------------------------------------------------------------------
#  Façade
# ---------------------------------------------------------------------------


class EfeFacade:
    """Host-facing façade over the EFE gRPC stack.

    Parameters
    ----------
    config:
        A frozen :class:`EfeConfig`. Override via the
        ``EfeConfig.with_override(...)`` helper in tests.
    """

    # ------------------------------------------------------------------
    #  Construction
    # ------------------------------------------------------------------

    def __init__(self, config: EfeConfig) -> None:
        global _INSTANCE_COUNT
        if not isinstance(config, EfeConfig):
            raise EfeFacadeConfigError(
                f"EfeFacade expects an EfeConfig, got {type(config).__name__}",
            )
        self._config: EfeConfig = config
        self._embedder: Any = None
        self._client: Any = None
        self._catalogs: dict[str, Any] = {}
        self._is_ready: bool = False
        self._is_closed: bool = False
        self._in_flight: int = 0
        self._stats_lock = threading.Lock()
        self._mutate_lock = threading.Lock()    # close / reload
        self._stats: FacadeStats = FacadeStats(
            model=config.embedder_model,
            endpoint=config.endpoint,
        )
        self._latency_window: list[float] = []
        self._latency_window_cap: int = 512
        self._executor: Optional[ThreadPoolExecutor] = None
        with _INSTANCE_LOCK:
            _INSTANCE_COUNT += 1
            self._instance_index = _INSTANCE_COUNT
        _log.info("efe_facade_init", extra={
            "endpoint": config.endpoint,
            "model": config.embedder_model,
            "catalogs": list(config.catalogs),
            "instance_index": self._instance_index,
        })

    @classmethod
    def from_env(
        cls, env: Optional[Mapping[str, str]] = None,
    ) -> "EfeFacade":
        """Convenience factory: build an :class:`EfeConfig` from env
        and construct the façade."""
        return cls(EfeConfig.from_env(env))

    # ------------------------------------------------------------------
    #  Public properties
    # ------------------------------------------------------------------

    @property
    def config(self) -> EfeConfig:
        return self._config

    @property
    def is_ready(self) -> bool:
        return self._is_ready and not self._is_closed

    @property
    def is_closed(self) -> bool:
        return self._is_closed

    @property
    def catalogs(self) -> list[str]:
        return list(self._catalogs.keys())

    # ------------------------------------------------------------------
    #  Lifecycle
    # ------------------------------------------------------------------

    def preload(self) -> None:
        """Load the embedder, every declared catalog, and (optionally)
        send a classify probe.

        Blocks for the entire duration. Call exactly once, from the
        framework's startup hook.

        Raises
        ------
        EfeFacadeInitError
            Embedder/client/catalog failed to materialise.
        EfeFacadeCatalogError
            A catalog YAML is missing or malformed.
        EfeFacadeConfigError
            The config is self-inconsistent.
        """
        self._ensure_open()
        t0 = time.perf_counter()
        with self._mutate_lock:
            if self._is_ready:
                return
            self._init_embedder()
            self._init_client()
            self._init_catalogs()
            if self._config.preload_classify_probe and self._catalogs:
                self._do_probe()
            self._is_ready = True
            self._stats.preload_ms = int((time.perf_counter() - t0) * 1000)
            self._stats.is_ready = True
            self._stats.catalogs_loaded = tuple(self._catalogs.keys())
        _log.info("efe_facade_preload_done", extra={
            "preload_ms": self._stats.preload_ms,
            "catalogs": list(self._catalogs.keys()),
            "probe": self._config.preload_classify_probe,
        })

    # ------------------------------------------------------------------
    #  Context-manager protocol (DT-D-2)
    #
    #  `with EfeFacade(...) as efe:` is the canonical app-tier idiom
    #  documented in `LETTER_D_ARCHITECTURE.md` §5. Letting the
    #  `with` block exit auto-calls `close()` on success OR exception,
    #  which gives apps a leak-proof shutdown path even when their
    #  request handler raises.
    #
    #  Pin tested by `test_d20_dt_d2_efefacade_api_methods_stable`.
    # ------------------------------------------------------------------

    def __enter__(self) -> "EfeFacade":
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        # Default close() timeout is sufficient for any well-behaved
        # client ; apps that need a tighter shutdown can call
        # `efe.close(timeout_s=...)` explicitly before exiting `with`.
        self.close()

    async def __aenter__(self) -> "EfeFacade":
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> None:
        # Async path mirrors sync — close() drains in-flight calls
        # using a small sleep loop ; safe to call from an async
        # context (we don't hold the event loop).
        self.close()

    def close(self, *, timeout_s: float = 30.0) -> None:
        """Drain in-flight classify calls, close the gRPC channel, and
        release the embedder reference. Idempotent."""
        with self._mutate_lock:
            if self._is_closed:
                return
            self._is_closed = True
            self._stats.is_closed = True
        t0 = time.perf_counter()
        deadline = t0 + max(0.0, float(timeout_s))
        # Wait up to timeout for in-flight calls.
        while True:
            with self._stats_lock:
                inflight = self._in_flight
            if inflight == 0 or time.perf_counter() >= deadline:
                break
            time.sleep(0.05)
        with self._mutate_lock:
            if self._client is not None:
                try:
                    self._client.close()
                except Exception as e:  # pragma: no cover
                    _log.warning("efe_facade_client_close_error",
                                 extra={"error": str(e)})
                self._client = None
            if self._executor is not None:
                try:
                    self._executor.shutdown(wait=False, cancel_futures=True)
                except TypeError:  # pragma: no cover — Python 3.8
                    self._executor.shutdown(wait=False)
                self._executor = None
            # Keep catalogs + embedder for potential re-use — Python GC
            # takes care of actual release once all refs drop.
            self._is_ready = False
            self._stats.is_ready = False
        _log.info("efe_facade_close", extra={
            "in_flight_at_start": inflight,
            "wait_ms": int((time.perf_counter() - t0) * 1000),
        })

    def healthcheck(self) -> HealthStatus:
        """Check embedder + gRPC server. Returns a structured result."""
        t0 = time.perf_counter()
        checks: dict[str, bool] = {
            "embedder_loaded": self._embedder is not None,
            "client_initialised": self._client is not None,
            "catalogs_loaded": bool(self._catalogs),
            "is_ready": self.is_ready,
        }
        reason: Optional[str] = None
        grpc_ok = False
        if self._client is not None:
            try:
                grpc_ok = bool(self._client.healthcheck())
            except Exception as e:
                reason = f"healthcheck call failed: {e}"
        checks["grpc_reachable"] = grpc_ok
        latency = int((time.perf_counter() - t0) * 1000)
        if self._is_closed:
            status = "NOT_SERVING"
            reason = reason or "facade closed"
        elif not self._is_ready:
            status = "NOT_READY"
            reason = reason or "preload() not called or failed"
        elif all(checks.values()):
            status = "SERVING"
        else:
            status = "NOT_SERVING"
            reason = reason or f"checks failed: {[k for k, v in checks.items() if not v]}"
        return HealthStatus(
            status=status, latency_ms=latency, checks=checks, reason=reason,
        )

    def reload_catalog(self, name: str) -> None:
        """Reload one catalog from disk (copy-on-write).

        Concurrent :meth:`classify` calls against a different catalog
        continue unaffected; calls against ``name`` see the old version
        until the swap completes atomically.
        """
        self._ensure_open()
        if name not in self._catalogs:
            raise EfeFacadeUsageError(
                f"catalog {name!r} is not loaded",
                hint=f"known catalogs: {list(self._catalogs.keys())}",
            )
        from ..catalog import AnchorCatalog
        from ..errors import CatalogError
        current = self._catalogs[name]
        src = getattr(current, "source_path", None)
        if not src:
            raise EfeFacadeCatalogError(
                f"catalog {name!r} has no source_path, cannot reload",
            )
        try:
            new_catalog = AnchorCatalog.load(
                Path(src), embedder=self._embedder,
            )
        except CatalogError as e:
            raise from_bridge_error(e) from e
        except FileNotFoundError as e:
            raise EfeFacadeCatalogError(str(e), cause=e) from e
        old_fp = getattr(current, "embedder_fingerprint", "?")
        new_fp = getattr(new_catalog, "embedder_fingerprint", "?")
        # Copy-on-write: replace entry atomically.
        new_map = dict(self._catalogs)
        new_map[name] = new_catalog
        self._catalogs = new_map
        _log.info("efe_facade_reload_catalog", extra={
            "name": name, "old_fingerprint": old_fp, "new_fingerprint": new_fp,
        })

    # ------------------------------------------------------------------
    #  Hot path — classify
    # ------------------------------------------------------------------

    def classify(
        self,
        text: str,
        *,
        catalog: str,
        timeout_s: Optional[float] = None,
        request_id: Optional[str] = None,
        strict: bool = False,
    ) -> Verdict:
        """Synchronously classify ``text`` under ``catalog``.

        Thread-safe. Raises :class:`EfeFacadeError` on failure.

        Parameters
        ----------
        text:
            UTF-8 string, 1..``config.max_text_length`` chars.
        catalog:
            Name of a catalog declared in :attr:`EfeConfig.catalogs`.
        timeout_s:
            Reserved for per-call deadline override. **Currently not
            honoured** by the underlying gRPC stub; the global
            :attr:`EfeConfig.request_timeout_s` governs. A future
            release will route this through. Passing a value here
            emits a ``debug`` log event and is otherwise ignored.
        request_id:
            Optional client-assigned id, propagated to the server via
            the request payload and to every log line on this path.
            When ``None``, we fall back to :data:`request_id_var`.
        strict:
            If True, raise :class:`EfeFacadeRejectedError` when the
            verdict has ``reject == True`` or the confidence falls
            below the catalog's ``confidence_high`` threshold.
        """
        self._ensure_serving()
        self._validate_input(text, catalog)
        if timeout_s is not None:
            # The current EfeClient sync stub binds its gRPC deadline at
            # construction time. We cannot override per-call without a
            # lock-around-mutation dance that introduces cross-thread
            # interference. Silently acknowledge via a debug log so
            # forward-compatible code can already pass the arg.
            _log.debug(
                "efe_facade_timeout_s_ignored",
                extra={
                    "requested": float(timeout_s),
                    "actual_s": self._config.effective_request_timeout_s,
                },
            )
            if float(timeout_s) <= 0:
                raise EfeFacadeUsageError(
                    "timeout_s must be positive when provided",
                )
        rid = request_id or request_id_var.get()
        catalog_obj = self._catalogs[catalog]

        t_start = time.perf_counter()
        with self._stats_lock:
            self._in_flight += 1
            self._stats.in_flight = self._in_flight
        try:
            try:
                from ..errors import BridgeError
                result = self._client.classify(
                    text, catalog_obj,
                    request_id=rid,
                    budget_wall_clock_ms=self._config.classify_budget_ms,
                    max_rounds=self._config.max_rounds,
                )
            except Exception as e:
                self._record_error()
                from ..errors import BridgeError as _BE
                if isinstance(e, _BE):
                    raise from_bridge_error(e) from e
                raise EfeFacadeError(
                    f"unexpected error from client.classify: {type(e).__name__}: {e}",
                    cause=e,
                ) from e
            elapsed_ms = int((time.perf_counter() - t_start) * 1000)
            verdict = _result_to_verdict(
                result, catalog=catalog, elapsed_ms=elapsed_ms, request_id=rid,
            )
            self._record_success(verdict, elapsed_ms)
            if strict and verdict.is_reject:
                raise EfeFacadeRejectedError(
                    f"classification rejected under strict=True "
                    f"(confidence={verdict.confidence:.3f})",
                    hint="the swarm did not reach the confidence floor",
                    context=verdict.as_dict(),
                )
            _log.debug("efe_facade_classify_done", extra={
                "catalog": catalog,
                "label": verdict.label,
                "confidence": verdict.confidence,
                "rounds": verdict.rounds,
                "elapsed_ms": elapsed_ms,
                "request_id": rid,
                "text_length": len(text),
                "text_hash": _short_hash(text),
            })
            return verdict
        finally:
            with self._stats_lock:
                self._in_flight -= 1
                self._stats.in_flight = self._in_flight

    async def classify_async(
        self,
        text: str,
        *,
        catalog: str,
        timeout_s: Optional[float] = None,
        request_id: Optional[str] = None,
        strict: bool = False,
    ) -> Verdict:
        """Asynchronous variant — runs the sync classify in a thread
        pool so the event loop stays responsive."""
        self._ensure_serving()
        if self._executor is None:
            # Lazy construction so we don't waste a pool in sync-only hosts.
            with self._mutate_lock:
                if self._executor is None:
                    self._executor = ThreadPoolExecutor(
                        max_workers=self._config.async_executor_max_workers,
                        thread_name_prefix="efe-facade",
                    )
        loop = asyncio.get_running_loop()
        func = functools.partial(
            self.classify,
            text, catalog=catalog, timeout_s=timeout_s,
            request_id=request_id, strict=strict,
        )
        return await loop.run_in_executor(self._executor, func)

    # ------------------------------------------------------------------
    #  Introspection
    # ------------------------------------------------------------------

    def stats(self) -> FacadeStats:
        """Return a snapshot of the running stats."""
        with self._stats_lock:
            # Compute p50/p95 on the sliding window.
            xs = sorted(self._latency_window)
            if xs:
                p50 = xs[len(xs) // 2]
                p95 = xs[max(0, int(len(xs) * 0.95) - 1)]
            else:
                p50 = 0.0
                p95 = 0.0
            snap = replace(
                self._stats,
                latency_p50_ms_last_window=p50,
                latency_p95_ms_last_window=p95,
            )
        return snap

    # ------------------------------------------------------------------
    #  Internals
    # ------------------------------------------------------------------

    def _ensure_open(self) -> None:
        if self._is_closed:
            raise EfeFacadeClosedError(
                "EfeFacade.close() was called; construct a new facade to recover",
            )

    def _ensure_serving(self) -> None:
        self._ensure_open()
        if not self._is_ready:
            raise EfeFacadeInitError(
                "EfeFacade is not ready — call preload() in your startup hook",
                hint="most frameworks call preload() in their boot event; "
                     "see docs/integrations/<framework>.md",
            )

    def _validate_input(self, text: Any, catalog: Any) -> None:
        if not isinstance(text, str):
            raise EfeFacadeUsageError(
                f"text must be a string, got {type(text).__name__}",
            )
        if not text.strip():
            raise EfeFacadeUsageError("text is empty or whitespace only")
        if len(text) > self._config.max_text_length:
            raise EfeFacadeUsageError(
                f"text too long ({len(text)} > "
                f"{self._config.max_text_length} chars)",
                hint="trim the input or raise max_text_length",
            )
        if not isinstance(catalog, str) or not catalog:
            raise EfeFacadeUsageError("catalog must be a non-empty string")
        if catalog not in self._catalogs:
            raise EfeFacadeUsageError(
                f"unknown catalog {catalog!r}",
                hint=f"available catalogs: {list(self._catalogs.keys())}",
            )

    def _init_embedder(self) -> None:
        try:
            from ..embedder import LocalSTEmbedder
            self._embedder = LocalSTEmbedder(self._config.embedder_model)
        except Exception as e:
            raise EfeFacadeInitError(
                f"embedder init failed: {type(e).__name__}: {e}",
                hint="install sentence-transformers + check the model name",
                cause=e,
            ) from e

    def _init_client(self) -> None:
        try:
            from ..client import EfeClient
            self._client = EfeClient(
                endpoint=self._config.endpoint,
                embedder=self._embedder,
                jwt=self._config.jwt,
                tenant=self._config.tenant,
                timeout_s=self._config.effective_request_timeout_s,
                cache_size=self._config.cache_size,
                cache_ttl_s=self._config.cache_ttl_s,
                insecure=not self._config.tls,
            )
        except Exception as e:
            raise EfeFacadeInitError(
                f"client init failed: {type(e).__name__}: {e}",
                cause=e,
            ) from e

    def _init_catalogs(self) -> None:
        from ..catalog import AnchorCatalog
        from ..errors import CatalogError
        # Build into a local dict so a partial failure doesn't leave
        # self._catalogs in a half-loaded state — retries of preload()
        # restart from scratch.
        new_catalogs: dict[str, Any] = {}
        for name in self._config.catalogs:
            path = self._resolve_catalog_path(name)
            try:
                catalog = AnchorCatalog.load(path, embedder=self._embedder)
            except CatalogError as e:
                raise from_bridge_error(e) from e
            except FileNotFoundError as e:
                raise EfeFacadeCatalogError(
                    f"catalog file not found for {name!r}: {e}",
                    cause=e,
                ) from e
            except Exception as e:
                raise EfeFacadeCatalogError(
                    f"failed to load catalog {name!r}: {type(e).__name__}: {e}",
                    cause=e,
                ) from e
            new_catalogs[name] = catalog
        # Atomic swap — publish all-or-nothing.
        self._catalogs = new_catalogs

    def _resolve_catalog_path(self, name: str) -> Path:
        for p in self._config.catalog_search_paths:
            candidate = Path(p) / f"{name}.yaml"
            if candidate.is_file():
                return candidate
        cwd_candidate = Path("catalogs") / f"{name}.yaml"
        if cwd_candidate.is_file():
            return cwd_candidate
        raise EfeFacadeCatalogError(
            f"catalog {name!r} not found (search paths: "
            f"{list(self._config.catalog_search_paths) + ['./catalogs/']})",
            hint="add catalog_search_paths to EfeConfig",
        )

    def _do_probe(self) -> None:
        # Pick the first loaded catalog; send a tiny probe text.
        name = next(iter(self._catalogs.keys()))
        try:
            self._client.classify(
                "probe",
                self._catalogs[name],
                budget_wall_clock_ms=self._config.classify_budget_ms,
                max_rounds=self._config.max_rounds,
            )
        except Exception as e:  # probe failure is non-fatal
            _log.warning("efe_facade_preload_probe_failed", extra={
                "error": str(e), "catalog": name,
            })

    # ---- stats helpers ----

    def _record_success(self, verdict: Verdict, elapsed_ms: int) -> None:
        with self._stats_lock:
            self._stats.classify_total += 1
            if verdict.is_reject:
                self._stats.classify_rejected += 1
            self._stats.latency_sum_ms += elapsed_ms
            self._stats.latency_count += 1
            self._latency_window.append(float(elapsed_ms))
            if len(self._latency_window) > self._latency_window_cap:
                # Drop oldest half — cheap O(n) every 2*N calls.
                del self._latency_window[: self._latency_window_cap // 2]

    def _record_error(self) -> None:
        with self._stats_lock:
            self._stats.classify_failed += 1


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _result_to_verdict(
    result: Any, *, catalog: str, elapsed_ms: int,
    request_id: Optional[str],
) -> Verdict:
    """Convert an internal ``ClassifyResult`` into a public :class:`Verdict`."""
    label = str(getattr(result, "label", "?"))
    confidence = float(getattr(result, "confidence", 0.0) or 0.0)
    reject = bool(getattr(result, "reject", False))
    rounds = int(getattr(result, "rounds", 0) or 0)
    alignment_delta = float(getattr(result, "alignment_delta", 0.0) or 0.0)
    extra = getattr(result, "extra", None)
    zone: str
    if isinstance(extra, Mapping) and extra.get("zone") in {"trusted", "warn", "reject"}:
        zone = str(extra["zone"])
    else:
        zone = "reject" if reject else "trusted"
    return Verdict(
        label=label,
        confidence=confidence,
        reject=reject,
        rounds=rounds,
        alignment_delta=alignment_delta,
        catalog=catalog,
        zone=zone,
        elapsed_ms=elapsed_ms,
        request_id=request_id,
    )


def _short_hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]


__all__ = ["EfeFacade", "request_id_var"]

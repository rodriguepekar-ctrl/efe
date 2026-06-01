"""Async client for the ``SwarmService`` gRPC surface.

The client is a thin orchestration layer over a pluggable transport:

* **Default transport** — a real gRPC stub wrapping the
  ``efe.v1.SwarmService`` generated code (loaded lazily to keep the
  platform importable without grpcio installed).

* **In-memory transport** — a deterministic fake used by tests and by
  the HTTP API when running in ``standalone`` mode without a Rust
  backend.

The client exposes a single async method :meth:`execute` that returns a
:class:`SwarmOutcome` after running the full retry/deadline machinery.
"""

from __future__ import annotations

import asyncio
import contextlib
import logging
import random
import time
import uuid
from collections.abc import AsyncIterator, Awaitable, Callable
from dataclasses import dataclass
from typing import Any, Protocol, runtime_checkable

from efe_platform.errors import (
    EfePlatformError,
    SwarmDeadlineExceeded,
    SwarmDivergence,
    SwarmError,
    SwarmInvalidSpec,
    SwarmUnavailable,
)
from efe_platform.swarm.protocol import (
    DecodedPayload,
    DecodeMode,
    ExecuteRequest,
    ExecuteResponse,
    SwarmOutcome,
    SwarmSpec,
    Verdict,
)
from efe_platform.swarm.verdict import analyze_verdict

_LOG = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
#  Endpoint
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SwarmEndpoint:
    """Addressable identity of the server."""

    target: str
    tls: bool = False
    tls_ca_path: str | None = None

    def __post_init__(self) -> None:
        if not self.target:
            raise ValueError("SwarmEndpoint.target must be non-empty")


# ---------------------------------------------------------------------------
#  Transport protocol
# ---------------------------------------------------------------------------


@runtime_checkable
class SwarmTransport(Protocol):
    """Minimum surface the client needs from a transport.

    Implementations:

    * :class:`GrpcSwarmTransport` — the real wire transport wrapping
      the generated ``efe.v1`` stubs. Lazy-imported to keep ``grpcio``
      optional.
    * :class:`InMemorySwarmTransport` — test/local fake.

    .. note:: ``execute_streaming`` is declared as **non-async** here
        because implementations are async generators (``async def``
        with ``yield``); declaring it ``async def -> AsyncIterator``
        would require an extra ``await`` before the ``async for``.
    """

    async def execute(
        self,
        req: ExecuteRequest,
        *,
        metadata: tuple[tuple[str, str], ...],
        deadline_s: float,
    ) -> ExecuteResponse: ...

    def execute_streaming(
        self,
        req: ExecuteRequest,
        *,
        metadata: tuple[tuple[str, str], ...],
        deadline_s: float,
    ) -> AsyncIterator[dict[str, Any]]: ...

    async def describe_spec(
        self,
        spec: SwarmSpec,
        *,
        deadline_s: float,
    ) -> dict[str, Any]: ...

    async def close(self) -> None: ...


# ---------------------------------------------------------------------------
#  SwarmClient — main surface
# ---------------------------------------------------------------------------


class SwarmClient:
    """Orchestrating async client — retries, deadlines, metadata.

    Example::

        async with SwarmClient(SwarmEndpoint("localhost:50051")) as client:
            outcome = await client.execute(spec, tenant="acme")

    The context manager form guarantees graceful shutdown of the
    underlying transport.
    """

    def __init__(
        self,
        endpoint: SwarmEndpoint,
        *,
        transport: SwarmTransport | None = None,
        max_retries: int = 3,
        default_deadline_ms: int = 5_000,
        jwt: str | None = None,
        retry_jitter_ms: int = 50,
    ) -> None:
        if max_retries < 0:
            raise ValueError("max_retries must be >= 0")
        if default_deadline_ms <= 0:
            raise ValueError("default_deadline_ms must be > 0")
        self._endpoint = endpoint
        self._transport: SwarmTransport = transport or _lazy_grpc_transport(endpoint)
        self._max_retries = max_retries
        self._default_deadline_ms = default_deadline_ms
        self._jwt = jwt
        self._retry_jitter_ms = retry_jitter_ms
        self._closed = False

    # ---------------------------------------------------- Context manager

    async def __aenter__(self) -> SwarmClient:
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.close()

    async def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        with contextlib.suppress(Exception):
            await self._transport.close()

    # ----------------------------------------------------------- Public API

    async def execute(
        self,
        spec: SwarmSpec,
        *,
        raw_prompt: str = "",
        tenant: str = "default",
        request_id: str | None = None,
        decode_mode: DecodeMode | None = None,
        deadline_ms: int | None = None,
    ) -> SwarmOutcome:
        """Run one classify/inference and return a :class:`SwarmOutcome`."""
        rid: str = request_id or self._new_request_id()
        req = ExecuteRequest(
            spec=spec,
            raw_prompt=raw_prompt,
            request_id=rid,
            decode_mode=decode_mode or DecodeMode(),
            deadline_ms=deadline_ms,
        )
        metadata = self._build_metadata(tenant=tenant, request_id=rid)
        deadline_s = (deadline_ms or self._default_deadline_ms) / 1000.0

        response = await self._with_retries(
            lambda: self._transport.execute(req, metadata=metadata, deadline_s=deadline_s)
        )

        self._validate_response_verdict(response)
        return SwarmOutcome.from_response(response)

    async def describe(self, spec: SwarmSpec, *, deadline_ms: int | None = None) -> dict[str, Any]:
        """Dry-run validation — returns ``{valid, errors, hbm_bytes_estimate, rounds_expected}``."""
        deadline_s = (deadline_ms or self._default_deadline_ms) / 1000.0
        try:
            return await self._transport.describe_spec(spec, deadline_s=deadline_s)
        except EfePlatformError:
            raise
        except Exception as exc:  # pragma: no cover — defensive
            raise SwarmUnavailable(f"describe_spec failed: {exc}") from exc

    async def execute_streaming(
        self,
        spec: SwarmSpec,
        *,
        raw_prompt: str = "",
        tenant: str = "default",
        request_id: str | None = None,
        decode_mode: DecodeMode | None = None,
        deadline_ms: int | None = None,
    ) -> AsyncIterator[dict[str, Any]]:
        """Yield stream chunks as dicts — ``{kind, payload}``.

        Streaming is *not* retried — the caller decides what to do if
        the stream breaks mid-flight.
        """
        rid: str = request_id or self._new_request_id()
        req = ExecuteRequest(
            spec=spec,
            raw_prompt=raw_prompt,
            request_id=rid,
            decode_mode=decode_mode or DecodeMode(),
            deadline_ms=deadline_ms,
        )
        metadata = self._build_metadata(tenant=tenant, request_id=rid)
        deadline_s = (deadline_ms or self._default_deadline_ms) / 1000.0
        async for chunk in self._transport.execute_streaming(
            req, metadata=metadata, deadline_s=deadline_s
        ):
            yield chunk

    # ---------------------------------------------------------- Internals

    async def _with_retries(
        self,
        op: Callable[[], Awaitable[ExecuteResponse]],
    ) -> ExecuteResponse:
        attempt = 0
        while True:
            try:
                return await op()
            except EfePlatformError as exc:
                if not exc.retryable or attempt >= self._max_retries:
                    raise
                backoff = self._compute_backoff(attempt)
                _LOG.warning(
                    "swarm.retry attempt=%d code=%s backoff=%.3fs",
                    attempt + 1,
                    exc.code,
                    backoff,
                )
                await asyncio.sleep(backoff)
                attempt += 1
            except asyncio.TimeoutError as exc:
                if attempt >= self._max_retries:
                    raise SwarmDeadlineExceeded("deadline exceeded") from exc
                backoff = self._compute_backoff(attempt)
                await asyncio.sleep(backoff)
                attempt += 1
            except Exception as exc:  # pragma: no cover — unexpected
                raise SwarmUnavailable(f"transport failure: {exc}") from exc

    def _compute_backoff(self, attempt: int) -> float:
        # Exponential with jitter — caps at 2s.
        base: float = min(2.0, 0.1 * float(2**attempt))
        jitter: float = random.uniform(0, float(self._retry_jitter_ms)) / 1000.0
        return float(base + jitter)

    def _build_metadata(
        self, *, tenant: str, request_id: str
    ) -> tuple[tuple[str, str], ...]:
        entries: list[tuple[str, str]] = [
            ("x-request-id", request_id),
            ("x-tenant", tenant),
        ]
        if self._jwt:
            entries.append(("authorization", f"Bearer {self._jwt}"))
        return tuple(entries)

    def _validate_response_verdict(self, response: ExecuteResponse) -> None:
        """Raise on divergent / unspecified verdicts.

        The client turns hostile verdicts into typed exceptions so
        callers don't silently consume garbage output.
        """
        if response.verdict in (Verdict.DIVERGENCE, Verdict.MODE_COLLAPSE):
            report = analyze_verdict(response)
            raise SwarmDivergence(
                f"swarm verdict {response.verdict.name}",
                detail=report.to_dict(),
            )

    @staticmethod
    def _new_request_id() -> str:
        return str(uuid.uuid4())


# ---------------------------------------------------------------------------
#  In-memory transport — used by tests + standalone dev mode
# ---------------------------------------------------------------------------


class InMemorySwarmTransport:
    """Deterministic in-process fake that implements :class:`SwarmTransport`.

    The fake lets you pre-seed responses keyed by ``(tenant, first 32
    chars of raw_prompt)``. Unseeded calls produce a CONVERGED verdict
    with a canned textual echo — enough to smoke-test the bot loop.
    """

    def __init__(self) -> None:
        self._seeded: dict[tuple[str, str], ExecuteResponse] = {}
        self._call_log: list[ExecuteRequest] = []
        self._closed = False

    # --------------------------------------------------------- Seed API

    def seed(
        self,
        *,
        tenant: str,
        prompt_prefix: str,
        response: ExecuteResponse,
    ) -> None:
        self._seeded[(tenant, prompt_prefix[:32])] = response

    @property
    def call_log(self) -> tuple[ExecuteRequest, ...]:
        return tuple(self._call_log)

    # ----------------------------------------------- SwarmTransport impl

    async def execute(
        self,
        req: ExecuteRequest,
        *,
        metadata: tuple[tuple[str, str], ...],
        deadline_s: float,
    ) -> ExecuteResponse:
        if self._closed:
            raise SwarmUnavailable("transport closed")
        self._call_log.append(req)
        tenant = dict(metadata).get("x-tenant", "default")
        key = (tenant, req.raw_prompt[:32])
        if key in self._seeded:
            seeded = self._seeded[key]
            # Preserve the caller's request_id for correlation.
            return ExecuteResponse(
                swarm_id=seeded.swarm_id,
                request_id=req.request_id or seeded.request_id,
                payload=seeded.payload,
                confidence=seeded.confidence,
                verdict=seeded.verdict,
                rounds_executed=seeded.rounds_executed,
                total_ms=seeded.total_ms,
                alignment_delta=seeded.alignment_delta,
            )
        # Default canned response.
        text = f"[stub] tenant={tenant} prompt={req.raw_prompt[:24]!r}"
        return ExecuteResponse(
            swarm_id=str(uuid.uuid4()),
            request_id=req.request_id or str(uuid.uuid4()),
            payload=DecodedPayload(text=text),
            confidence=0.9,
            verdict=Verdict.CONVERGED,
            rounds_executed=3,
            total_ms=12.5,
            alignment_delta=0.04,
        )

    async def execute_streaming(
        self,
        req: ExecuteRequest,
        *,
        metadata: tuple[tuple[str, str], ...],
        deadline_s: float,
    ) -> AsyncIterator[dict[str, Any]]:
        final = await self.execute(req, metadata=metadata, deadline_s=deadline_s)
        # Simulate a 3-chunk stream.
        assert final.payload.text is not None
        text = final.payload.text
        thirds = max(1, len(text) // 3)
        yield {"kind": "text_delta", "payload": text[:thirds]}
        yield {"kind": "text_delta", "payload": text[thirds : 2 * thirds]}
        yield {"kind": "final", "payload": final}

    async def describe_spec(
        self,
        spec: SwarmSpec,
        *,
        deadline_s: float,
    ) -> dict[str, Any]:
        if spec.dim_latent > 65_536:
            raise SwarmInvalidSpec(f"dim_latent too large: {spec.dim_latent}")
        return {
            "valid": True,
            "errors": (),
            "hbm_bytes_estimate": spec.dim_latent * 4 * _pop_total(spec),
            "rounds_expected": min(spec.max_rounds, 8),
        }

    async def close(self) -> None:
        self._closed = True


def _pop_total(spec: SwarmSpec) -> int:
    return sum(p.count for p in spec.population)


# ---------------------------------------------------------------------------
#  Lazy gRPC transport factory
# ---------------------------------------------------------------------------


def _lazy_grpc_transport(endpoint: SwarmEndpoint) -> SwarmTransport:
    """Build a gRPC transport on first use.

    This helper keeps ``grpcio`` and the generated stubs optional — a
    user of the in-memory transport never pays the import cost. If the
    stubs are missing we fall back to the in-memory transport with a
    warning so ``efe-platform serve --standalone`` remains usable.
    """
    try:
        from efe_platform.swarm._grpc_transport import GrpcSwarmTransport

        return GrpcSwarmTransport(endpoint)
    except Exception as exc:  # pragma: no cover — missing grpcio path
        _LOG.warning(
            "gRPC stubs unavailable (%s); falling back to InMemorySwarmTransport. "
            "Install efe-tool or set transport= explicitly for production use.",
            exc,
        )
        return InMemorySwarmTransport()


# ---------------------------------------------------------------------------
#  Timing helper used in tests
# ---------------------------------------------------------------------------


def _now_ms() -> int:
    return int(time.monotonic() * 1000)


__all__ = [
    "InMemorySwarmTransport",
    "SwarmClient",
    "SwarmEndpoint",
    "SwarmError",
    "SwarmTransport",
]

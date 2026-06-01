"""Orchestrator client — abstract protocol + two concrete impls.

The worker talks to M03 via **four RPCs** (see M04 architecture §3.1):

    Heartbeat          — every ``transport.heartbeat_interval_s`` seconds
    ReportCheckpoint   — after every checkpoint save
    ReportResult       — once, on successful run completion
    ReportError        — once, on fatal error

In production the worker would import the generated gRPC stub from
``efe.v1.worker_pb2_grpc``. For this first iteration we ship two
concrete clients that deliberately do not depend on codegen:

* :class:`HttpOrchestratorClient` — calls the matching REST endpoints
  exposed by :mod:`efe_orch.api.http_app` (``/v1/orch/workers/*``).
  The wire format is JSON, which keeps the worker image small and
  makes debugging trivial.
* :class:`FakeOrchestratorClient` — pure in-memory; records every
  request on public lists for test assertions.

Both implement the :class:`OrchestratorClient` protocol so the driver
has a single injection point.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol

from efe_worker.errors import OrchestratorUnreachable, TransportError


@dataclass(frozen=True, slots=True)
class HeartbeatRequest:
    run_id: str
    worker_id: str
    beat_seq: int
    state_hint: str = "training"
    progress_ratio: float = 0.0
    step: int = 0
    extra: dict[str, float] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class CheckpointRequest:
    run_id: str
    worker_id: str
    step: int
    blob_uri: str
    metrics: dict[str, float] = field(default_factory=dict)
    size_bytes: int = 0
    is_best: bool = False


@dataclass(frozen=True, slots=True)
class ResultRequest:
    run_id: str
    worker_id: str
    artifact_url: str
    metrics: dict[str, float] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class ErrorRequest:
    run_id: str
    worker_id: str
    error_code: str
    error_message: str


class OrchestratorClient(Protocol):
    async def heartbeat(self, req: HeartbeatRequest) -> bool:  # pragma: no cover
        ...

    async def report_checkpoint(
        self, req: CheckpointRequest,
    ) -> bool:  # pragma: no cover
        ...

    async def report_result(
        self, req: ResultRequest,
    ) -> bool:  # pragma: no cover
        ...

    async def report_error(
        self, req: ErrorRequest,
    ) -> bool:  # pragma: no cover
        ...

    async def close(self) -> None:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# Fake client — in-memory, records everything
# ---------------------------------------------------------------------------


@dataclass
class FakeOrchestratorClient:
    """Deterministic in-memory stub for tests.

    Every request is appended to the corresponding list, in order.
    Success / failure is controlled via :attr:`fail_heartbeat` etc.
    """

    heartbeats: list[HeartbeatRequest] = field(default_factory=list)
    checkpoints: list[CheckpointRequest] = field(default_factory=list)
    results: list[ResultRequest] = field(default_factory=list)
    errors: list[ErrorRequest] = field(default_factory=list)
    closed: bool = False
    fail_heartbeat: bool = False
    fail_checkpoint: bool = False
    fail_result: bool = False
    fail_error: bool = False

    async def heartbeat(self, req: HeartbeatRequest) -> bool:
        if self.fail_heartbeat:
            raise OrchestratorUnreachable("fake HB configured to fail")
        self.heartbeats.append(req)
        return True

    async def report_checkpoint(self, req: CheckpointRequest) -> bool:
        if self.fail_checkpoint:
            raise OrchestratorUnreachable("fake ckpt configured to fail")
        self.checkpoints.append(req)
        return True

    async def report_result(self, req: ResultRequest) -> bool:
        if self.fail_result:
            raise OrchestratorUnreachable("fake result configured to fail")
        self.results.append(req)
        return True

    async def report_error(self, req: ErrorRequest) -> bool:
        if self.fail_error:
            raise OrchestratorUnreachable("fake error-report configured to fail")
        self.errors.append(req)
        return True

    async def close(self) -> None:
        self.closed = True


# ---------------------------------------------------------------------------
# HTTP client (uses the M03 REST endpoints)
# ---------------------------------------------------------------------------


class HttpOrchestratorClient:
    """Thin async HTTP client against the orchestrator REST API.

    We deliberately avoid gRPC codegen for the MVP: the orchestrator
    already exposes equivalent endpoints at
    ``/v1/orch/workers/{heartbeat,checkpoint,result,error}`` and the
    JSON payloads are 1:1 with our dataclasses.

    httpx is the transport. On transient failures (5xx, timeouts) we
    retry up to ``max_retries`` with an exponential backoff capped at
    ``max_backoff``.
    """

    def __init__(
        self,
        base_url: str,
        *,
        timeout_s: float = 5.0,
        max_retries: int = 3,
        max_backoff_s: float = 8.0,
    ) -> None:
        try:
            import httpx
        except ImportError as exc:  # pragma: no cover — optional dep
            raise TransportError(
                "httpx is required for HttpOrchestratorClient",
            ) from exc
        self._httpx = httpx
        self._base = base_url.rstrip("/")
        self._timeout = timeout_s
        self._max_retries = max_retries
        self._max_backoff = max_backoff_s
        self._client: Any | None = None

    async def _get_client(self) -> Any:
        if self._client is None:
            self._client = self._httpx.AsyncClient(timeout=self._timeout)
        return self._client

    async def _post(self, path: str, body: dict[str, Any]) -> bool:
        import asyncio

        client = await self._get_client()
        url = f"{self._base}{path}"
        backoff = 0.2
        for attempt in range(self._max_retries + 1):
            try:
                resp = await client.post(url, json=body)
            except Exception as exc:
                if attempt >= self._max_retries:
                    raise OrchestratorUnreachable(
                        f"POST {path} failed after {attempt + 1} attempts: {exc}",
                    ) from exc
                await asyncio.sleep(min(backoff, self._max_backoff))
                backoff *= 2
                continue
            if resp.status_code >= 500:
                if attempt >= self._max_retries:
                    raise TransportError(
                        f"orchestrator returned {resp.status_code} "
                        f"for {path}",
                        detail={"body": resp.text[:400]},
                    )
                await asyncio.sleep(min(backoff, self._max_backoff))
                backoff *= 2
                continue
            if resp.status_code >= 400:
                raise TransportError(
                    f"orchestrator rejected {path}: {resp.status_code}",
                    detail={"body": resp.text[:400]},
                )
            return True
        return False  # pragma: no cover — unreachable

    async def heartbeat(self, req: HeartbeatRequest) -> bool:
        return await self._post("/v1/orch/workers/heartbeat", _to_dict(req))

    async def report_checkpoint(self, req: CheckpointRequest) -> bool:
        return await self._post("/v1/orch/workers/checkpoint", _to_dict(req))

    async def report_result(self, req: ResultRequest) -> bool:
        return await self._post("/v1/orch/workers/result", _to_dict(req))

    async def report_error(self, req: ErrorRequest) -> bool:
        return await self._post("/v1/orch/workers/error", _to_dict(req))

    async def close(self) -> None:
        if self._client is not None:
            await self._client.aclose()
            self._client = None


def _to_dict(req: Any) -> dict[str, Any]:
    """Frozen-dataclass → plain dict for JSON POST."""
    out: dict[str, Any] = {}
    for f in req.__dataclass_fields__:
        v = getattr(req, f)
        if isinstance(v, dict):
            out[f] = dict(v)
        else:
            out[f] = v
    return out


__all__ = [
    "CheckpointRequest",
    "ErrorRequest",
    "FakeOrchestratorClient",
    "HeartbeatRequest",
    "HttpOrchestratorClient",
    "OrchestratorClient",
    "ResultRequest",
]

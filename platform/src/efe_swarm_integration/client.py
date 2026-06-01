"""SwarmClient — the M12 public API.

Consumers (M06, M07, M10, Mode 1) hold a single :class:`SwarmClient`
instance per process. The client orchestrates :

1. AuthN/AuthZ via M11 (injected ``pdp`` + ``audit``).
2. Tenant bulkhead + circuit breaker.
3. Embedding via an injected :class:`EmbedderService`.
4. HBM + deadline budget check.
5. Contract adapter build / parse.
6. Reconciliation → :class:`VerdictBundle` / :class:`ClassifyResponse`.
7. Telemetry (metrics + spans + audit).
"""

from __future__ import annotations

import logging
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, AsyncIterator, Protocol

from efe_swarm_integration.budget.hbm import HbmBudgeter
from efe_swarm_integration.catalog.bridge import CatalogBridge
from efe_swarm_integration.config import SwarmIntegrationConfig
from efe_swarm_integration.contract import (
    ContractAdapter,
    parse_describe_spec_reply,
    parse_execute_response,
)
from efe_swarm_integration.errors import (
    CatalogNotFound,
    CircuitOpen,
    PiiDetectedInPrompt,
    QuotaExceeded,
    SwarmUnavailable,
    UnauthorisedM12,
)
from efe_swarm_integration.flow_control.bulkhead import BulkheadRegistry
from efe_swarm_integration.flow_control.circuit_breaker import (
    CircuitBreakerRegistry,
)
from efe_swarm_integration.reconciler.decoder import (
    ClassifyResponse,
    VerdictReconciler,
    reconcile,
)
from efe_swarm_integration.streaming.server_stream import (
    StreamEvent,
    consume_stream,
)
from efe_swarm_integration.telemetry import get_registry
from efe_swarm_integration.transport.interceptors.tenant import (
    build_tenant_metadata,
)
from efe_swarm_integration.transport.rpc_driver import RpcDriver
from efe_swarm_integration.types import (
    ConeRef,
    DecodeMode,
    DescribeSpecReply,
    ExecuteRequest,
    ExecuteResponse,
    SwarmSpec,
    VerdictBundle,
    default_spec_for_classify,
)

_LOG = logging.getLogger("efe_swarm_integration.client")

__all__ = [
    "ClassifyRequest",
    "EmbedderService",
    "PdpPort",
    "PiiLinterPort",
    "SwarmClient",
]


class EmbedderService(Protocol):
    async def embed(self, *texts: str) -> list[tuple[float, ...]]: ...  # pragma: no cover


class PdpPort(Protocol):
    async def authorise(
        self, *, actor_id: str, tenant_id: str, action: str, resource_kind: str
    ) -> bool: ...  # pragma: no cover


class PiiLinterPort(Protocol):
    def find(self, text: str) -> list[Any]: ...  # pragma: no cover


@dataclass(frozen=True, slots=True)
class ClassifyRequest:
    text: str
    catalog_key: tuple[str, str]  # (feature_id, task_id | None)
    tenant_id: str
    task_id: str | None = None
    actor_id: str = "service"
    max_rounds: int = 8
    budget_ms: int = 500
    idempotency_key: str | None = None


@dataclass(slots=True)
class SwarmClient:
    """Central access point for the platform → swarm bridge."""

    driver: RpcDriver
    config: SwarmIntegrationConfig
    embedder: EmbedderService
    catalog: CatalogBridge
    bulkheads: BulkheadRegistry
    breakers: CircuitBreakerRegistry
    adapter: ContractAdapter = field(default_factory=ContractAdapter)
    reconciler: VerdictReconciler = field(default_factory=VerdictReconciler)
    budgeter: HbmBudgeter = field(
        default_factory=lambda: HbmBudgeter(
            estimator=__import__(
                "efe_swarm_integration.budget.estimator", fromlist=["HbmEstimator"]
            ).HbmEstimator()
        )
    )
    pdp: PdpPort | None = None
    pii_linter: PiiLinterPort | None = None

    # ------------------------------------------------------------------
    # Public entry points
    # ------------------------------------------------------------------

    async def classify(self, req: ClassifyRequest) -> ClassifyResponse:
        """Mode 1 classification — fast path."""
        await self._authorise(req.actor_id, req.tenant_id, "swarm.classify")
        feature_id, task_id = req.catalog_key
        cone_ref = await self.catalog.resolve(
            tenant_id=req.tenant_id, feature_id=feature_id, task_id=task_id
        )
        labels = await self.catalog.labels_for(
            tenant_id=req.tenant_id, feature_id=feature_id, task_id=task_id
        )

        vectors = await self.embedder.embed(req.text)
        if not vectors:
            raise CatalogNotFound("embedder returned no vectors")
        embedding = vectors[0]

        spec = default_spec_for_classify(
            embedding=embedding,
            cone_ref=cone_ref,
            budget_ms=req.budget_ms,
            max_rounds=req.max_rounds,
        )
        if req.idempotency_key is not None:
            # store key for audit ; RPC echoes via metadata.
            pass

        execute_req = ExecuteRequest(
            spec=spec,
            decode_mode=DecodeMode(kind="pass_through"),
            request_id="",
            raw_prompt=None,
            deadline=None,
        )
        bundle = await self._execute(
            execute_req,
            tenant_id=req.tenant_id,
            task_id=task_id,
            actor_id=req.actor_id,
            idempotency_key=req.idempotency_key,
            cone_radius=0.35,  # fallback ; TODO: pull from binding
        )
        return self.reconciler.to_classify(
            bundle,
            labels=list(labels) if labels else [],
            anchors=None,
            cone_radius=0.35,
        )

    async def execute(
        self,
        req: ExecuteRequest,
        *,
        tenant_id: str,
        task_id: str | None = None,
        actor_id: str = "service",
        idempotency_key: str | None = None,
        cone_radius: float = 0.35,
    ) -> VerdictBundle:
        await self._authorise(actor_id, tenant_id, "swarm.execute")
        # PII guard on raw_prompt.
        if req.raw_prompt is not None and self.pii_linter is not None:
            if self.pii_linter.find(req.raw_prompt):
                raise PiiDetectedInPrompt(
                    "raw_prompt carries PII", detail={"len": len(req.raw_prompt)}
                )
        return await self._execute(
            req,
            tenant_id=tenant_id,
            task_id=task_id,
            actor_id=actor_id,
            idempotency_key=idempotency_key,
            cone_radius=cone_radius,
        )

    async def execute_streaming(
        self,
        req: ExecuteRequest,
        *,
        tenant_id: str,
        task_id: str | None = None,
        actor_id: str = "service",
        idempotency_key: str | None = None,
    ) -> AsyncIterator[StreamEvent]:
        await self._authorise(actor_id, tenant_id, "swarm.execute_streaming")
        payload = self.adapter.build_execute_request(req)
        self.budgeter.verify(req.spec)
        metadata = build_tenant_metadata(
            tenant_id=tenant_id,
            task_id=task_id,
            request_id=req.request_id or None,
            idempotency_key=idempotency_key,
            actor_id=actor_id,
        )
        async with await self.bulkheads.for_tenant(tenant_id):
            try:
                cb = await self.breakers.for_tenant(tenant_id)
                await cb.pre_call()
                chunks_iter = self.driver.server_streaming(
                    method="/efe.v1.SwarmService/ExecuteStreaming",
                    payload=payload,
                    metadata=metadata,
                    deadline_s=None,
                )
                # Await the coroutine-returning factory.
                iterator: AsyncIterator[Any] = await chunks_iter  # type: ignore[assignment]
                ok = False
                try:
                    async for event in consume_stream(
                        _dict_iterator_from_rpc(iterator)
                    ):
                        yield event
                        if event.kind == "final":
                            ok = True
                finally:
                    await cb.record(success=ok)
            except CircuitOpen:
                raise
            except Exception:
                raise

    async def describe_spec(
        self, spec: SwarmSpec, *, tenant_id: str, deadline_s: float = 2.0
    ) -> DescribeSpecReply:
        payload = {"spec": self.adapter.build_swarm_spec(spec)}
        metadata = build_tenant_metadata(tenant_id=tenant_id)
        try:
            resp = await self.driver.unary(
                method="/efe.v1.SwarmService/DescribeSpec",
                payload=payload,
                metadata=metadata,
                deadline_s=deadline_s,
            )
        except Exception as exc:
            raise SwarmUnavailable(
                f"DescribeSpec failed: {exc}"
            ) from exc
        return parse_describe_spec_reply(resp.payload)

    # ------------------------------------------------------------------
    # Internals
    # ------------------------------------------------------------------

    async def _authorise(self, actor_id: str, tenant_id: str, action: str) -> None:
        if self.pdp is None:
            return
        ok = await self.pdp.authorise(
            actor_id=actor_id,
            tenant_id=tenant_id,
            action=action,
            resource_kind="swarm",
        )
        if not ok:
            raise UnauthorisedM12(
                f"principal {actor_id!r} not allowed {action!r}",
            )

    async def _execute(
        self,
        req: ExecuteRequest,
        *,
        tenant_id: str,
        task_id: str | None,
        actor_id: str,
        idempotency_key: str | None,
        cone_radius: float,
    ) -> VerdictBundle:
        self.budgeter.verify(req.spec)
        payload = self.adapter.build_execute_request(req)
        metadata = build_tenant_metadata(
            tenant_id=tenant_id,
            task_id=task_id,
            request_id=req.request_id or None,
            idempotency_key=idempotency_key,
            actor_id=actor_id,
        )
        async with await self.bulkheads.for_tenant(tenant_id):
            cb = await self.breakers.for_tenant(tenant_id)
            await cb.pre_call()
            start = time.monotonic()
            ok = False
            try:
                resp = await self.driver.unary(
                    method="/efe.v1.SwarmService/Execute",
                    payload=payload,
                    metadata=metadata,
                    deadline_s=(req.spec.budget.max_wall_clock_ms + 200) / 1000.0,
                )
                response: ExecuteResponse = parse_execute_response(resp.payload)
                bundle = reconcile(response, cone_radius=cone_radius)
                ok = True
                # Metrics
                reg = get_registry()
                verdict_metric = reg.get("efe_m12_verdict_total")
                if verdict_metric is not None:
                    try:
                        verdict_metric.inc(verdict=bundle.verdict.value)
                    except Exception:  # pragma: no cover
                        pass
                hist = reg.get("efe_m12_alignment_delta")
                if hist is not None:
                    try:
                        hist.observe(bundle.alignment_delta)
                    except Exception:  # pragma: no cover
                        pass
                return bundle
            finally:
                await cb.record(success=ok)
                latency = max(0.0, time.monotonic() - start)
                reg = get_registry()
                lat = reg.get("efe_m12_rpc_latency_seconds")
                if lat is not None:
                    try:
                        lat.observe(latency, method="Execute")
                    except Exception:  # pragma: no cover
                        pass
                total = reg.get("efe_m12_rpc_total")
                if total is not None:
                    try:
                        total.inc(
                            tenant=tenant_id,
                            method="Execute",
                            status="OK" if ok else "FAILED",
                        )
                    except Exception:  # pragma: no cover
                        pass

    async def close(self) -> None:
        """Release any transport resources."""
        # The driver is owned by the pool — external close orchestrated there.
        pass


async def _dict_iterator_from_rpc(it: AsyncIterator[Any]) -> AsyncIterator[dict[str, Any]]:
    """Convert :class:`RpcResponse` stream into a plain dict stream."""
    async for r in it:
        yield r.payload if hasattr(r, "payload") else r


# Keep imports live
_ = datetime, timezone, QuotaExceeded, ConeRef

"""Implementation of ``efe-admin healthcheck``.

Probes the server's grpc-health endpoint (or falls back to
:meth:`EfeClient._healthcheck_via_describe`). Always emits a
schema-versioned document on stdout.

Exit codes:

    0  — SERVING
    5  — NOT_SERVING (or transport error)
"""

from __future__ import annotations

import time
from typing import Any

from ..config import ResolvedConfig
from ..errors import NetworkError, from_bridge_error
from ..logging import Logger
from ..output import build_healthcheck_document, render


def run(cfg: ResolvedConfig, log: Logger) -> int:
    log.info("healthcheck_start", endpoint=cfg.endpoint)

    # Lazy imports — this subcommand is often called inside a loop
    # by a Kubernetes readiness probe, so we want a lean startup.
    from efe_tool.errors import BridgeError

    try:
        from efe_tool.embedder import LocalSTEmbedder
        from efe_tool.client import EfeClient

        # A sentence-transformers instance is required to construct the
        # client, but it stays lazy — no torch weight is loaded unless
        # we actually call ``.encode()``. healthcheck never does.
        embedder = LocalSTEmbedder(cfg.embedder_model)

        client = EfeClient(
            endpoint=cfg.endpoint,
            embedder=embedder,
            jwt=cfg.jwt,
            tenant=cfg.tenant,
            timeout_s=cfg.timeout_s,
            insecure=not cfg.tls,
        )
    except BridgeError as e:
        raise from_bridge_error(e) from e

    t0 = time.perf_counter()
    serving = False
    try:
        try:
            serving = bool(client.healthcheck())
        except BridgeError as e:
            raise from_bridge_error(e) from e
    finally:
        try:
            client.close()
        except Exception as e:  # pragma: no cover
            log.warn("healthcheck_close_failed", error=str(e))
    latency_ms = int((time.perf_counter() - t0) * 1000)

    doc = build_healthcheck_document(
        serving=serving,
        latency_ms=latency_ms,
        endpoint=cfg.endpoint,
        request_id=cfg.request_id,
    )
    render(doc, format=cfg.format, pretty=cfg.pretty)

    log.info(
        "healthcheck_done",
        serving=serving,
        latency_ms=latency_ms,
        endpoint=cfg.endpoint,
    )
    if not serving:
        raise NetworkError(
            f"server not serving at {cfg.endpoint}",
            hint="is `efe-server` up? check logs; try --timeout-s 5",
            context={"endpoint": cfg.endpoint, "latency_ms": latency_ms},
        )
    return 0


__all__ = ["run"]

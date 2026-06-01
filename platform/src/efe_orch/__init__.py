"""EFE Training Orchestrator — M03.

The *brain* of the fine-tuning service. This package owns:

    * the run state machine (``DRAFT → … → SUCCEEDED | FAILED``),
    * the resource scheduler (pool selection + quota enforcement),
    * the Kubernetes dispatcher (worker Job manifest rendering),
    * the heartbeat reaper (worker liveness monitoring),
    * the retry / error-classification engine,
    * the gRPC + REST API used by M05 (CLI), M02 (labeling UI) and
      the worker fleet,
    * an append-only audit of every state transition.

Relation to the rest of the platform:

    efe_ft        — M01 Feature Store + M02 Labeling / AL
    efe_orch      — **M03 Training Orchestrator (this package)**
    efe_platform  — swarm orchestration client library

The three packages run as separate services in production; they share
only the DB schema they each own. This package is strictly typed,
tested with ``pytest-asyncio`` and designed to boot without any
Kubernetes or Postgres access (both have portable fakes).

Public surface (stable):

    efe_orch.errors          — exception hierarchy
    efe_orch.config          — pydantic-settings configuration
    efe_orch.db              — SQLAlchemy engine + 5 tables
    efe_orch.state_machine   — FSM + advisory locking
    efe_orch.scheduler       — pool selection + quota gate
    efe_orch.dispatcher      — K8s Job renderer + client (+ fake)
    efe_orch.retry           — error classifier + backoff policy
    efe_orch.heartbeat       — recorder + reaper
    efe_orch.services        — run + worker domain services
    efe_orch.observability   — metrics + structured logging
    efe_orch.api             — FastAPI HTTP surface
"""

from __future__ import annotations

from efe_orch.version import VERSION, __version__

__all__ = ["VERSION", "__version__"]

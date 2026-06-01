"""Run context — the mutable state carried through a single run.

Everything that the outer driver, the transport layer, the data
loader, and the method objects need to share lives on this bundle.
The split is:

* :class:`RunIdentity` (static, frozen, from :mod:`efe_worker.config`).
* :class:`RunContext`  — composition of identity + dynamic state.

``RunContext`` owns:

* ``phase`` — current coarse lifecycle phase (``boot`` / ``loading`` /
  ``training`` / ``eval`` / ``upload``). Used by the heartbeat loop
  to fill the ``state_hint`` field.
* ``step`` — current training step counter (1-based once training
  begins; 0 before).
* ``live_metrics`` — last-observed scalar metrics (loss, lr, gpu_util,
  …) snapshotted for the next heartbeat.
* ``progress_ratio`` — best-effort estimate in ``[0, 1]``; the driver
  updates it at each step.
* ``stop_event`` — ``asyncio.Event`` that any component can set to
  trigger graceful shutdown (SIGTERM path, deadline).

The transport + filesystem dependencies (S3, gRPC stub, clock,
checkpoint manager) are *injected* so tests can swap fakes freely.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import datetime
from typing import TYPE_CHECKING

from efe_worker.clock import Clock, SystemClock
from efe_worker.config import RunIdentity, WorkerSettings

if TYPE_CHECKING:  # pragma: no cover — type-only
    from efe_worker.transport.grpc_client import OrchestratorClient
    from efe_worker.transport.s3 import BlobStore


#: Coarse lifecycle phases emitted in heartbeat ``state_hint``.
PHASES = frozenset({
    "boot", "loading", "training", "eval", "upload", "done", "error",
})


@dataclass
class RunContext:
    """Dynamic state carried through a run.

    Construction is cheap; the usual pattern is:

        ctx = RunContext(settings=settings, clock=SystemClock(),
                         transport=transport, blob_store=blob_store)

    Access is **not** thread-safe — the driver runs on a single
    asyncio event loop.
    """

    settings: WorkerSettings
    clock: Clock = field(default_factory=SystemClock)
    transport: OrchestratorClient | None = None
    blob_store: BlobStore | None = None

    # ---- dynamic state -------------------------------------------------

    phase: str = "boot"
    step: int = 0
    progress_ratio: float = 0.0
    live_metrics: dict[str, float] = field(default_factory=dict)
    started_at: datetime | None = None
    stop_event: asyncio.Event = field(default_factory=asyncio.Event)

    # ---- run identity shortcut ----------------------------------------

    @property
    def identity(self) -> RunIdentity:
        return self.settings.identity

    @property
    def run_id(self) -> str:
        return self.identity.run_id

    @property
    def worker_id(self) -> str:
        return self.identity.worker_id

    @property
    def tenant_id(self) -> str:
        return self.identity.tenant_id

    # ---- phase transitions --------------------------------------------

    def set_phase(self, phase: str) -> None:
        if phase not in PHASES:
            raise ValueError(
                f"unknown phase {phase!r}, expected one of {sorted(PHASES)}",
            )
        self.phase = phase

    def mark_started(self) -> None:
        if self.started_at is None:
            self.started_at = self.clock.now()

    def record(self, *, step: int | None = None, **metrics: float) -> None:
        """Update ``live_metrics`` and optionally ``step``.

        Called every N steps from the training loop. Values are floats
        so the heartbeat RPC can ship them unchanged in the ``extra``
        map.
        """
        if step is not None:
            self.step = int(step)
        for k, v in metrics.items():
            try:
                self.live_metrics[k] = float(v)
            except (TypeError, ValueError):
                # Silently skip non-numeric (keep the map clean).
                continue

    def progress(self) -> float:
        """Return the best-effort progress ratio, clamped to [0, 1]."""
        r = self.progress_ratio
        if r < 0.0:
            return 0.0
        if r > 1.0:
            return 1.0
        return float(r)

    def request_stop(self) -> None:
        """Signal every async component to wind down."""
        self.stop_event.set()

    def should_stop(self) -> bool:
        return self.stop_event.is_set()

    # ---- user-deadline check (optional) -------------------------------

    def user_deadline_reached(self) -> bool:
        dl = self.settings.training.user_deadline_seconds
        if dl is None or self.started_at is None:
            return False
        elapsed = (self.clock.now() - self.started_at).total_seconds()
        return elapsed >= dl


__all__ = ["PHASES", "RunContext"]

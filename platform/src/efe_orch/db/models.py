"""SQLAlchemy 2.0 declarative models for the 5 orchestrator tables.

Schema (mirrors the architecture doc §11):

    1. ``finetune_runs``          — main row; one per run, with FSM state.
    2. ``run_state_transitions``  — append-only transition audit trail.
    3. ``worker_heartbeats``      — worker liveness beacons.
    4. ``run_checkpoints``        — resume points reported by workers.
    5. ``tenant_quotas``          — per-tenant concurrency + monthly caps.

Portable dialect support: SQLite for tests, Postgres for prod. The
:class:`UUIDStr` and :class:`JsonType` TypeDecorators map to
``TEXT`` / ``JSON`` on SQLite and to ``UUID`` / ``JSONB`` on Postgres
so the application layer always deals with ``str`` / ``dict``.
"""

from __future__ import annotations

import uuid
from datetime import datetime, timezone
from typing import Any

from sqlalchemy import (
    BigInteger,
    Boolean,
    CheckConstraint,
    DateTime,
    Float,
    ForeignKey,
    Index,
    Integer,
    String,
    Text,
    UniqueConstraint,
)
from sqlalchemy.dialects.postgresql import JSONB
from sqlalchemy.dialects.postgresql import UUID as PG_UUID
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship
from sqlalchemy.types import JSON, TypeDecorator


def _utcnow() -> datetime:
    return datetime.now(timezone.utc)


# ---------------------------------------------------------------------------
#  Portable types (SQLite / Postgres)
# ---------------------------------------------------------------------------


class UUIDStr(TypeDecorator[str]):
    """UUIDs stored as ``TEXT`` in SQLite, ``UUID`` in Postgres.

    Python side is always ``str`` — callers never see a
    :class:`uuid.UUID` instance. Stored bytes are 36 chars on SQLite,
    16 on Postgres.
    """

    impl = String(36)
    cache_ok = True

    def load_dialect_impl(self, dialect: Any) -> Any:
        if dialect.name == "postgresql":
            return dialect.type_descriptor(PG_UUID(as_uuid=False))
        return dialect.type_descriptor(String(36))

    def process_bind_param(self, value: Any, dialect: Any) -> Any:
        if value is None:
            return None
        if isinstance(value, uuid.UUID):
            return str(value)
        return str(value)

    def process_result_value(self, value: Any, dialect: Any) -> Any:
        return None if value is None else str(value)


class JsonType(TypeDecorator[Any]):
    """JSONB on Postgres, plain JSON on SQLite."""

    impl = JSON
    cache_ok = True

    def load_dialect_impl(self, dialect: Any) -> Any:
        if dialect.name == "postgresql":
            return dialect.type_descriptor(JSONB())
        return dialect.type_descriptor(JSON())


# ---------------------------------------------------------------------------
#  Base
# ---------------------------------------------------------------------------


class Base(DeclarativeBase):
    """Declarative base for every ORM class in efe_orch."""


# ---------------------------------------------------------------------------
#  finetune_runs — the primary row
# ---------------------------------------------------------------------------


class FinetuneRun(Base):
    """One row per submitted fine-tuning run.

    The row lives through every FSM state; ``state`` is mutated in
    place by the state-machine transition function (under advisory
    lock) and a shadow row is appended to ``run_state_transitions``.

    Index rationale:

    * ``(tenant_id, created_at)`` — primary "list my recent runs" query.
    * ``(state, pool)``           — driver for the reconciler loop.
    * ``(worker_id)``             — reaper path (worker → its runs).
    * ``(snapshot_id)``           — links back to M01 snapshot.
    """

    __tablename__ = "finetune_runs"
    __table_args__ = (
        CheckConstraint(
            "state IN ("
            "'draft','queued','scheduled','training','evaluating',"
            "'uploading','succeeded','failed','cancelling','cancelled',"
            "'retrying','lost'"
            ")",
            name="valid_state",
        ),
        CheckConstraint(
            "priority IN ('standard','premium')", name="valid_priority",
        ),
        CheckConstraint("attempt >= 0", name="attempt_nonneg"),
        CheckConstraint(
            "retry_count >= 0 AND retry_count <= 16",
            name="retry_bounded",
        ),
        Index("ix_runs_tenant_created", "tenant_id", "created_at"),
        Index("ix_runs_tenant_state", "tenant_id", "state"),
        Index("ix_runs_state_pool", "state", "pool"),
        Index("ix_runs_worker", "worker_id"),
        Index("ix_runs_snapshot", "snapshot_id"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False, index=True)
    task_id: Mapped[str] = mapped_column(Text, nullable=False)

    # --- identity / lineage ---
    submitted_by: Mapped[str] = mapped_column(Text, nullable=False)
    snapshot_id: Mapped[str] = mapped_column(UUIDStr, nullable=False)
    base_model: Mapped[str] = mapped_column(
        Text, nullable=False,
        default="sentence-transformers/all-MiniLM-L6-v2",
    )
    recipe_version: Mapped[str] = mapped_column(
        String(32), nullable=False, default="v1",
    )

    # --- FSM state ---
    state: Mapped[str] = mapped_column(
        String(16), nullable=False, default="draft", index=True,
    )
    state_version: Mapped[int] = mapped_column(
        Integer, nullable=False, default=0,
        comment="Optimistic-concurrency counter; bumped on every transition.",
    )
    priority: Mapped[str] = mapped_column(
        String(16), nullable=False, default="standard",
    )

    # --- placement ---
    pool: Mapped[str | None] = mapped_column(String(64), nullable=True)
    worker_id: Mapped[str | None] = mapped_column(String(64), nullable=True)
    k8s_job_name: Mapped[str | None] = mapped_column(Text, nullable=True)

    # --- retry accounting ---
    attempt: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    retry_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    next_retry_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    last_error_code: Mapped[str | None] = mapped_column(String(64), nullable=True)
    last_error_message: Mapped[str | None] = mapped_column(Text, nullable=True)

    # --- results ---
    result_artifact_url: Mapped[str | None] = mapped_column(Text, nullable=True)
    result_metrics: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    hyperparameters: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    tags: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )

    # --- timestamps ---
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
        default=_utcnow, onupdate=_utcnow,
    )
    queued_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    started_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    finished_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )

    # --- relationships ---
    transitions: Mapped[list[RunStateTransition]] = relationship(
        back_populates="run",
        cascade="all, delete-orphan",
        order_by="RunStateTransition.created_at",
    )
    heartbeats: Mapped[list[WorkerHeartbeat]] = relationship(
        back_populates="run",
        cascade="all, delete-orphan",
        order_by="WorkerHeartbeat.observed_at",
    )
    checkpoints: Mapped[list[RunCheckpoint]] = relationship(
        back_populates="run",
        cascade="all, delete-orphan",
        order_by="RunCheckpoint.step",
    )

    def as_dict(self) -> dict[str, Any]:
        """Serialise to a primitive dict suitable for JSON responses."""
        return {
            "id": self.id,
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "submitted_by": self.submitted_by,
            "snapshot_id": self.snapshot_id,
            "base_model": self.base_model,
            "recipe_version": self.recipe_version,
            "state": self.state,
            "state_version": self.state_version,
            "priority": self.priority,
            "pool": self.pool,
            "worker_id": self.worker_id,
            "k8s_job_name": self.k8s_job_name,
            "attempt": self.attempt,
            "retry_count": self.retry_count,
            "next_retry_at": _iso(self.next_retry_at),
            "last_error_code": self.last_error_code,
            "last_error_message": self.last_error_message,
            "result_artifact_url": self.result_artifact_url,
            "result_metrics": dict(self.result_metrics or {}),
            "hyperparameters": dict(self.hyperparameters or {}),
            "tags": dict(self.tags or {}),
            "created_at": _iso(self.created_at),
            "updated_at": _iso(self.updated_at),
            "queued_at": _iso(self.queued_at),
            "started_at": _iso(self.started_at),
            "finished_at": _iso(self.finished_at),
        }


# ---------------------------------------------------------------------------
#  run_state_transitions — audit trail
# ---------------------------------------------------------------------------


class RunStateTransition(Base):
    """Append-only transition log.

    Every ``(from, to)`` state change for any run produces one row.
    This table is the source of truth for:

    * Audit / compliance replays,
    * Dashboards ("median time in QUEUED"),
    * Debugging ("why did this run retry?") via the ``reason`` column.
    """

    __tablename__ = "run_state_transitions"
    __table_args__ = (
        Index("ix_trans_run_created", "run_id", "created_at"),
        Index("ix_trans_to_state", "to_state"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    run_id: Mapped[str] = mapped_column(
        UUIDStr,
        ForeignKey("finetune_runs.id", ondelete="CASCADE"),
        nullable=False,
    )
    from_state: Mapped[str | None] = mapped_column(String(16), nullable=True)
    to_state: Mapped[str] = mapped_column(String(16), nullable=False)
    actor: Mapped[str] = mapped_column(Text, nullable=False)
    reason: Mapped[str] = mapped_column(Text, nullable=False, default="")
    payload: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )

    run: Mapped[FinetuneRun] = relationship(back_populates="transitions")


# ---------------------------------------------------------------------------
#  worker_heartbeats — worker liveness
# ---------------------------------------------------------------------------


class WorkerHeartbeat(Base):
    """One heartbeat per ``(run_id, worker_id, beat_seq)``.

    Workers send a heartbeat every ``heartbeat.interval_s`` seconds;
    the reaper uses the latest ``observed_at`` per run to decide
    whether a worker is still alive. The table is trimmed by a
    scheduled job (retention = 72h by default).
    """

    __tablename__ = "worker_heartbeats"
    __table_args__ = (
        UniqueConstraint(
            "run_id", "worker_id", "beat_seq",
            name="uniq_hb_per_beat",
        ),
        Index("ix_hb_run_observed", "run_id", "observed_at"),
        Index("ix_hb_worker_observed", "worker_id", "observed_at"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    run_id: Mapped[str] = mapped_column(
        UUIDStr,
        ForeignKey("finetune_runs.id", ondelete="CASCADE"),
        nullable=False,
    )
    worker_id: Mapped[str] = mapped_column(String(64), nullable=False)
    beat_seq: Mapped[int] = mapped_column(BigInteger, nullable=False)
    observed_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    state_hint: Mapped[str] = mapped_column(
        String(16), nullable=False, default="training",
    )
    progress_ratio: Mapped[float] = mapped_column(
        Float, nullable=False, default=0.0,
    )
    step: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    extra: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )

    run: Mapped[FinetuneRun] = relationship(back_populates="heartbeats")


# ---------------------------------------------------------------------------
#  run_checkpoints — restart points
# ---------------------------------------------------------------------------


class RunCheckpoint(Base):
    """Worker-reported checkpoints.

    A checkpoint = one snapshot of model weights + optimiser state at
    a specific ``step``. Persisted to blob storage; this table keeps
    the pointer. On retry, the dispatcher picks the latest checkpoint
    for the run to resume from instead of starting from zero.
    """

    __tablename__ = "run_checkpoints"
    __table_args__ = (
        UniqueConstraint("run_id", "step", name="uniq_ckpt_per_step"),
        Index("ix_ckpt_run", "run_id"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    run_id: Mapped[str] = mapped_column(
        UUIDStr,
        ForeignKey("finetune_runs.id", ondelete="CASCADE"),
        nullable=False,
    )
    step: Mapped[int] = mapped_column(Integer, nullable=False)
    blob_uri: Mapped[str] = mapped_column(Text, nullable=False)
    metrics: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    size_bytes: Mapped[int] = mapped_column(BigInteger, nullable=False, default=0)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    is_best: Mapped[bool] = mapped_column(Boolean, nullable=False, default=False)

    run: Mapped[FinetuneRun] = relationship(back_populates="checkpoints")


# ---------------------------------------------------------------------------
#  tenant_quotas — one row per tenant
# ---------------------------------------------------------------------------


class TenantQuota(Base):
    """Tenant-scoped limits enforced before any run is QUEUED.

    Fields are configured from the control plane; absence of a row
    means "use the global defaults from :class:`QuotaSettings`". The
    ``current_*`` counters are maintained by the run-service; a
    scheduled reset job zeroes them at month boundaries.
    """

    __tablename__ = "tenant_quotas"

    tenant_id: Mapped[str] = mapped_column(Text, primary_key=True)
    max_concurrent_runs: Mapped[int] = mapped_column(
        Integer, nullable=False, default=4,
    )
    max_monthly_runs: Mapped[int] = mapped_column(
        Integer, nullable=False, default=200,
    )
    max_monthly_gpu_seconds: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=3_600 * 24,
    )
    allowed_pools: Mapped[list[str]] = mapped_column(
        JsonType, nullable=False, default=list,
    )

    # Usage counters (rolling window = current calendar month).
    current_concurrent_runs: Mapped[int] = mapped_column(
        Integer, nullable=False, default=0,
    )
    current_monthly_runs: Mapped[int] = mapped_column(
        Integer, nullable=False, default=0,
    )
    current_monthly_gpu_seconds: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=0,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
        default=_utcnow, onupdate=_utcnow,
    )


# ---------------------------------------------------------------------------
#  helpers
# ---------------------------------------------------------------------------


def _iso(dt: datetime | None) -> str | None:
    return dt.isoformat() if dt is not None else None


__all__ = [
    "Base",
    "FinetuneRun",
    "JsonType",
    "RunCheckpoint",
    "RunStateTransition",
    "TenantQuota",
    "UUIDStr",
    "WorkerHeartbeat",
]

"""SQLAlchemy 2.0 declarative models for the 8 M05 tables.

Schema (per M05 architecture doc §14, extended with review_requests):

    1. ``eval_runs``         — one row per evaluation
    2. ``eval_metrics``      — per-metric rows attached to an eval
    3. ``golden_sets``       — locked labelled eval datasets
    4. ``champions``         — current champion per (tenant, task)
    5. ``champion_history``  — append-only log of champion changes
    6. ``promotion_gates``   — per-task rule config (jsonb)
    7. ``rollback_monitors`` — post-promote canary observations
    8. ``audit_events``      — signed decision audit chain

Portable types: UUIDStr (TEXT/UUID), JsonType (JSON/JSONB).
"""

from __future__ import annotations

import uuid
from datetime import datetime, timezone
from typing import Any

from sqlalchemy import (
    Boolean,
    CheckConstraint,
    DateTime,
    Float,
    ForeignKey,
    Index,
    Integer,
    LargeBinary,
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
# portable types
# ---------------------------------------------------------------------------


class UUIDStr(TypeDecorator[str]):
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
    impl = JSON
    cache_ok = True

    def load_dialect_impl(self, dialect: Any) -> Any:
        if dialect.name == "postgresql":
            return dialect.type_descriptor(JSONB())
        return dialect.type_descriptor(JSON())


class Base(DeclarativeBase):
    """Declarative base for efe_gate tables."""


# ---------------------------------------------------------------------------
# eval_runs
# ---------------------------------------------------------------------------


class EvalRun(Base):
    __tablename__ = "eval_runs"
    __table_args__ = (
        CheckConstraint(
            "state IN ('queued','running','measured','decided',"
            "'promoted','rejected','human_review','failed','rolled_back')",
            name="valid_eval_state",
        ),
        CheckConstraint(
            "decision IS NULL OR decision IN ("
            "'promote','reject','human_review','pending')",
            name="valid_eval_decision",
        ),
        UniqueConstraint(
            "tenant_id", "task_id", "candidate_run_id",
            name="uniq_eval_per_candidate",
        ),
        Index("ix_eval_tenant_state", "tenant_id", "state"),
        Index("ix_eval_candidate", "candidate_run_id"),
        Index("ix_eval_golden", "golden_set_id"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False, index=True)
    task_id: Mapped[str] = mapped_column(Text, nullable=False)
    candidate_run_id: Mapped[str] = mapped_column(UUIDStr, nullable=False)
    candidate_uri: Mapped[str] = mapped_column(Text, nullable=False)
    champion_run_id: Mapped[str | None] = mapped_column(UUIDStr, nullable=True)
    champion_artifact_uri: Mapped[str | None] = mapped_column(
        Text, nullable=True,
    )
    golden_set_id: Mapped[str] = mapped_column(
        UUIDStr, ForeignKey("golden_sets.id", ondelete="RESTRICT"),
        nullable=False,
    )
    state: Mapped[str] = mapped_column(
        String(16), nullable=False, default="queued", index=True,
    )
    state_version: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    decision: Mapped[str | None] = mapped_column(String(16), nullable=True)
    decision_reason: Mapped[str | None] = mapped_column(Text, nullable=True)
    submitted_by: Mapped[str] = mapped_column(Text, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
        default=_utcnow, onupdate=_utcnow,
    )
    started_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    finished_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    audit_chain_hash: Mapped[bytes | None] = mapped_column(
        LargeBinary(32), nullable=True,
    )
    tags: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )

    metrics: Mapped[list[EvalMetric]] = relationship(
        back_populates="run", cascade="all, delete-orphan",
    )

    def as_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "candidate_run_id": self.candidate_run_id,
            "candidate_uri": self.candidate_uri,
            "champion_run_id": self.champion_run_id,
            "champion_artifact_uri": self.champion_artifact_uri,
            "golden_set_id": self.golden_set_id,
            "state": self.state,
            "state_version": self.state_version,
            "decision": self.decision,
            "decision_reason": self.decision_reason,
            "submitted_by": self.submitted_by,
            "created_at": _iso(self.created_at),
            "updated_at": _iso(self.updated_at),
            "started_at": _iso(self.started_at),
            "finished_at": _iso(self.finished_at),
            "audit_chain_hash": (
                self.audit_chain_hash.hex() if self.audit_chain_hash else None
            ),
            "tags": dict(self.tags or {}),
        }


# ---------------------------------------------------------------------------
# eval_metrics
# ---------------------------------------------------------------------------


class EvalMetric(Base):
    __tablename__ = "eval_metrics"
    __table_args__ = (
        CheckConstraint(
            "role IN ('primary','secondary','budget','fairness')",
            name="valid_metric_role",
        ),
        UniqueConstraint(
            "eval_run_id", "metric_name", "group_key",
            name="uniq_metric_per_eval",
        ),
        Index("ix_metric_run", "eval_run_id"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    eval_run_id: Mapped[str] = mapped_column(
        UUIDStr, ForeignKey("eval_runs.id", ondelete="CASCADE"),
        nullable=False,
    )
    metric_name: Mapped[str] = mapped_column(String(64), nullable=False)
    role: Mapped[str] = mapped_column(String(16), nullable=False)
    group_key: Mapped[str] = mapped_column(
        String(64), nullable=False, default="__all__",
    )
    value: Mapped[float] = mapped_column(Float, nullable=False)
    ci_lower: Mapped[float | None] = mapped_column(Float, nullable=True)
    ci_upper: Mapped[float | None] = mapped_column(Float, nullable=True)
    sample_size: Mapped[int] = mapped_column(Integer, nullable=False)
    vs_champion_delta: Mapped[float | None] = mapped_column(
        Float, nullable=True,
    )
    p_value: Mapped[float | None] = mapped_column(Float, nullable=True)
    passed: Mapped[bool | None] = mapped_column(Boolean, nullable=True)
    computed_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )

    run: Mapped[EvalRun] = relationship(back_populates="metrics")


# ---------------------------------------------------------------------------
# golden_sets
# ---------------------------------------------------------------------------


class GoldenSet(Base):
    __tablename__ = "golden_sets"
    __table_args__ = (
        UniqueConstraint(
            "tenant_id", "task_id", "content_hash",
            name="uniq_golden_per_content",
        ),
        Index("ix_golden_tenant_task", "tenant_id", "task_id"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False)
    task_id: Mapped[str] = mapped_column(Text, nullable=False)
    snapshot_uri: Mapped[str] = mapped_column(Text, nullable=False)
    content_hash: Mapped[bytes] = mapped_column(
        LargeBinary(32), nullable=False,
    )
    n_samples: Mapped[int] = mapped_column(Integer, nullable=False)
    label_balance: Mapped[dict[str, int]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    fairness_attrs: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    locked_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    created_by: Mapped[str] = mapped_column(Text, nullable=False)
    deprecated: Mapped[bool] = mapped_column(
        Boolean, nullable=False, default=False,
    )


# ---------------------------------------------------------------------------
# champions
# ---------------------------------------------------------------------------


class Champion(Base):
    __tablename__ = "champions"
    __table_args__ = (
        Index("ix_champion_tenant", "tenant_id"),
    )

    tenant_id: Mapped[str] = mapped_column(Text, primary_key=True)
    task_id: Mapped[str] = mapped_column(Text, primary_key=True)
    artifact_uri: Mapped[str] = mapped_column(Text, nullable=False)
    run_id: Mapped[str] = mapped_column(UUIDStr, nullable=False)
    eval_run_id: Mapped[str] = mapped_column(
        UUIDStr, ForeignKey("eval_runs.id"), nullable=False,
    )
    promoted_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    promoted_by: Mapped[str] = mapped_column(Text, nullable=False)
    metrics_snapshot: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    version: Mapped[int] = mapped_column(Integer, nullable=False, default=1)


class ChampionHistory(Base):
    __tablename__ = "champion_history"
    __table_args__ = (
        CheckConstraint(
            "action IN ('promote','rollback')",
            name="valid_history_action",
        ),
        Index("ix_history_tenant_task", "tenant_id", "task_id"),
        Index("ix_history_at", "at"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False)
    task_id: Mapped[str] = mapped_column(Text, nullable=False)
    from_run_id: Mapped[str | None] = mapped_column(UUIDStr, nullable=True)
    to_run_id: Mapped[str] = mapped_column(UUIDStr, nullable=False)
    action: Mapped[str] = mapped_column(String(16), nullable=False)
    actor: Mapped[str] = mapped_column(Text, nullable=False)
    reason: Mapped[str] = mapped_column(Text, nullable=False, default="")
    at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    audit_hash: Mapped[bytes | None] = mapped_column(
        LargeBinary(32), nullable=True,
    )


# ---------------------------------------------------------------------------
# promotion_gates (rules)
# ---------------------------------------------------------------------------


class PromotionGate(Base):
    __tablename__ = "promotion_gates"

    tenant_id: Mapped[str] = mapped_column(Text, primary_key=True)
    task_id: Mapped[str] = mapped_column(Text, primary_key=True)
    rule: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    updated_by: Mapped[str] = mapped_column(Text, nullable=False)
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
        default=_utcnow, onupdate=_utcnow,
    )


# ---------------------------------------------------------------------------
# rollback_monitors
# ---------------------------------------------------------------------------


class RollbackMonitor(Base):
    __tablename__ = "rollback_monitors"
    __table_args__ = (
        CheckConstraint(
            "state IN ('watching','rolled_back','expired')",
            name="valid_rb_state",
        ),
        Index("ix_rb_state", "state"),
        Index("ix_rb_deadline", "deadline"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False)
    task_id: Mapped[str] = mapped_column(Text, nullable=False)
    run_id: Mapped[str] = mapped_column(UUIDStr, nullable=False)
    eval_run_id: Mapped[str] = mapped_column(
        UUIDStr, ForeignKey("eval_runs.id"), nullable=False,
    )
    started_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    deadline: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    state: Mapped[str] = mapped_column(
        String(16), nullable=False, default="watching",
    )
    observed_metrics: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    last_tick_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    n_ticks: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    final_reason: Mapped[str | None] = mapped_column(Text, nullable=True)


# ---------------------------------------------------------------------------
# review_requests
# ---------------------------------------------------------------------------


class ReviewRequest(Base):
    __tablename__ = "review_requests"
    __table_args__ = (
        CheckConstraint(
            "status IN ('pending','approved','rejected','timed_out')",
            name="valid_review_status",
        ),
        UniqueConstraint("eval_run_id", name="uniq_review_per_eval"),
        Index("ix_review_status", "status"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    eval_run_id: Mapped[str] = mapped_column(
        UUIDStr, ForeignKey("eval_runs.id", ondelete="CASCADE"),
        nullable=False,
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False)
    task_id: Mapped[str] = mapped_column(Text, nullable=False)
    reason: Mapped[str] = mapped_column(Text, nullable=False, default="")
    requires_dual_signoff: Mapped[bool] = mapped_column(
        Boolean, nullable=False, default=True,
    )
    signatures: Mapped[list[dict[str, Any]]] = mapped_column(
        JsonType, nullable=False, default=list,
    )
    status: Mapped[str] = mapped_column(
        String(16), nullable=False, default="pending",
    )
    deadline: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    resolved_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )


# ---------------------------------------------------------------------------
# audit_events (signed chain)
# ---------------------------------------------------------------------------


class AuditEvent(Base):
    __tablename__ = "audit_events"
    __table_args__ = (
        # ``seq`` is the *authoritative* per-tenant chain order.
        # ``created_at`` alone is not monotonic enough (two writes in
        # the same microsecond race on the UUID tiebreak), which
        # breaks :func:`verify_chain` replay. ``UNIQUE(tenant_id, seq)``
        # is enforced so concurrent writers error-loudly rather than
        # silently corrupting the chain.
        UniqueConstraint(
            "tenant_id", "seq", name="uniq_audit_seq_per_tenant",
        ),
        Index("ix_audit_tenant_seq", "tenant_id", "seq"),
        Index("ix_audit_tenant_created", "tenant_id", "created_at"),
        Index("ix_audit_subject", "subject_id"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False)
    seq: Mapped[int] = mapped_column(Integer, nullable=False)
    kind: Mapped[str] = mapped_column(String(64), nullable=False)
    actor: Mapped[str] = mapped_column(Text, nullable=False)
    subject_id: Mapped[str | None] = mapped_column(UUIDStr, nullable=True)
    payload: Mapped[dict[str, Any]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    prev_hash: Mapped[bytes | None] = mapped_column(
        LargeBinary(32), nullable=True,
    )
    this_hash: Mapped[bytes] = mapped_column(
        LargeBinary(32), nullable=False,
    )
    signature: Mapped[bytes | None] = mapped_column(
        LargeBinary(64), nullable=True,
    )
    signing_key_id: Mapped[str | None] = mapped_column(
        String(32), nullable=True,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _iso(dt: datetime | None) -> str | None:
    return dt.isoformat() if dt is not None else None


__all__ = [
    "AuditEvent",
    "Base",
    "Champion",
    "ChampionHistory",
    "EvalMetric",
    "EvalRun",
    "GoldenSet",
    "JsonType",
    "PromotionGate",
    "ReviewRequest",
    "RollbackMonitor",
    "UUIDStr",
]

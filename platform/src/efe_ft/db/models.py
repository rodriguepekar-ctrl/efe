"""SQLAlchemy 2.0 declarative mapped models for the 6 M01 tables.

These are the **source of truth** for the database schema. Alembic
migrations are generated from them. Field-level comments document the
invariants; table-level constraints mirror the DDL in the architecture
document.

Type hints use SQLAlchemy 2.0 ``Mapped[...]`` annotations — strict
typing all the way down.
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
    ForeignKey,
    Index,
    Integer,
    LargeBinary,
    String,
    Text,
    UniqueConstraint,
)
from sqlalchemy.dialects.postgresql import ARRAY as PG_ARRAY
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

    Python side always exposes ``str`` so downstream code does not
    have to care about the underlying dialect.
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
    """JSONB on Postgres, plain JSON on SQLite.

    Original name ``JSONB_or_JSON`` violated CapWords — renamed, a
    compat alias is re-exported below.
    """

    impl = JSON
    cache_ok = True

    def load_dialect_impl(self, dialect: Any) -> Any:
        if dialect.name == "postgresql":
            return dialect.type_descriptor(JSONB())
        return dialect.type_descriptor(JSON())


class StrArray(TypeDecorator[list[str]]):
    """``TEXT[]`` on Postgres, semicolon-joined TEXT on SQLite.

    SQLite doesn't support real arrays; we serialise as ``"a;b;c"``.
    Empty list = empty string. Separator ``;`` is forbidden in values
    (we assert this at insertion in application code).
    """

    impl = Text
    cache_ok = True

    def load_dialect_impl(self, dialect: Any) -> Any:
        if dialect.name == "postgresql":
            return dialect.type_descriptor(PG_ARRAY(Text()))
        return dialect.type_descriptor(Text())

    def process_bind_param(self, value: Any, dialect: Any) -> Any:
        if value is None:
            return None
        if dialect.name == "postgresql":
            return list(value)
        return ";".join(value)

    def process_result_value(self, value: Any, dialect: Any) -> Any:
        if value is None:
            return []
        if isinstance(value, list):
            return value
        return [x for x in value.split(";") if x]


# ---------------------------------------------------------------------------
#  Base
# ---------------------------------------------------------------------------


class Base(DeclarativeBase):
    """Declarative base for all ORM models in efe-ft."""


# ---------------------------------------------------------------------------
#  training_samples — central entity
# ---------------------------------------------------------------------------


class TrainingSample(Base):
    __tablename__ = "training_samples"
    __table_args__ = (
        UniqueConstraint("tenant_id", "task_id", "text_sha256",
                         name="uniq_sample_per_text"),
        CheckConstraint("source IN ('batch','webhook','labeling')",
                        name="valid_source"),
        CheckConstraint(
            "status IN ('pending','labelled','rejected','archived')",
            name="valid_status",
        ),
        Index("ix_samples_tenant_task_created",
              "tenant_id", "task_id", "created_at"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False, index=True)
    task_id: Mapped[str] = mapped_column(Text, nullable=False)
    text: Mapped[str] = mapped_column(Text, nullable=False)
    text_sha256: Mapped[bytes] = mapped_column(
        LargeBinary(32), nullable=False, index=True,
    )
    language: Mapped[str] = mapped_column(
        String(8), nullable=False, default="fr",
    )
    label: Mapped[str | None] = mapped_column(Text, nullable=True)
    label_confidence: Mapped[float | None] = mapped_column(nullable=True)
    source: Mapped[str] = mapped_column(String(16), nullable=False)
    origin_ref: Mapped[str | None] = mapped_column(Text, nullable=True)
    pii_flags: Mapped[list[str]] = mapped_column(
        StrArray, nullable=False, default=list,
    )
    embedding_sha256: Mapped[bytes | None] = mapped_column(
        LargeBinary(32), nullable=True,
    )
    status: Mapped[str] = mapped_column(
        String(16), nullable=False, default="pending",
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
        default=_utcnow, onupdate=_utcnow,
    )

    # Relationships
    history: Mapped[list[LabelHistory]] = relationship(
        back_populates="sample", cascade="all, delete-orphan",
    )

    def as_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "text": self.text,
            "language": self.language,
            "label": self.label,
            "label_confidence": self.label_confidence,
            "source": self.source,
            "origin_ref": self.origin_ref,
            "pii_flags": list(self.pii_flags or []),
            "status": self.status,
            "created_at": self.created_at.isoformat() if self.created_at else None,
        }


# ---------------------------------------------------------------------------
#  snapshots + snapshot_members — immutable materialisations
# ---------------------------------------------------------------------------


class Snapshot(Base):
    __tablename__ = "snapshots"
    __table_args__ = (
        UniqueConstraint(
            "tenant_id", "task_id", "content_hash",
            name="uniq_content_per_task",
        ),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False, index=True)
    task_id: Mapped[str] = mapped_column(Text, nullable=False)
    sample_ids_hash: Mapped[bytes] = mapped_column(
        LargeBinary(32), nullable=False,
    )
    content_hash: Mapped[bytes] = mapped_column(
        LargeBinary(32), nullable=False, index=True,
    )
    n_samples: Mapped[int] = mapped_column(Integer, nullable=False)
    label_balance: Mapped[dict[str, int]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    holdout_ids_hash: Mapped[bytes | None] = mapped_column(
        LargeBinary(32), nullable=True,
    )
    s3_payload_uri: Mapped[str | None] = mapped_column(Text, nullable=True)
    created_by: Mapped[str] = mapped_column(Text, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )

    members: Mapped[list[SnapshotMember]] = relationship(
        back_populates="snapshot", cascade="all, delete-orphan",
    )


class SnapshotMember(Base):
    __tablename__ = "snapshot_members"
    __table_args__ = (
        CheckConstraint(
            "partition IN ('train','val','holdout')",
            name="valid_partition",
        ),
        Index("ix_snapmember_sample", "sample_id"),
    )

    snapshot_id: Mapped[str] = mapped_column(
        UUIDStr, ForeignKey("snapshots.id", ondelete="CASCADE"),
        primary_key=True,
    )
    sample_id: Mapped[str] = mapped_column(
        UUIDStr, ForeignKey("training_samples.id", ondelete="RESTRICT"),
        primary_key=True,
    )
    partition: Mapped[str] = mapped_column(String(10), nullable=False)

    snapshot: Mapped[Snapshot] = relationship(back_populates="members")


# ---------------------------------------------------------------------------
#  embedding_cache
# ---------------------------------------------------------------------------


class EmbeddingCache(Base):
    __tablename__ = "embedding_cache"
    __table_args__ = (
        Index("ix_embcache_last_used", "last_used_at"),
    )

    text_sha256: Mapped[bytes] = mapped_column(
        LargeBinary(32), primary_key=True,
    )
    embedder_version: Mapped[str] = mapped_column(
        String(64), primary_key=True,
    )
    dim: Mapped[int] = mapped_column(Integer, nullable=False)
    s3_uri: Mapped[str] = mapped_column(Text, nullable=False)
    bytes_size: Mapped[int] = mapped_column(
        "bytes", Integer, nullable=False,
    )
    checksum: Mapped[bytes] = mapped_column(
        LargeBinary(32), nullable=False,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )
    last_used_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )


# ---------------------------------------------------------------------------
#  label_history — append-only
# ---------------------------------------------------------------------------


class LabelHistory(Base):
    __tablename__ = "label_history"
    __table_args__ = (
        Index("ix_history_sample", "sample_id"),
        Index("ix_history_annotator_ts", "annotator_id", "created_at"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    sample_id: Mapped[str] = mapped_column(
        UUIDStr, ForeignKey("training_samples.id", ondelete="CASCADE"),
        nullable=False,
    )
    annotator_id: Mapped[str] = mapped_column(Text, nullable=False)
    label_value: Mapped[str] = mapped_column(Text, nullable=False)
    confidence: Mapped[float] = mapped_column(nullable=False, default=1.0)
    round: Mapped[int] = mapped_column(Integer, nullable=False, default=1)
    is_expert: Mapped[bool] = mapped_column(
        Boolean, nullable=False, default=False,
    )
    time_ms: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )

    sample: Mapped[TrainingSample] = relationship(back_populates="history")


# ---------------------------------------------------------------------------
#  audit_events — tamper-evident chain
# ---------------------------------------------------------------------------


class AuditEvent(Base):
    __tablename__ = "audit_events"
    __table_args__ = (
        Index("ix_audit_tenant_created", "tenant_id", "created_at"),
        Index("ix_audit_subject", "subject_id"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False)
    kind: Mapped[str] = mapped_column(String(64), nullable=False)
    actor_id: Mapped[str] = mapped_column(Text, nullable=False)
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
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )


# ---------------------------------------------------------------------------
#  ingest_batches
# ---------------------------------------------------------------------------


class IngestBatch(Base):
    __tablename__ = "ingest_batches"
    __table_args__ = (
        Index("ix_batches_tenant_created", "tenant_id", "created_at"),
        UniqueConstraint("tenant_id", "sha256", name="uniq_batch_sha"),
    )

    id: Mapped[str] = mapped_column(
        UUIDStr, primary_key=True, default=lambda: str(uuid.uuid4()),
    )
    tenant_id: Mapped[str] = mapped_column(Text, nullable=False)
    task_id: Mapped[str] = mapped_column(Text, nullable=False)
    source: Mapped[str] = mapped_column(String(16), nullable=False)
    sha256: Mapped[bytes] = mapped_column(LargeBinary(32), nullable=False)
    n_received: Mapped[int] = mapped_column(Integer, nullable=False)
    n_accepted: Mapped[int] = mapped_column(Integer, nullable=False)
    n_rejected: Mapped[int] = mapped_column(Integer, nullable=False)
    rejections: Mapped[dict[str, int]] = mapped_column(
        JsonType, nullable=False, default=dict,
    )
    actor_id: Mapped[str] = mapped_column(Text, nullable=False)
    latency_ms: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=0,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_utcnow,
    )


__all__ = [
    "AuditEvent",
    "Base",
    "EmbeddingCache",
    "IngestBatch",
    "JsonType",
    "LabelHistory",
    "Snapshot",
    "SnapshotMember",
    "StrArray",
    "TrainingSample",
    "UUIDStr",
]

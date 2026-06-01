"""SQLAlchemy 2.0 async ORM models for :mod:`efe_drift`.

Schema overview (7 tables — covers L1 Catalog/Ingestion + L2 Aggregation +
L3 Baseline) :

* ``feature_catalog``              — registered features (versioned)
* ``baselines``                    — promoted reference snapshots
* ``baseline_state_transitions``   — audit journal for FSM moves
* ``feature_snapshots``            — rolled-up histograms per bucket
* ``raw_events``                   — ingestion hot path (hypertable in
                                      prod, plain table on SQLite for
                                      unit tests)
* ``drift_evaluations``            — output of the L4 detectors (stub
                                      here — schema only for
                                      forward-compat)
* ``alert_incidents``              — idem (stub)

Portability notes :

* ``JSONB``  → ``JSON`` on SQLite so tests run without Postgres.
* ``BYTEA``  → ``LargeBinary``.
* Partitioning + hypertable DDL is omitted at declarative level;
  Alembic migrations re-attach it on Postgres.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any

from sqlalchemy import (
    JSON,
    BigInteger,
    Boolean,
    CheckConstraint,
    DateTime,
    Double,
    ForeignKey,
    Index,
    Integer,
    LargeBinary,
    String,
    Text,
    UniqueConstraint,
    func,
)
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


class Base(DeclarativeBase):
    """Declarative root for all efe_drift ORM models."""


# ---------------------------------------------------------------------------
# feature_catalog
# ---------------------------------------------------------------------------


class FeatureCatalogRow(Base):
    __tablename__ = "feature_catalog"
    __table_args__ = (
        UniqueConstraint(
            "tenant_id", "task_id", "name", "version",
            name="uq_feature_catalog_scope",
        ),
        Index(
            "ix_feature_catalog_tenant_task",
            "tenant_id", "task_id",
        ),
        CheckConstraint(
            "kind IN ('numeric','categorical','embedding','binary')",
            name="ck_feature_kind",
        ),
        CheckConstraint(
            "binning_strategy IN ('equifrequency','equiwidth','custom','none')",
            name="ck_binning_strategy",
        ),
        CheckConstraint(
            "retention_days > 0",
            name="ck_retention_days_pos",
        ),
    )

    feature_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    name: Mapped[str] = mapped_column(String(128), nullable=False)
    version: Mapped[int] = mapped_column(
        Integer, nullable=False, default=1, server_default="1",
    )
    kind: Mapped[str] = mapped_column(String(16), nullable=False)
    extractor: Mapped[str] = mapped_column(String(128), nullable=False)
    binning_strategy: Mapped[str] = mapped_column(String(16), nullable=False)
    expected_cardinality: Mapped[int | None] = mapped_column(
        Integer, nullable=True,
    )
    retention_days: Mapped[int] = mapped_column(
        Integer, nullable=False, default=90, server_default="90",
    )
    enabled: Mapped[bool] = mapped_column(
        Boolean, nullable=False, default=True, server_default="true",
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    created_by: Mapped[str] = mapped_column(
        String(256), nullable=False, default="system",
        server_default="system",
    )
    notes: Mapped[str | None] = mapped_column(Text, nullable=True)


# ---------------------------------------------------------------------------
# baselines
# ---------------------------------------------------------------------------


class BaselineRow(Base):
    __tablename__ = "baselines"
    __table_args__ = (
        CheckConstraint(
            "state IN ('planned','accumulating','candidate',"
            "'active','retiring','retired','rejected')",
            name="ck_baseline_state",
        ),
        CheckConstraint(
            "overlap_days >= 1 AND overlap_days <= 14",
            name="ck_overlap_days",
        ),
        Index("ix_baselines_state_tenant", "state", "tenant_id"),
        Index("ix_baselines_feature", "feature_id"),
    )

    baseline_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    feature_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("feature_catalog.feature_id"),
        nullable=False,
    )
    model_artifact_id: Mapped[str | None] = mapped_column(
        String(256), nullable=True,
    )
    state: Mapped[str] = mapped_column(String(16), nullable=False)
    state_version: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=0, server_default="0",
    )
    window_start: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    window_end: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    sample_count: Mapped[int | None] = mapped_column(
        BigInteger, nullable=True,
    )
    bins_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    categorical_categories_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    centroid_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    std_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    stats_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    spec_hash: Mapped[bytes | None] = mapped_column(
        LargeBinary, nullable=True,
    )
    signing_key_id: Mapped[str | None] = mapped_column(
        String(256), nullable=True,
    )
    signature: Mapped[bytes | None] = mapped_column(
        LargeBinary, nullable=True,
    )
    planned_end_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    overlap_days: Mapped[int] = mapped_column(
        Integer, nullable=False, default=3, server_default="3",
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    promoted_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    retired_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    notes: Mapped[str | None] = mapped_column(Text, nullable=True)


# ---------------------------------------------------------------------------
# baseline_state_transitions
# ---------------------------------------------------------------------------


class BaselineStateTransitionRow(Base):
    __tablename__ = "baseline_state_transitions"
    __table_args__ = (
        Index(
            "ix_baseline_trans_baseline_time",
            "baseline_id", "occurred_at",
        ),
    )

    transition_id: Mapped[str] = mapped_column(
        String(36), primary_key=True,
    )
    baseline_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("baselines.baseline_id", ondelete="CASCADE"),
        nullable=False,
    )
    from_state: Mapped[str] = mapped_column(String(16), nullable=False)
    to_state: Mapped[str] = mapped_column(String(16), nullable=False)
    actor: Mapped[str] = mapped_column(String(256), nullable=False)
    reason: Mapped[str | None] = mapped_column(Text, nullable=True)
    occurred_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


# ---------------------------------------------------------------------------
# feature_snapshots — aggregated histograms per bucket
# ---------------------------------------------------------------------------


class FeatureSnapshotRow(Base):
    __tablename__ = "feature_snapshots"
    __table_args__ = (
        CheckConstraint(
            "bucket_kind IN ('minute','hour','day','week')",
            name="ck_bucket_kind",
        ),
        CheckConstraint(
            "sample_count >= 0",
            name="ck_snapshot_sample_count_nn",
        ),
        Index(
            "ix_snapshots_feat_bucket_time",
            "feature_id", "bucket_kind", "bucket_start",
        ),
        Index(
            "ix_snapshots_tenant_task_time",
            "tenant_id", "task_id", "bucket_start",
        ),
    )

    snapshot_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    feature_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("feature_catalog.feature_id"),
        nullable=False,
    )
    bucket_kind: Mapped[str] = mapped_column(String(8), nullable=False)
    bucket_start: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    bucket_end: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    sample_count: Mapped[int] = mapped_column(BigInteger, nullable=False)
    histogram_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    stats_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    categorical_counts_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    embedding_centroid_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    embedding_std_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    embedding_ref: Mapped[str | None] = mapped_column(Text, nullable=True)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


# ---------------------------------------------------------------------------
# raw_events — hot path ingestion storage
# ---------------------------------------------------------------------------


class RawEventRow(Base):
    __tablename__ = "raw_events"
    __table_args__ = (
        Index(
            "ix_raw_events_feat_time",
            "feature_id", "observed_at",
        ),
        Index(
            "ix_raw_events_tenant_time",
            "tenant_id", "observed_at",
        ),
    )

    event_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    feature_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("feature_catalog.feature_id"),
        nullable=False,
    )
    observed_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    value_num: Mapped[float | None] = mapped_column(Double, nullable=True)
    value_cat: Mapped[str | None] = mapped_column(
        String(256), nullable=True,
    )
    value_emb: Mapped[bytes | None] = mapped_column(
        LargeBinary, nullable=True,
    )
    user_id_hash: Mapped[str | None] = mapped_column(
        String(64), nullable=True,
    )
    request_id: Mapped[str | None] = mapped_column(
        String(128), nullable=True,
    )


# ---------------------------------------------------------------------------
# drift_evaluations — stub for L4, schema-only forward-compat
# ---------------------------------------------------------------------------


class DriftEvaluationRow(Base):
    __tablename__ = "drift_evaluations"
    __table_args__ = (
        CheckConstraint(
            "method IN ('psi','ks','wasserstein','chi_squared',"
            "'js_divergence','mmd','classifier_based')",
            name="ck_evaluation_method",
        ),
        CheckConstraint(
            "severity IN ('none','warning','major','critical')",
            name="ck_evaluation_severity",
        ),
        Index(
            "ix_evals_tenant_time",
            "tenant_id", "computed_at",
        ),
    )

    evaluation_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    feature_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("feature_catalog.feature_id"),
        nullable=False,
    )
    baseline_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("baselines.baseline_id"),
        nullable=False,
    )
    window_start: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    window_end: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    method: Mapped[str] = mapped_column(String(32), nullable=False)
    score: Mapped[float] = mapped_column(Double, nullable=False)
    p_value: Mapped[float | None] = mapped_column(Double, nullable=True)
    severity: Mapped[str] = mapped_column(String(16), nullable=False)
    details_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    computed_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


# ---------------------------------------------------------------------------
# alert_incidents — stub for L5, schema-only forward-compat
# ---------------------------------------------------------------------------


class AlertIncidentRow(Base):
    __tablename__ = "alert_incidents"
    __table_args__ = (
        CheckConstraint(
            "state IN ('firing','acked','resolved')",
            name="ck_incident_state",
        ),
        Index("ix_incidents_state", "state", "fired_at"),
    )

    incident_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    rule_id: Mapped[str] = mapped_column(String(36), nullable=False)
    baseline_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("baselines.baseline_id"),
        nullable=False,
    )
    feature_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("feature_catalog.feature_id"),
        nullable=False,
    )
    state: Mapped[str] = mapped_column(String(16), nullable=False)
    severity: Mapped[str] = mapped_column(String(16), nullable=False)
    score_at_fire: Mapped[float] = mapped_column(Double, nullable=False)
    fired_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    acked_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    acked_by: Mapped[str | None] = mapped_column(
        String(256), nullable=True,
    )
    resolved_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    dispatch_results_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )


__all__ = [
    "AlertIncidentRow",
    "Base",
    "BaselineRow",
    "BaselineStateTransitionRow",
    "DriftEvaluationRow",
    "FeatureCatalogRow",
    "FeatureSnapshotRow",
    "RawEventRow",
]

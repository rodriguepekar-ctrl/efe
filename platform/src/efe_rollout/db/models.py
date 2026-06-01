"""SQLAlchemy 2.0 async ORM models for :mod:`efe_rollout`.

Schema overview (10 tables, matches M08 §6.1 DDL):

* ``experiments``                  — spec + FSM state + state_version
* ``variants``                     — one arm per experiment
* ``experiment_preregistrations``  — HMAC-sealed plan
* ``exposures``                    — audit trail, partition-by-day in prod
* ``outcomes``                     — metric observations, partition-by-day
* ``analyses``                     — computed stats results
* ``bandit_state``                 — Beta(α,β) per arm
* ``guardrails``                   — per-experiment guardrail definitions
* ``forced_assignments``           — QA / founders overrides
* ``layer_allocations``            — ≤ 1 active exp per (tenant, task, layer)

Portability notes:

* ``JSONB`` → ``JSON`` on SQLite so unit tests run without Postgres.
* ``BYTEA`` → ``LargeBinary``.
* ``PARTITION BY RANGE`` is omitted in the declarative definition (it
  is a DDL-only concern); Alembic migrations re-attach partitioning on
  Postgres.
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
    SmallInteger,
    String,
    Text,
    UniqueConstraint,
    func,
)
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship


class Base(DeclarativeBase):
    """Declarative root for all efe_rollout ORM models."""


# ---------------------------------------------------------------------------
# experiments
# ---------------------------------------------------------------------------


class ExperimentRow(Base):
    __tablename__ = "experiments"
    __table_args__ = (
        UniqueConstraint(
            "tenant_id", "name", name="uq_experiments_tenant_name",
        ),
        Index("ix_experiments_tenant_state", "tenant_id", "state"),
        Index("ix_experiments_layer_state", "layer", "state"),
        CheckConstraint(
            "state IN ('draft','designed','registered','running','paused',"
            "'aborted','analyzed','shipped','rollback',"
            "'inconclusive','canceled')",
            name="ck_experiment_state",
        ),
        CheckConstraint(
            "kind IN ('ab_fixed','ab_sequential','bandit','holdout')",
            name="ck_experiment_kind",
        ),
        CheckConstraint(
            "stopping_rule IN ('fixed_horizon','pocock','obf')",
            name="ck_experiment_stopping_rule",
        ),
        CheckConstraint(
            "alpha > 0 AND alpha < 0.5",
            name="ck_experiment_alpha",
        ),
        CheckConstraint(
            "power_target > 0.5 AND power_target < 1",
            name="ck_experiment_power",
        ),
    )

    experiment_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    name: Mapped[str] = mapped_column(String(256), nullable=False)
    layer: Mapped[str] = mapped_column(String(64), nullable=False)
    kind: Mapped[str] = mapped_column(String(32), nullable=False)
    state: Mapped[str] = mapped_column(String(32), nullable=False)
    state_version: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=0, server_default="0",
    )
    owner: Mapped[str] = mapped_column(String(256), nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    designed_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    registered_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    started_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    ended_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    planned_end_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    planned_sample_size: Mapped[int | None] = mapped_column(
        Integer, nullable=True,
    )
    hypothesis: Mapped[str] = mapped_column(Text, nullable=False)
    primary_metric: Mapped[str] = mapped_column(String(128), nullable=False)
    alpha: Mapped[float] = mapped_column(
        Double, nullable=False, default=0.05, server_default="0.05",
    )
    power_target: Mapped[float] = mapped_column(
        Double, nullable=False, default=0.8, server_default="0.8",
    )
    minimum_detectable_effect: Mapped[float] = mapped_column(
        Double, nullable=False, default=0.0, server_default="0.0",
    )
    stopping_rule: Mapped[str] = mapped_column(
        String(32), nullable=False, default="fixed_horizon",
        server_default="fixed_horizon",
    )
    traffic_allocation_bps: Mapped[int] = mapped_column(
        Integer, nullable=False, default=10_000, server_default="10000",
    )
    spec_hash: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)

    variants: Mapped[list[VariantRow]] = relationship(
        back_populates="experiment",
        cascade="all, delete-orphan",
    )


# ---------------------------------------------------------------------------
# variants
# ---------------------------------------------------------------------------


class VariantRow(Base):
    __tablename__ = "variants"
    __table_args__ = (
        UniqueConstraint(
            "experiment_id", "name", name="uq_variants_exp_name",
        ),
        Index("ix_variants_experiment", "experiment_id"),
        CheckConstraint(
            "allocation_pct >= 0 AND allocation_pct <= 100",
            name="ck_variant_alloc",
        ),
    )

    variant_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    experiment_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("experiments.experiment_id", ondelete="CASCADE"),
        nullable=False,
    )
    name: Mapped[str] = mapped_column(String(128), nullable=False)
    is_control: Mapped[bool] = mapped_column(
        Boolean, nullable=False, default=False, server_default="false",
    )
    allocation_pct: Mapped[int] = mapped_column(SmallInteger, nullable=False)
    payload_jsonb: Mapped[dict[str, Any]] = mapped_column(JSON, nullable=False)

    experiment: Mapped[ExperimentRow] = relationship(back_populates="variants")


# ---------------------------------------------------------------------------
# experiment_preregistrations
# ---------------------------------------------------------------------------


class PreRegistrationRow(Base):
    __tablename__ = "experiment_preregistrations"
    __table_args__ = (
        UniqueConstraint(
            "experiment_id", name="uq_prereg_experiment",
        ),
    )

    prereg_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    experiment_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("experiments.experiment_id", ondelete="CASCADE"),
        nullable=False,
    )
    spec_hash: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)
    signing_key_id: Mapped[str] = mapped_column(String(128), nullable=False)
    signature: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)
    plan_json: Mapped[str] = mapped_column(Text, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


# ---------------------------------------------------------------------------
# exposures
# ---------------------------------------------------------------------------


class ExposureRow(Base):
    __tablename__ = "exposures"
    __table_args__ = (
        Index("ix_exposures_exp_time", "experiment_id", "assigned_at"),
        Index("ix_exposures_exp_user", "experiment_id", "user_id"),
        Index("ix_exposures_tenant_time", "tenant_id", "assigned_at"),
    )

    exposure_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    experiment_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("experiments.experiment_id"),
        nullable=False,
    )
    variant_id: Mapped[str | None] = mapped_column(
        String(36),
        ForeignKey("variants.variant_id"),
        nullable=True,
    )
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    user_id: Mapped[str] = mapped_column(String(256), nullable=False)
    assigned_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    layer: Mapped[str] = mapped_column(String(64), nullable=False)
    forced: Mapped[bool] = mapped_column(
        Boolean, nullable=False, default=False, server_default="false",
    )
    holdout: Mapped[bool] = mapped_column(
        Boolean, nullable=False, default=False, server_default="false",
    )
    request_id: Mapped[str | None] = mapped_column(String(128), nullable=True)
    segment_hash: Mapped[bytes | None] = mapped_column(
        LargeBinary, nullable=True,
    )
    hmac_sig: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)


# ---------------------------------------------------------------------------
# outcomes
# ---------------------------------------------------------------------------


class OutcomeRow(Base):
    __tablename__ = "outcomes"
    __table_args__ = (
        Index(
            "ix_outcomes_exp_metric_time",
            "experiment_id", "metric_name", "observed_at",
        ),
        Index("ix_outcomes_user", "experiment_id", "user_id"),
    )

    outcome_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    experiment_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("experiments.experiment_id"),
        nullable=False,
    )
    variant_id: Mapped[str | None] = mapped_column(
        String(36),
        ForeignKey("variants.variant_id"),
        nullable=True,
    )
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    user_id: Mapped[str] = mapped_column(String(256), nullable=False)
    metric_name: Mapped[str] = mapped_column(String(128), nullable=False)
    metric_value: Mapped[float] = mapped_column(Double, nullable=False)
    observed_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    request_id: Mapped[str | None] = mapped_column(String(128), nullable=True)
    covariate_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )


# ---------------------------------------------------------------------------
# analyses
# ---------------------------------------------------------------------------


class AnalysisRow(Base):
    __tablename__ = "analyses"
    __table_args__ = (
        Index("ix_analyses_exp_time", "experiment_id", "computed_at"),
        CheckConstraint(
            "kind IN ('interim','final','post_hoc','segment','cuped')",
            name="ck_analysis_kind",
        ),
    )

    analysis_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    experiment_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("experiments.experiment_id"),
        nullable=False,
    )
    kind: Mapped[str] = mapped_column(String(16), nullable=False)
    computed_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    primary_metric: Mapped[str] = mapped_column(String(128), nullable=False)
    n_per_variant: Mapped[dict[str, Any]] = mapped_column(JSON, nullable=False)
    estimate_json: Mapped[dict[str, Any]] = mapped_column(JSON, nullable=False)
    ci_95_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    p_value: Mapped[float | None] = mapped_column(Double, nullable=True)
    posterior_prob: Mapped[float | None] = mapped_column(Double, nullable=True)
    decision: Mapped[str | None] = mapped_column(String(32), nullable=True)
    methods_json: Mapped[dict[str, Any]] = mapped_column(JSON, nullable=False)
    segments_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )


# ---------------------------------------------------------------------------
# bandit_state
# ---------------------------------------------------------------------------


class BanditStateRow(Base):
    __tablename__ = "bandit_state"
    __table_args__ = (
        CheckConstraint("alpha > 0", name="ck_bandit_alpha_pos"),
        CheckConstraint("beta > 0", name="ck_bandit_beta_pos"),
        CheckConstraint("n_pulls >= 0", name="ck_bandit_pulls_nn"),
        CheckConstraint("n_successes >= 0", name="ck_bandit_successes_nn"),
    )

    experiment_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("experiments.experiment_id"),
        primary_key=True,
    )
    variant_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("variants.variant_id"),
        primary_key=True,
    )
    alpha: Mapped[float] = mapped_column(
        Double, nullable=False, default=1.0, server_default="1.0",
    )
    beta: Mapped[float] = mapped_column(
        Double, nullable=False, default=1.0, server_default="1.0",
    )
    n_pulls: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=0, server_default="0",
    )
    n_successes: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=0, server_default="0",
    )
    last_updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


# ---------------------------------------------------------------------------
# guardrails
# ---------------------------------------------------------------------------


class GuardrailRow(Base):
    __tablename__ = "guardrails"
    __table_args__ = (
        Index(
            "ix_guardrails_exp_metric", "experiment_id", "metric_name",
        ),
        CheckConstraint(
            "direction IN ('up','down')", name="ck_guardrail_direction",
        ),
        CheckConstraint(
            "threshold_pct > 0", name="ck_guardrail_threshold_pos",
        ),
        CheckConstraint(
            "consecutive_breaches > 0",
            name="ck_guardrail_consec_pos",
        ),
    )

    guardrail_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    experiment_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("experiments.experiment_id", ondelete="CASCADE"),
        nullable=False,
    )
    metric_name: Mapped[str] = mapped_column(String(128), nullable=False)
    direction: Mapped[str] = mapped_column(String(4), nullable=False)
    threshold_pct: Mapped[float] = mapped_column(Double, nullable=False)
    window_s: Mapped[int] = mapped_column(
        Integer, nullable=False, default=300, server_default="300",
    )
    consecutive_breaches: Mapped[int] = mapped_column(
        Integer, nullable=False, default=3, server_default="3",
    )
    breaches_so_far: Mapped[int] = mapped_column(
        Integer, nullable=False, default=0, server_default="0",
    )
    last_check_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    last_breach_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )


# ---------------------------------------------------------------------------
# forced_assignments
# ---------------------------------------------------------------------------


class ForcedAssignmentRow(Base):
    __tablename__ = "forced_assignments"

    experiment_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("experiments.experiment_id", ondelete="CASCADE"),
        primary_key=True,
    )
    user_id: Mapped[str] = mapped_column(String(256), primary_key=True)
    variant_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("variants.variant_id"),
        nullable=False,
    )
    reason: Mapped[str] = mapped_column(Text, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    created_by: Mapped[str] = mapped_column(String(256), nullable=False)


# ---------------------------------------------------------------------------
# layer_allocations
# ---------------------------------------------------------------------------


class LayerAllocationRow(Base):
    __tablename__ = "layer_allocations"
    __table_args__ = (
        Index(
            "ix_layer_alloc_slot",
            "tenant_id", "task_id", "layer",
        ),
    )

    tenant_id: Mapped[str] = mapped_column(String(128), primary_key=True)
    task_id: Mapped[str] = mapped_column(String(128), primary_key=True)
    layer: Mapped[str] = mapped_column(String(64), primary_key=True)
    experiment_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("experiments.experiment_id"),
        primary_key=True,
    )
    started_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    ended_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )


__all__ = [
    "AnalysisRow",
    "BanditStateRow",
    "Base",
    "ExperimentRow",
    "ExposureRow",
    "ForcedAssignmentRow",
    "GuardrailRow",
    "LayerAllocationRow",
    "OutcomeRow",
    "PreRegistrationRow",
    "VariantRow",
]

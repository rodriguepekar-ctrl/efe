"""SQLAlchemy 2.0 async ORM models for efe_registry.

Schema overview (12 tables):

* ``artifacts``                  — canonical artifact records
* ``artifact_signatures``        — cosign signatures (multi per artifact)
* ``sboms``                      — CycloneDX SBOM attestations
* ``slsa_provenance``            — SLSA in-toto provenance
* ``endpoints``                  — live pods serving artifacts
* ``routing_table``              — durable copy; Redis is hot path
* ``canary_runs``                — canary progression state
* ``shadow_taps``                — shadow diff aggregates
* ``rollback_events``            — chronology of rollbacks (SLO proof)
* ``contracts``                  — (input,output) schema hashes
* ``artifact_contracts``         — M:N artifact↔contract
* ``artifact_promotions``        — idempotence ledger from M05
* ``artifact_state_transitions`` — FSM journal (auditable)

The models here stay faithful to the SQL DDL in the architecture
document (§6.1) while staying compatible with SQLite for tests
(``JSONB`` maps to ``JSON``; ``BYTEA`` maps to ``LargeBinary``;
partial indexes are created only on dialects that support them).
"""

from __future__ import annotations

from datetime import datetime
from typing import Any

from sqlalchemy import (
    JSON,
    BigInteger,
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
    event,
    func,
    text,
)
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship


class Base(DeclarativeBase):
    """Declarative root for all efe_registry ORM models."""


# ---------------------------------------------------------------------------
# Artifacts
# ---------------------------------------------------------------------------


class Artifact(Base):
    __tablename__ = "artifacts"
    __table_args__ = (
        UniqueConstraint("tenant_id", "task_id", "semver",
                          name="uq_artifact_tenant_task_semver"),
        UniqueConstraint("content_hash", name="uq_artifact_content_hash"),
        Index("ix_artifacts_tenant_task_state",
              "tenant_id", "task_id", "state"),
        Index("ix_artifacts_state_promoted",
              "state", "promoted_at"),
        CheckConstraint(
            "state IN ('draft','uploaded','signed','admitted',"
            "'serving','champion','deprecated','purged',"
            "'quarantined','rejected')",
            name="ck_artifact_state",
        ),
        CheckConstraint(
            "framework IN ('vllm','peft','full','other')",
            name="ck_artifact_framework",
        ),
        CheckConstraint("size_bytes > 0", name="ck_artifact_size_positive"),
    )

    artifact_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    semver: Mapped[str] = mapped_column(String(64), nullable=False)
    content_hash: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)
    oci_ref: Mapped[str] = mapped_column(Text, nullable=False)
    s3_url: Mapped[str] = mapped_column(Text, nullable=False)
    parent_run_id: Mapped[str] = mapped_column(String(36), nullable=False)
    parent_decision_id: Mapped[str | None] = mapped_column(
        String(36), nullable=True,
    )
    size_bytes: Mapped[int] = mapped_column(BigInteger, nullable=False)
    framework: Mapped[str] = mapped_column(String(16), nullable=False)
    state: Mapped[str] = mapped_column(String(16), nullable=False)
    state_version: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=0, server_default="0",
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    uploaded_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    signed_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    admitted_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    served_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    promoted_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    deprecated_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    purged_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )

    signatures: Mapped[list[ArtifactSignature]] = relationship(
        back_populates="artifact", cascade="all, delete-orphan",
    )
    sboms: Mapped[list[SBOM]] = relationship(
        back_populates="artifact", cascade="all, delete-orphan",
    )
    provenance: Mapped[list[SLSAProvenance]] = relationship(
        back_populates="artifact", cascade="all, delete-orphan",
    )


# ---------------------------------------------------------------------------
# Signatures
# ---------------------------------------------------------------------------


class ArtifactSignature(Base):
    __tablename__ = "artifact_signatures"
    __table_args__ = (
        CheckConstraint(
            "signing_method IN ('key_kms','keyless_oidc','attestation')",
            name="ck_sig_method",
        ),
        Index("ix_sig_artifact", "artifact_id"),
    )

    signature_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    artifact_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id", ondelete="CASCADE"),
        nullable=False,
    )
    signing_method: Mapped[str] = mapped_column(String(32), nullable=False)
    key_id: Mapped[str | None] = mapped_column(String(256), nullable=True)
    fulcio_cert_pem: Mapped[str | None] = mapped_column(Text, nullable=True)
    rekor_entry_uuid: Mapped[str | None] = mapped_column(
        String(128), nullable=True, unique=True,
    )
    rekor_inclusion_proof_json: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    signature_b64: Mapped[str] = mapped_column(Text, nullable=False)
    payload_sha256: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)
    signing_identity: Mapped[str | None] = mapped_column(Text, nullable=True)
    signing_issuer: Mapped[str | None] = mapped_column(Text, nullable=True)
    verified_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    verified_ok: Mapped[bool] = mapped_column(
        default=False, server_default="0", nullable=False,
    )
    verification_details: Mapped[dict[str, Any] | None] = mapped_column(
        JSON, nullable=True,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )

    artifact: Mapped[Artifact] = relationship(back_populates="signatures")


# ---------------------------------------------------------------------------
# SBOM
# ---------------------------------------------------------------------------


class SBOM(Base):
    __tablename__ = "sboms"
    __table_args__ = (
        UniqueConstraint("artifact_id", "format",
                          name="uq_sbom_artifact_format"),
    )

    sbom_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    artifact_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id", ondelete="CASCADE"),
        nullable=False,
    )
    format: Mapped[str] = mapped_column(
        String(32), nullable=False, default="cyclonedx-1.5",
    )
    content_jsonb: Mapped[dict[str, Any]] = mapped_column(JSON, nullable=False)
    content_sha256: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)
    attestation_rekor_uuid: Mapped[str | None] = mapped_column(
        String(128), nullable=True,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )

    artifact: Mapped[Artifact] = relationship(back_populates="sboms")


# ---------------------------------------------------------------------------
# SLSA provenance
# ---------------------------------------------------------------------------


class SLSAProvenance(Base):
    __tablename__ = "slsa_provenance"
    __table_args__ = (
        CheckConstraint("slsa_level BETWEEN 1 AND 4",
                         name="ck_slsa_level"),
    )

    provenance_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    artifact_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id", ondelete="CASCADE"),
        nullable=False,
    )
    slsa_level: Mapped[int] = mapped_column(SmallInteger, nullable=False)
    builder_id: Mapped[str] = mapped_column(Text, nullable=False)
    build_type: Mapped[str] = mapped_column(Text, nullable=False)
    invocation_jsonb: Mapped[dict[str, Any]] = mapped_column(
        JSON, nullable=False,
    )
    metadata_jsonb: Mapped[dict[str, Any]] = mapped_column(
        JSON, nullable=False,
    )
    materials_jsonb: Mapped[dict[str, Any]] = mapped_column(
        JSON, nullable=False,
    )
    dsse_envelope_b64: Mapped[str] = mapped_column(Text, nullable=False)
    rekor_entry_uuid: Mapped[str | None] = mapped_column(
        String(128), nullable=True,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )

    artifact: Mapped[Artifact] = relationship(back_populates="provenance")


# ---------------------------------------------------------------------------
# Endpoints
# ---------------------------------------------------------------------------


class Endpoint(Base):
    __tablename__ = "endpoints"
    __table_args__ = (
        CheckConstraint(
            "role IN ('champion','candidate','shadow','standby_warm')",
            name="ck_endpoint_role",
        ),
        CheckConstraint(
            "state IN ('pending','loading','ready',"
            "'draining','drained','failed')",
            name="ck_endpoint_state",
        ),
        CheckConstraint(
            "tensor_parallel IN (1,2,4,8)",
            name="ck_endpoint_tp",
        ),
        Index("ix_endpoints_state", "state"),
        Index("ix_endpoints_artifact", "artifact_id"),
        Index("ix_endpoints_tenant_task",
              "tenant_id", "task_id", "role"),
    )

    endpoint_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    pool_id: Mapped[str] = mapped_column(String(64), nullable=False)
    pod_name: Mapped[str] = mapped_column(
        String(128), nullable=False, unique=True,
    )
    namespace: Mapped[str] = mapped_column(String(64), nullable=False)
    url: Mapped[str] = mapped_column(Text, nullable=False)
    # Denormalised tenancy: needed for tenant-scoped queries in the router
    # and promoter. Kept in sync with ``artifact.tenant_id`` at endpoint
    # creation time (K8s operator / promoter path).
    tenant_id: Mapped[str] = mapped_column(
        String(128), nullable=False, default="", server_default="",
    )
    task_id: Mapped[str] = mapped_column(
        String(128), nullable=False, default="", server_default="",
    )
    artifact_id: Mapped[str | None] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id"), nullable=True,
    )
    role: Mapped[str] = mapped_column(String(16), nullable=False)
    state: Mapped[str] = mapped_column(String(16), nullable=False)
    readiness_since: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    last_heartbeat_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    tensor_parallel: Mapped[int] = mapped_column(
        SmallInteger, nullable=False, default=1,
    )
    lora_capacity: Mapped[int] = mapped_column(
        SmallInteger, nullable=False, default=0,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


# ---------------------------------------------------------------------------
# Routing table
# ---------------------------------------------------------------------------


class RoutingTableRow(Base):
    __tablename__ = "routing_table"
    __table_args__ = (
        CheckConstraint("weight_percent BETWEEN 0 AND 100",
                         name="ck_routing_weight"),
        CheckConstraint(
            "role IN ('champion','candidate','shadow','standby_warm')",
            name="ck_routing_role",
        ),
        Index("ix_routing_tenant_task", "tenant_id", "task_id"),
    )

    tenant_id: Mapped[str] = mapped_column(String(128), primary_key=True)
    task_id: Mapped[str] = mapped_column(String(128), primary_key=True)
    variant: Mapped[str] = mapped_column(
        String(64), primary_key=True, default="default",
    )
    endpoint_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("endpoints.endpoint_id"),
        primary_key=True,
    )
    weight_percent: Mapped[int] = mapped_column(SmallInteger, nullable=False)
    priority: Mapped[int] = mapped_column(
        SmallInteger, nullable=False, default=0,
    )
    version: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=0, server_default="0",
    )
    role: Mapped[str] = mapped_column(String(16), nullable=False)
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    updated_by: Mapped[str] = mapped_column(String(128), nullable=False)


# ---------------------------------------------------------------------------
# Canary runs
# ---------------------------------------------------------------------------


class CanaryRun(Base):
    __tablename__ = "canary_runs"
    __table_args__ = (
        CheckConstraint(
            "current_percent BETWEEN 0 AND 100",
            name="ck_canary_current_percent",
        ),
        CheckConstraint(
            "target_percent BETWEEN 0 AND 100",
            name="ck_canary_target_percent",
        ),
        CheckConstraint(
            "schedule_kind IN ('geometric','linear','manual')",
            name="ck_canary_schedule",
        ),
        CheckConstraint(
            "state IN ('ramping','paused','completed',"
            "'rolled_back','aborted')",
            name="ck_canary_state",
        ),
    )

    canary_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    candidate_artifact: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id"), nullable=False,
    )
    baseline_artifact: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id"), nullable=False,
    )
    current_percent: Mapped[int] = mapped_column(
        SmallInteger, nullable=False, default=1,
    )
    target_percent: Mapped[int] = mapped_column(
        SmallInteger, nullable=False, default=100,
    )
    schedule_kind: Mapped[str] = mapped_column(
        String(16), nullable=False, default="geometric",
    )
    step_duration_s: Mapped[int] = mapped_column(
        Integer, nullable=False, default=600,
    )
    state: Mapped[str] = mapped_column(String(16), nullable=False)
    started_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    last_step_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    ended_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    slo_breach_count: Mapped[int] = mapped_column(
        SmallInteger, nullable=False, default=0,
    )
    auto_rollback: Mapped[bool] = mapped_column(
        nullable=False, default=True, server_default="1",
    )
    auto_advance: Mapped[bool] = mapped_column(
        nullable=False, default=True, server_default="1",
    )


# Postgres-specific partial unique index (applied only on Postgres).
# SQLite ignores this in tests; application-level exclusion is additionally
# enforced in :class:`efe_registry.core.promoter.ArtifactPromoter` before
# insert so SQLite-only environments still respect the invariant.
@event.listens_for(CanaryRun.__table__, "after_create")
def _maybe_create_canary_partial_index(
    target: Any, connection: Any, **kw: Any,
) -> None:
    if connection.dialect.name == "postgresql":
        connection.execute(text(
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_canary_active "
            "ON canary_runs (tenant_id, task_id) "
            "WHERE state IN ('ramping','paused')",
        ))


# ---------------------------------------------------------------------------
# Shadow taps
# ---------------------------------------------------------------------------


class ShadowTap(Base):
    __tablename__ = "shadow_taps"
    __table_args__ = (
        CheckConstraint(
            "tap_rate_percent BETWEEN 0 AND 100",
            name="ck_shadow_rate",
        ),
    )

    shadow_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    candidate_artifact: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id"), nullable=False,
    )
    baseline_artifact: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id"), nullable=False,
    )
    tap_rate_percent: Mapped[int] = mapped_column(
        SmallInteger, nullable=False,
    )
    started_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    ended_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    n_samples: Mapped[int] = mapped_column(
        BigInteger, nullable=False, default=0,
    )
    cosine_mean: Mapped[float | None] = mapped_column(Double, nullable=True)
    cosine_p05: Mapped[float | None] = mapped_column(Double, nullable=True)
    latency_delta_ms_mean: Mapped[float | None] = mapped_column(
        Double, nullable=True,
    )


# ---------------------------------------------------------------------------
# Rollback events
# ---------------------------------------------------------------------------


class RollbackEvent(Base):
    __tablename__ = "rollback_events"
    __table_args__ = (
        CheckConstraint(
            "trigger_kind IN ('manual','auto_slo','auto_regression','admin')",
            name="ck_rollback_trigger",
        ),
        CheckConstraint(
            "state IN ('requested','in_progress','completed','failed')",
            name="ck_rollback_state",
        ),
        UniqueConstraint("idempotency_key", name="uq_rollback_idem"),
        Index("ix_rollback_tenant_task", "tenant_id", "task_id"),
    )

    rollback_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    # Nullable at request time — populated during the LOOKUP phase. Without
    # nullable FK columns, the initial INSERT would crash on Postgres with
    # a FK violation (sentinel strings like "pending" are not valid
    # artifact_ids).
    from_artifact: Mapped[str | None] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id"), nullable=True,
    )
    to_artifact: Mapped[str | None] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id"), nullable=True,
    )
    trigger_kind: Mapped[str] = mapped_column(String(32), nullable=False)
    actor: Mapped[str] = mapped_column(String(128), nullable=False)
    reason: Mapped[str] = mapped_column(Text, nullable=False)
    idempotency_key: Mapped[bytes] = mapped_column(
        LargeBinary, nullable=False,
    )
    state: Mapped[str] = mapped_column(String(16), nullable=False)
    requested_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False,
    )
    started_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    completed_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    latency_ms: Mapped[int | None] = mapped_column(Integer, nullable=True)
    phases_jsonb: Mapped[dict[str, Any]] = mapped_column(
        JSON, nullable=False, default=dict, server_default="{}",
    )


# ---------------------------------------------------------------------------
# Contracts
# ---------------------------------------------------------------------------


class Contract(Base):
    __tablename__ = "contracts"

    contract_hash: Mapped[bytes] = mapped_column(
        LargeBinary, primary_key=True,
    )
    input_schema_canonical: Mapped[dict[str, Any]] = mapped_column(
        JSON, nullable=False,
    )
    output_schema_canonical: Mapped[dict[str, Any]] = mapped_column(
        JSON, nullable=False,
    )
    registered_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    deprecated_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    sunset_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    description: Mapped[str | None] = mapped_column(Text, nullable=True)


class ArtifactContract(Base):
    __tablename__ = "artifact_contracts"

    artifact_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id", ondelete="CASCADE"),
        primary_key=True,
    )
    contract_hash: Mapped[bytes] = mapped_column(
        LargeBinary, ForeignKey("contracts.contract_hash"),
        primary_key=True,
    )


# ---------------------------------------------------------------------------
# Promotions (idempotence ledger)
# ---------------------------------------------------------------------------


class ArtifactPromotion(Base):
    __tablename__ = "artifact_promotions"

    decision_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    artifact_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id"), nullable=False,
    )
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    received_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    accepted: Mapped[bool] = mapped_column(nullable=False)
    rejection_reason: Mapped[str | None] = mapped_column(
        Text, nullable=True,
    )


# ---------------------------------------------------------------------------
# FSM journal
# ---------------------------------------------------------------------------


class ArtifactStateTransition(Base):
    __tablename__ = "artifact_state_transitions"
    __table_args__ = (
        Index("ix_transitions_artifact_ts", "artifact_id", "ts"),
    )

    transition_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    artifact_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("artifacts.artifact_id", ondelete="CASCADE"),
        nullable=False,
    )
    from_state: Mapped[str] = mapped_column(String(16), nullable=False)
    to_state: Mapped[str] = mapped_column(String(16), nullable=False)
    old_version: Mapped[int] = mapped_column(BigInteger, nullable=False)
    new_version: Mapped[int] = mapped_column(BigInteger, nullable=False)
    actor: Mapped[str] = mapped_column(String(128), nullable=False)
    reason: Mapped[str | None] = mapped_column(Text, nullable=True)
    nats_msg_id: Mapped[str | None] = mapped_column(
        String(128), nullable=True,
    )
    ts: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


__all__ = [
    "SBOM",
    "Artifact",
    "ArtifactContract",
    "ArtifactPromotion",
    "ArtifactSignature",
    "ArtifactStateTransition",
    "Base",
    "CanaryRun",
    "Contract",
    "Endpoint",
    "RollbackEvent",
    "RoutingTableRow",
    "SLSAProvenance",
    "ShadowTap",
]

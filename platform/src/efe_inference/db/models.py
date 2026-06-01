"""SQLAlchemy 2.0 async ORM — 7 tables for efe_inference.

Schema overview (M07_ARCHITECTURE.md §6.1):

* ``inference_backends``       — deployed backends (vLLM/Claude/GPT/...)
* ``prompt_templates``         — versioned Jinja2 templates per model family
* ``tool_schemas``             — JSON-Schema'd tools exposed to the model
* ``safety_policies``          — per-tenant safety policy bundles
* ``inference_logs``           — sampled request log (NO PII, only hashes)
* ``safety_events``            — every block / flag, signed HMAC
* ``circuit_states``           — persisted circuit-breaker last-known state
"""

from __future__ import annotations

from datetime import datetime
from typing import Any

from sqlalchemy import (
    JSON,
    CheckConstraint,
    DateTime,
    ForeignKey,
    Index,
    Integer,
    LargeBinary,
    Numeric,
    SmallInteger,
    String,
    Text,
    UniqueConstraint,
    func,
)
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


class Base(DeclarativeBase):
    """Declarative root for all efe_inference ORM models."""


# ---------------------------------------------------------------------------
# Backends
# ---------------------------------------------------------------------------


class InferenceBackend(Base):
    __tablename__ = "inference_backends"
    __table_args__ = (
        UniqueConstraint("name", name="uq_backend_name"),
        CheckConstraint(
            "kind IN ('vllm_local','anthropic','openai','mistral',"
            "'peft_local','mock')",
            name="ck_backend_kind",
        ),
        CheckConstraint(
            "state IN ('active','draining','disabled')",
            name="ck_backend_state",
        ),
    )

    backend_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    name: Mapped[str] = mapped_column(String(64), nullable=False)
    kind: Mapped[str] = mapped_column(String(32), nullable=False)
    endpoint_url: Mapped[str | None] = mapped_column(Text, nullable=True)
    api_key_secret_id: Mapped[str | None] = mapped_column(
        String(256), nullable=True,
    )
    model_family: Mapped[str] = mapped_column(String(32), nullable=False)
    tokenizer_name: Mapped[str] = mapped_column(String(128), nullable=False)
    capabilities: Mapped[dict[str, Any]] = mapped_column(
        JSON, nullable=False, default=dict, server_default="{}",
    )
    cost_per_1k_prompt_usd: Mapped[float | None] = mapped_column(
        Numeric(10, 6), nullable=True,
    )
    cost_per_1k_completion_usd: Mapped[float | None] = mapped_column(
        Numeric(10, 6), nullable=True,
    )
    max_context_tokens: Mapped[int] = mapped_column(Integer, nullable=False)
    rate_limit_rpm: Mapped[int | None] = mapped_column(
        Integer, nullable=True,
    )
    priority: Mapped[int] = mapped_column(
        SmallInteger, nullable=False, default=100,
    )
    state: Mapped[str] = mapped_column(String(16), nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


# ---------------------------------------------------------------------------
# Prompt templates
# ---------------------------------------------------------------------------


class PromptTemplate(Base):
    __tablename__ = "prompt_templates"
    __table_args__ = (
        UniqueConstraint("name", "model_family", "version",
                          name="uq_template_name_family_version"),
    )

    template_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    name: Mapped[str] = mapped_column(String(64), nullable=False)
    model_family: Mapped[str] = mapped_column(String(32), nullable=False)
    version: Mapped[str] = mapped_column(String(32), nullable=False)
    content: Mapped[str] = mapped_column(Text, nullable=False)
    content_hash: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    deprecated_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )


# ---------------------------------------------------------------------------
# Tool schemas
# ---------------------------------------------------------------------------


class ToolSchema(Base):
    __tablename__ = "tool_schemas"
    __table_args__ = (
        UniqueConstraint("tenant_id", "name", "version",
                          name="uq_tool_tenant_name_version"),
    )

    tool_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    name: Mapped[str] = mapped_column(String(64), nullable=False)
    version: Mapped[str] = mapped_column(String(32), nullable=False)
    description: Mapped[str] = mapped_column(Text, nullable=False)
    parameters_schema: Mapped[dict[str, Any]] = mapped_column(
        JSON, nullable=False,
    )
    parameters_hash: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    deprecated_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )


# ---------------------------------------------------------------------------
# Safety policies
# ---------------------------------------------------------------------------


class SafetyPolicy(Base):
    __tablename__ = "safety_policies"
    __table_args__ = (
        UniqueConstraint("tenant_id", "name",
                          name="uq_policy_tenant_name"),
    )

    policy_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    name: Mapped[str] = mapped_column(String(64), nullable=False)
    pii_categories: Mapped[list[str]] = mapped_column(
        JSON, nullable=False,
        default=list,
        server_default='["email","phone","iban","ssn","credit_card"]',
    )
    moderation_thresholds: Mapped[dict[str, float]] = mapped_column(
        JSON, nullable=False, default=dict, server_default="{}",
    )
    block_on_injection_risk: Mapped[int] = mapped_column(
        SmallInteger, nullable=False, default=80,
    )
    allow_list_regex: Mapped[list[str]] = mapped_column(
        JSON, nullable=False, default=list, server_default="[]",
    )
    deny_list_regex: Mapped[list[str]] = mapped_column(
        JSON, nullable=False, default=list, server_default="[]",
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )
    active_from: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    active_until: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )


# ---------------------------------------------------------------------------
# Sampled inference log (NO PII — hashes only)
# ---------------------------------------------------------------------------


class InferenceLog(Base):
    __tablename__ = "inference_logs"
    __table_args__ = (
        Index("ix_inf_logs_tenant_time",
              "tenant_id", "created_at"),
    )

    request_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    task_id: Mapped[str] = mapped_column(String(128), nullable=False)
    backend_id: Mapped[str | None] = mapped_column(
        String(36), ForeignKey("inference_backends.backend_id"),
        nullable=True,
    )
    prompt_hash: Mapped[bytes | None] = mapped_column(
        LargeBinary, nullable=True,
    )
    response_hash: Mapped[bytes | None] = mapped_column(
        LargeBinary, nullable=True,
    )
    prompt_tokens: Mapped[int] = mapped_column(Integer, nullable=False)
    completion_tokens: Mapped[int] = mapped_column(Integer, nullable=False)
    ttft_ms: Mapped[int | None] = mapped_column(Integer, nullable=True)
    total_latency_ms: Mapped[int | None] = mapped_column(Integer, nullable=True)
    finish_reason: Mapped[str | None] = mapped_column(
        String(32), nullable=True,
    )
    cache_kind: Mapped[str | None] = mapped_column(String(16), nullable=True)
    safety_flags: Mapped[dict[str, Any]] = mapped_column(
        JSON, nullable=False, default=dict, server_default="{}",
    )
    cost_usd: Mapped[float | None] = mapped_column(
        Numeric(12, 8), nullable=True,
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


# ---------------------------------------------------------------------------
# Safety events (signed HMAC)
# ---------------------------------------------------------------------------


class SafetyEvent(Base):
    __tablename__ = "safety_events"
    __table_args__ = (
        Index("ix_safety_tenant_time",
              "tenant_id", "created_at"),
        CheckConstraint(
            "severity IN ('info','warn','block')",
            name="ck_safety_severity",
        ),
        CheckConstraint(
            "stage IN ('preflight_pii','preflight_moderation',"
            "'preflight_injection','postflight_pii',"
            "'postflight_moderation','postflight_policy')",
            name="ck_safety_stage",
        ),
    )

    event_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    request_id: Mapped[str] = mapped_column(String(36), nullable=False)
    tenant_id: Mapped[str] = mapped_column(String(128), nullable=False)
    stage: Mapped[str] = mapped_column(String(32), nullable=False)
    severity: Mapped[str] = mapped_column(String(16), nullable=False)
    categories: Mapped[list[str]] = mapped_column(JSON, nullable=False)
    payload_hash: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)
    signature_b64: Mapped[str] = mapped_column(Text, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


# ---------------------------------------------------------------------------
# Circuit-breaker persisted state (rehydrated on pod boot)
# ---------------------------------------------------------------------------


class CircuitState(Base):
    __tablename__ = "circuit_states"
    __table_args__ = (
        CheckConstraint(
            "state IN ('closed','open','half_open')",
            name="ck_circuit_state",
        ),
    )

    tenant_id: Mapped[str] = mapped_column(String(128), primary_key=True)
    backend_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("inference_backends.backend_id"),
        primary_key=True,
    )
    state: Mapped[str] = mapped_column(String(16), nullable=False)
    opened_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True,
    )
    failure_count: Mapped[int] = mapped_column(
        Integer, nullable=False, default=0, server_default="0",
    )
    success_count: Mapped[int] = mapped_column(
        Integer, nullable=False, default=0, server_default="0",
    )
    last_transition_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=func.now(),
    )


__all__ = [
    "Base",
    "CircuitState",
    "InferenceBackend",
    "InferenceLog",
    "PromptTemplate",
    "SafetyEvent",
    "SafetyPolicy",
    "ToolSchema",
]

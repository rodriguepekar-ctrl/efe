"""Hierarchical configuration for :mod:`efe_rollout`.

Precedence (highest → lowest):

1. constructor ``overrides`` (programmatic),
2. environment variables prefixed by ``EFE_ROLLOUT_`` (``SECTION__KEY``),
3. built-in defaults.

Every section is a frozen ``pydantic.BaseModel`` with ``extra="forbid"``
so misspelled keys fail fast at load time rather than silently taking a
default.
"""

from __future__ import annotations

import os
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from efe_rollout.errors import ConfigError

# ---------------------------------------------------------------------------
# Storage
# ---------------------------------------------------------------------------


class DatabaseSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = Field(
        default="sqlite+aiosqlite:///./efe_rollout.sqlite",
        description="Async SQLAlchemy URL. Prod: postgresql+asyncpg://...",
    )
    pool_size: int = Field(default=20, ge=1, le=200)
    pool_max_overflow: int = Field(default=10, ge=0, le=200)
    pool_timeout_s: float = Field(default=5.0, ge=0.1, le=60.0)
    echo: bool = False


class RedisSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = "redis://localhost:6379/2"
    assignment_cache_prefix: str = "rollout:assign"
    bandit_state_prefix: str = "rollout:bandit"
    guardrail_prefix: str = "rollout:guardrail"
    pubsub_channel_prefix: str = "rollout"


# ---------------------------------------------------------------------------
# Assignment
# ---------------------------------------------------------------------------


class AssignmentSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    # Holdout expressed in basis points (10 000 = 100 %). 100 bps = 1 %.
    holdout_basis_points: int = Field(default=100, ge=0, le=10_000)
    # Default traffic allocation cap per experiment (100 % of layer).
    traffic_allocation_basis_points: int = Field(
        default=10_000, ge=0, le=10_000,
    )
    cache_ttl_s: int = Field(default=3600, ge=1, le=7 * 86_400)
    assignment_hmac_key_hex: str = Field(default="0" * 64)

    @field_validator("assignment_hmac_key_hex")
    @classmethod
    def _check_hex(cls, v: str) -> str:
        if len(v) != 64:
            raise ValueError(
                f"assignment_hmac_key_hex must be 64 hex chars, got {len(v)}",
            )
        try:
            bytes.fromhex(v)
        except ValueError as exc:
            raise ValueError(f"assignment_hmac_key_hex not hex: {exc}") from exc
        return v


# ---------------------------------------------------------------------------
# Layers
# ---------------------------------------------------------------------------


class LayerSaltSpec(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    salt: str

    @field_validator("name")
    @classmethod
    def _nonempty_name(cls, v: str) -> str:
        if not v.strip():
            raise ValueError("layer name must be non-empty")
        return v.strip()


class LayerSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    layers: tuple[LayerSaltSpec, ...] = Field(
        default_factory=lambda: (
            LayerSaltSpec(name="prompt", salt="salt-prompt-v1"),
            LayerSaltSpec(name="model", salt="salt-model-v1"),
            LayerSaltSpec(name="postproc", salt="salt-postproc-v1"),
        ),
    )
    allow_custom_layers: bool = True


# ---------------------------------------------------------------------------
# Ingestion
# ---------------------------------------------------------------------------


class IngestSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    exposure_buffer: int = Field(default=100, ge=1, le=10_000)
    outcome_buffer: int = Field(default=100, ge=1, le=10_000)
    flush_interval_s: float = Field(default=1.0, ge=0.05, le=60.0)
    strict_itt: bool = False  # reject outcomes without prior exposure
    exposure_hmac_key_hex: str = Field(default="0" * 64)

    @field_validator("exposure_hmac_key_hex")
    @classmethod
    def _check_hex(cls, v: str) -> str:
        if len(v) != 64:
            raise ValueError(
                f"exposure_hmac_key_hex must be 64 hex chars, got {len(v)}",
            )
        try:
            bytes.fromhex(v)
        except ValueError as exc:
            raise ValueError(f"exposure_hmac_key_hex not hex: {exc}") from exc
        return v


# ---------------------------------------------------------------------------
# Analysis
# ---------------------------------------------------------------------------


class AnalysisSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    default_alpha: float = Field(default=0.05, gt=0.0, lt=0.5)
    default_power: float = Field(default=0.8, gt=0.5, lt=1.0)
    apply_cuped_if_covariate: bool = True
    apply_cluster_robust_se: bool = True
    bootstrap_iterations: int = Field(default=2_000, ge=100, le=100_000)
    bayesian_posterior_samples: int = Field(
        default=50_000, ge=1_000, le=1_000_000,
    )
    bh_alpha: float = Field(default=0.05, gt=0.0, lt=0.5)


# ---------------------------------------------------------------------------
# Bandits
# ---------------------------------------------------------------------------


class BanditSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    batch_update_interval_s: int = Field(default=300, ge=10, le=3_600)
    minimum_allocation_pct: int = Field(default=5, ge=0, le=50)
    explore_boost_pulls: int = Field(default=1_000, ge=0, le=100_000)
    epsilon_default: float = Field(default=0.05, gt=0.0, lt=1.0)
    epsilon_decay: float = Field(default=0.999, gt=0.0, lt=1.0)


# ---------------------------------------------------------------------------
# Guardrails
# ---------------------------------------------------------------------------


class GuardrailSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    default_window_s: int = Field(default=300, ge=30, le=86_400)
    default_consecutive_breaches: int = Field(default=3, ge=1, le=20)
    default_aging_s: int = Field(default=86_400, ge=300, le=14 * 86_400)
    tick_period_s: int = Field(default=60, ge=5, le=600)
    auto_abort_deadline_s: float = Field(default=30.0, ge=1.0, le=600.0)


# ---------------------------------------------------------------------------
# NATS
# ---------------------------------------------------------------------------


class NatsSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = "nats://localhost:4222"
    topic_prefix: str = "efe.rollout"
    consumer_durable_name: str = "efe-rollout-consumer"
    message_schema_version: str = "1.0"
    hmac_key_hex: str = Field(default="0" * 64)
    envelope_skew_window_s: int = Field(default=300, ge=30, le=86_400)

    @field_validator("hmac_key_hex")
    @classmethod
    def _check_hex(cls, v: str) -> str:
        if len(v) != 64:
            raise ValueError(
                f"hmac_key_hex must be 64 hex chars, got {len(v)}",
            )
        try:
            bytes.fromhex(v)
        except ValueError as exc:
            raise ValueError(f"hmac_key_hex not hex: {exc}") from exc
        return v


# ---------------------------------------------------------------------------
# Pre-registration
# ---------------------------------------------------------------------------


class PreRegistrationSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    signing_key_id: str = "alias/efe-rollout"
    signing_key_hex: str = Field(default="0" * 64)

    @field_validator("signing_key_hex")
    @classmethod
    def _check_hex(cls, v: str) -> str:
        if len(v) != 64:
            raise ValueError(
                f"signing_key_hex must be 64 hex chars, got {len(v)}",
            )
        try:
            bytes.fromhex(v)
        except ValueError as exc:
            raise ValueError(f"signing_key_hex not hex: {exc}") from exc
        return v


# ---------------------------------------------------------------------------
# API
# ---------------------------------------------------------------------------


class ApiSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    host: str = "0.0.0.0"
    port: int = Field(default=8088, ge=1, le=65_535)
    request_timeout_s: float = Field(default=30.0, ge=0.1, le=600.0)
    enable_rest: bool = True


# ---------------------------------------------------------------------------
# Observability
# ---------------------------------------------------------------------------


class ObservabilitySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    service_name: str = "efe-rollout"
    metrics_namespace: str = "efe_rollout"
    enable_prometheus: bool = True
    log_level: Literal["DEBUG", "INFO", "WARNING", "ERROR"] = "INFO"
    log_json: bool = True
    otel_endpoint: str | None = None
    otel_sampling_ratio: float = Field(default=0.1, ge=0.0, le=1.0)


# ---------------------------------------------------------------------------
# Top-level
# ---------------------------------------------------------------------------


class RolloutSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    environment: Literal["dev", "staging", "prod", "test"] = "dev"
    database: DatabaseSettings = Field(default_factory=DatabaseSettings)
    redis: RedisSettings = Field(default_factory=RedisSettings)
    assignment: AssignmentSettings = Field(default_factory=AssignmentSettings)
    layers: LayerSettings = Field(default_factory=LayerSettings)
    ingest: IngestSettings = Field(default_factory=IngestSettings)
    analysis: AnalysisSettings = Field(default_factory=AnalysisSettings)
    bandits: BanditSettings = Field(default_factory=BanditSettings)
    guardrails: GuardrailSettings = Field(default_factory=GuardrailSettings)
    nats: NatsSettings = Field(default_factory=NatsSettings)
    prereg: PreRegistrationSettings = Field(
        default_factory=PreRegistrationSettings,
    )
    api: ApiSettings = Field(default_factory=ApiSettings)
    observability: ObservabilitySettings = Field(
        default_factory=ObservabilitySettings,
    )

    @classmethod
    def from_env(
        cls, overrides: dict[str, Any] | None = None,
    ) -> RolloutSettings:
        data = _from_environ("EFE_ROLLOUT_")
        if overrides:
            data = _merge(data, overrides)
        try:
            return cls.model_validate(data)
        except Exception as exc:
            raise ConfigError(
                f"invalid efe-rollout configuration: {exc}",
            ) from exc


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _merge(a: dict[str, Any], b: dict[str, Any]) -> dict[str, Any]:
    out = dict(a)
    for k, v in b.items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = _merge(out[k], v)
        else:
            out[k] = v
    return out


def _from_environ(prefix: str) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for raw_key, raw_val in os.environ.items():
        if not raw_key.startswith(prefix):
            continue
        tail = raw_key[len(prefix):]
        parts = tail.lower().split("__")
        cursor = out
        for head in parts[:-1]:
            nxt = cursor.setdefault(head, {})
            if not isinstance(nxt, dict):
                raise ConfigError(f"env conflict at {raw_key}")
            cursor = nxt
        cursor[parts[-1]] = _coerce(raw_val)
    return out


def _coerce(v: str) -> Any:
    low = v.strip().lower()
    if low in {"true", "yes", "on"}:
        return True
    if low in {"false", "no", "off"}:
        return False
    try:
        return int(v)
    except ValueError:
        pass
    try:
        return float(v)
    except ValueError:
        pass
    return v


__all__ = [
    "AnalysisSettings",
    "ApiSettings",
    "AssignmentSettings",
    "BanditSettings",
    "DatabaseSettings",
    "GuardrailSettings",
    "IngestSettings",
    "LayerSaltSpec",
    "LayerSettings",
    "NatsSettings",
    "ObservabilitySettings",
    "PreRegistrationSettings",
    "RedisSettings",
    "RolloutSettings",
]

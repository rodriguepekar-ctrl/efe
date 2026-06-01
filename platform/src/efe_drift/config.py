"""Hierarchical configuration for :mod:`efe_drift`.

Precedence (highest → lowest) :

1. constructor ``overrides`` (programmatic),
2. environment variables prefixed by ``EFE_DRIFT_`` (``SECTION__KEY``),
3. built-in defaults.

Every section is a frozen ``pydantic.BaseModel`` with ``extra="forbid"``
so misspelled keys fail fast at load time.
"""

from __future__ import annotations

import os
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from efe_drift.errors import ConfigError

# ---------------------------------------------------------------------------
# Storage
# ---------------------------------------------------------------------------


class DatabaseSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = Field(
        default="sqlite+aiosqlite:///./efe_drift.sqlite",
        description="Async SQLAlchemy URL. Prod: postgresql+asyncpg://...",
    )
    pool_size: int = Field(default=20, ge=1, le=200)
    pool_max_overflow: int = Field(default=10, ge=0, le=200)
    pool_timeout_s: float = Field(default=5.0, ge=0.1, le=60.0)
    echo: bool = False


class RedisSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = "redis://localhost:6379/3"
    score_cache_prefix: str = "drift:score"
    alert_suppress_prefix: str = "drift:alert:suppress"


# ---------------------------------------------------------------------------
# Ingestion
# ---------------------------------------------------------------------------


class IngestionSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    nats_subject_filter: tuple[str, ...] = (
        "efe.inference.completed",
        "efe.rollout.outcome",
    )
    buffer_size: int = Field(default=10_000, ge=100, le=1_000_000)
    flush_interval_s: float = Field(default=1.0, ge=0.05, le=60.0)
    max_ack_pending: int = Field(default=10_000, ge=100, le=1_000_000)
    backpressure_alert_s: float = Field(default=30.0, ge=1.0, le=600.0)


# ---------------------------------------------------------------------------
# Extraction
# ---------------------------------------------------------------------------


class ExtractionSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    sbert_model_name: str = "all-MiniLM-L6-v2"
    sbert_device: Literal["cpu", "cuda"] = "cpu"
    sbert_batch_size: int = Field(default=32, ge=1, le=512)
    lang_detect_min_chars: int = Field(default=8, ge=1, le=256)
    # Per-tenant salt used to pseudonymise user_ids before histogram
    # aggregation. 64 hex chars = 32 bytes.
    user_id_salt_hex: str = Field(default="0" * 64)

    @field_validator("user_id_salt_hex")
    @classmethod
    def _check_hex(cls, v: str) -> str:
        if len(v) != 64:
            raise ValueError(
                f"user_id_salt_hex must be 64 hex chars, got {len(v)}",
            )
        try:
            bytes.fromhex(v)
        except ValueError as exc:
            raise ValueError(f"user_id_salt_hex not hex: {exc}") from exc
        return v


# ---------------------------------------------------------------------------
# Aggregation
# ---------------------------------------------------------------------------


class AggregationSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    minute_rollup_period_s: int = Field(default=60, ge=10, le=600)
    hour_rollup_period_s: int = Field(default=3_600, ge=60, le=86_400)
    day_rollup_period_s: int = Field(default=86_400, ge=3_600, le=7 * 86_400)
    default_histogram_bins: int = Field(default=10, ge=2, le=200)
    embedding_reservoir_size: int = Field(
        default=10_000, ge=100, le=1_000_000,
    )
    # Top-K bucketing for high-cardinality categorical features.
    top_k_categorical: int = Field(default=10, ge=2, le=1000)


# ---------------------------------------------------------------------------
# Baseline
# ---------------------------------------------------------------------------


class BaselineSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    min_accumulation_days: int = Field(default=7, ge=1, le=90)
    default_overlap_days: int = Field(default=3, ge=1, le=14)
    min_sample_count: int = Field(default=10_000, ge=100, le=10_000_000)
    max_bin_skew_pct: float = Field(default=0.95, ge=0.5, le=1.0)
    auto_promote: bool = False
    signing_key_id: str = "alias/efe-drift-baseline"
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
# PII linter
# ---------------------------------------------------------------------------


class PiiLinterSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    enabled: bool = True
    sample_size: int = Field(default=1000, ge=10, le=1_000_000)
    # Minimum digit count for an identifier to be flagged.
    numeric_id_min_digits: int = Field(default=10, ge=6, le=32)
    # Schedule in cron syntax — interpreted by the scheduler outside
    # this module.
    schedule_cron: str = "0 2 * * *"


# ---------------------------------------------------------------------------
# NATS
# ---------------------------------------------------------------------------


class NatsSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = "nats://localhost:4222"
    topic_prefix: str = "efe.drift"
    consumer_durable_name: str = "efe-drift-consumer"
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
# API
# ---------------------------------------------------------------------------


class ApiSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    host: str = "0.0.0.0"
    port: int = Field(default=8089, ge=1, le=65_535)
    request_timeout_s: float = Field(default=30.0, ge=0.1, le=600.0)
    enable_rest: bool = True


# ---------------------------------------------------------------------------
# Observability
# ---------------------------------------------------------------------------


class ObservabilitySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    service_name: str = "efe-drift"
    metrics_namespace: str = "efe_drift"
    enable_prometheus: bool = True
    log_level: Literal["DEBUG", "INFO", "WARNING", "ERROR"] = "INFO"
    log_json: bool = True
    otel_endpoint: str | None = None
    otel_sampling_ratio: float = Field(default=0.1, ge=0.0, le=1.0)


# ---------------------------------------------------------------------------
# Retention
# ---------------------------------------------------------------------------


class RetentionSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    raw_events_days: int = Field(default=7, ge=1, le=90)
    snapshots_days: int = Field(default=90, ge=7, le=365)
    evaluations_days: int = Field(default=365, ge=30, le=3650)
    incidents_days: int = Field(default=365, ge=30, le=3650)


# ---------------------------------------------------------------------------
# Top-level
# ---------------------------------------------------------------------------


class DriftSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    environment: Literal["dev", "staging", "prod", "test"] = "dev"
    database: DatabaseSettings = Field(default_factory=DatabaseSettings)
    redis: RedisSettings = Field(default_factory=RedisSettings)
    ingestion: IngestionSettings = Field(default_factory=IngestionSettings)
    extraction: ExtractionSettings = Field(default_factory=ExtractionSettings)
    aggregation: AggregationSettings = Field(default_factory=AggregationSettings)
    baseline: BaselineSettings = Field(default_factory=BaselineSettings)
    pii_linter: PiiLinterSettings = Field(default_factory=PiiLinterSettings)
    nats: NatsSettings = Field(default_factory=NatsSettings)
    api: ApiSettings = Field(default_factory=ApiSettings)
    observability: ObservabilitySettings = Field(
        default_factory=ObservabilitySettings,
    )
    retention: RetentionSettings = Field(default_factory=RetentionSettings)

    @classmethod
    def from_env(
        cls, overrides: dict[str, Any] | None = None,
    ) -> DriftSettings:
        data = _from_environ("EFE_DRIFT_")
        if overrides:
            data = _merge(data, overrides)
        try:
            return cls.model_validate(data)
        except Exception as exc:
            raise ConfigError(
                f"invalid efe-drift configuration: {exc}",
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
    "AggregationSettings",
    "ApiSettings",
    "BaselineSettings",
    "DatabaseSettings",
    "DriftSettings",
    "ExtractionSettings",
    "IngestionSettings",
    "NatsSettings",
    "ObservabilitySettings",
    "PiiLinterSettings",
    "RedisSettings",
    "RetentionSettings",
]

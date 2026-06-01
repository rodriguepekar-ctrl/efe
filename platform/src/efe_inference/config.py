"""Hierarchical configuration for :mod:`efe_inference`.

Precedence (highest → lowest):

  1. constructor ``overrides``
  2. env variables prefixed by ``EFE_INFERENCE_``
  3. built-in defaults

All sections are frozen Pydantic models with ``extra="forbid"`` so
typos hard-fail at boot.
"""

from __future__ import annotations

import os
from enum import Enum
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from efe_inference.errors import ConfigError

# ---------------------------------------------------------------------------
# Backend kinds + ModelFamily
# ---------------------------------------------------------------------------


class BackendKind(str, Enum):
    VLLM_LOCAL = "vllm_local"
    ANTHROPIC = "anthropic"
    OPENAI = "openai"
    MISTRAL = "mistral"
    PEFT_LOCAL = "peft_local"
    MOCK = "mock"


# ---------------------------------------------------------------------------
# Backend configuration (one entry per concrete backend deployed)
# ---------------------------------------------------------------------------


class BackendConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str = Field(min_length=1, max_length=64)
    kind: BackendKind
    model_family: str = Field(min_length=1)
    tokenizer: str = Field(min_length=1)
    endpoint_url: str | None = None
    api_key_env: str | None = None
    priority: int = Field(default=100, ge=0, le=10_000)
    max_context_tokens: int = Field(default=8192, ge=128, le=2_097_152)
    cost_per_1k_prompt_usd: float = Field(default=0.0, ge=0.0)
    cost_per_1k_completion_usd: float = Field(default=0.0, ge=0.0)
    rate_limit_rpm: int | None = Field(default=None, ge=1)
    enabled: bool = True
    extra: dict[str, str] = Field(default_factory=dict)


# ---------------------------------------------------------------------------
# Database
# ---------------------------------------------------------------------------


class DatabaseSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = Field(
        default="sqlite+aiosqlite:///./efe_inference.sqlite",
        description="Async SQLAlchemy URL. Prod: postgresql+asyncpg://...",
    )
    pool_size: int = Field(default=10, ge=1, le=200)
    pool_max_overflow: int = Field(default=10, ge=0, le=200)
    pool_timeout_s: float = Field(default=5.0, ge=0.1, le=60.0)
    echo: bool = False


# ---------------------------------------------------------------------------
# Redis
# ---------------------------------------------------------------------------


class RedisSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = "redis://localhost:6379/0"
    verbatim_ttl_s: int = Field(default=3600, ge=0, le=86_400)
    semantic_ttl_s: int = Field(default=3600, ge=0, le=86_400)


# ---------------------------------------------------------------------------
# Safety
# ---------------------------------------------------------------------------


class PiiSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    engine: Literal["presidio", "regex_only"] = "regex_only"
    categories: tuple[str, ...] = (
        "email", "phone", "iban", "ssn", "credit_card",
        "ip_address", "url",
    )
    mode: Literal["redact", "hash", "block"] = "redact"
    salt_hex: str = "00" * 16


class ModerationSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    providers: tuple[str, ...] = ("local_blocklist",)
    thresholds: dict[str, float] = Field(
        default_factory=lambda: {
            "hate": 0.9, "violence": 0.9, "sexual": 0.9,
            "self_harm": 0.9, "harassment": 0.9,
        },
    )
    block_on_exceed: bool = True


class InjectionSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    threshold: int = Field(default=80, ge=0, le=100)
    block: bool = True


class SafetySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    pii: PiiSettings = Field(default_factory=PiiSettings)
    moderation: ModerationSettings = Field(default_factory=ModerationSettings)
    injection: InjectionSettings = Field(default_factory=InjectionSettings)
    preflight: bool = True
    postflight: bool = True
    audit_hmac_key_hex: str = "00" * 32

    @field_validator("audit_hmac_key_hex")
    @classmethod
    def _check_hex(cls, v: str) -> str:
        if len(v) != 64:
            raise ValueError(
                f"audit_hmac_key_hex must be 64 hex chars, got {len(v)}",
            )
        try:
            bytes.fromhex(v)
        except ValueError as exc:
            raise ValueError(f"audit_hmac_key_hex not hex: {exc}") from exc
        return v


# ---------------------------------------------------------------------------
# Cache
# ---------------------------------------------------------------------------


class CacheSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    verbatim_enabled: bool = True
    semantic_enabled: bool = True
    semantic_cosine_threshold: float = Field(default=0.95, ge=0.5, le=1.0)
    semantic_max_entries_per_task: int = Field(
        default=100_000, ge=10, le=10_000_000,
    )
    embed_model: str = "text-embedding-3-small"
    embed_dim: int = Field(default=1536, ge=64, le=8192)


# ---------------------------------------------------------------------------
# Resilience
# ---------------------------------------------------------------------------


class RetrySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    max_attempts: int = Field(default=3, ge=1, le=10)
    backoff_base_s: float = Field(default=0.5, ge=0.0001, le=30.0)
    backoff_max_s: float = Field(default=5.0, ge=0.001, le=60.0)
    jitter_factor: float = Field(default=0.2, ge=0.0, le=1.0)


class CircuitSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    failure_threshold: int = Field(default=5, ge=1, le=100)
    window_s: int = Field(default=60, ge=5, le=3600)
    success_threshold: int = Field(default=1, ge=1, le=100)
    cool_down_s: int = Field(default=30, ge=1, le=3600)


class TimeoutSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    connect_s: float = Field(default=5.0, ge=0.1, le=60.0)
    first_byte_s: float = Field(default=15.0, ge=0.1, le=600.0)
    inter_token_s: float = Field(default=10.0, ge=0.1, le=600.0)
    total_s: float = Field(default=60.0, ge=1.0, le=1800.0)


class ResilienceSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    retry: RetrySettings = Field(default_factory=RetrySettings)
    circuit: CircuitSettings = Field(default_factory=CircuitSettings)
    timeout: TimeoutSettings = Field(default_factory=TimeoutSettings)
    dlq_enabled: bool = True
    dlq_max_entries: int = Field(default=10_000, ge=10, le=10_000_000)


# ---------------------------------------------------------------------------
# Accounting
# ---------------------------------------------------------------------------


class AccountingSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    enforce_budget: bool = True
    default_daily_budget_usd: float = Field(
        default=100.0, ge=0.0, le=1_000_000.0,
    )
    reconcile_tolerance_pct: float = Field(
        default=1.0, ge=0.0, le=100.0,
    )


# ---------------------------------------------------------------------------
# API
# ---------------------------------------------------------------------------


class ApiSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    host: str = "0.0.0.0"
    port: int = Field(default=8087, ge=1, le=65_535)
    request_timeout_s: float = Field(default=60.0, ge=0.1, le=1800.0)
    enable_openai_shim: bool = True
    enable_anthropic_shim: bool = True
    enable_native_shim: bool = True
    max_request_body_bytes: int = Field(
        default=16_777_216, ge=1024, le=268_435_456,
    )


# ---------------------------------------------------------------------------
# NATS
# ---------------------------------------------------------------------------


class NatsSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = "nats://localhost:4222"
    topic_prefix: str = "efe.inference"
    schema_version: str = "1.0"
    hmac_key_hex: str = "00" * 32

    @field_validator("hmac_key_hex")
    @classmethod
    def _check_hex(cls, v: str) -> str:
        if len(v) != 64:
            raise ValueError("hmac_key_hex must be 64 hex chars")
        bytes.fromhex(v)
        return v


# ---------------------------------------------------------------------------
# Observability
# ---------------------------------------------------------------------------


class ObservabilitySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    service_name: str = "efe-inference"
    metrics_namespace: str = "efe_inference"
    log_level: Literal["DEBUG", "INFO", "WARNING", "ERROR"] = "INFO"
    log_json: bool = True
    otel_sampling_ratio: float = Field(default=0.05, ge=0.0, le=1.0)
    log_sampling_ratio: float = Field(default=1.0, ge=0.0, le=1.0)


# ---------------------------------------------------------------------------
# Top-level
# ---------------------------------------------------------------------------


class InferenceSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    environment: Literal["dev", "staging", "prod", "test"] = "dev"
    backends: tuple[BackendConfig, ...] = Field(default_factory=tuple)
    fallback_chain: tuple[str, ...] = Field(default_factory=tuple)
    database: DatabaseSettings = Field(default_factory=DatabaseSettings)
    redis: RedisSettings = Field(default_factory=RedisSettings)
    safety: SafetySettings = Field(default_factory=SafetySettings)
    cache: CacheSettings = Field(default_factory=CacheSettings)
    resilience: ResilienceSettings = Field(default_factory=ResilienceSettings)
    accounting: AccountingSettings = Field(default_factory=AccountingSettings)
    api: ApiSettings = Field(default_factory=ApiSettings)
    nats: NatsSettings = Field(default_factory=NatsSettings)
    observability: ObservabilitySettings = Field(
        default_factory=ObservabilitySettings,
    )

    @classmethod
    def from_env(
        cls, overrides: dict[str, Any] | None = None,
    ) -> InferenceSettings:
        data = _from_environ("EFE_INFERENCE_")
        if overrides:
            data = _merge(data, overrides)
        try:
            return cls.model_validate(data)
        except Exception as exc:
            raise ConfigError(
                f"invalid efe-inference configuration: {exc}",
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
    "AccountingSettings",
    "ApiSettings",
    "BackendConfig",
    "BackendKind",
    "CacheSettings",
    "CircuitSettings",
    "DatabaseSettings",
    "InferenceSettings",
    "InjectionSettings",
    "ModerationSettings",
    "NatsSettings",
    "ObservabilitySettings",
    "PiiSettings",
    "RedisSettings",
    "ResilienceSettings",
    "RetrySettings",
    "SafetySettings",
    "TimeoutSettings",
]

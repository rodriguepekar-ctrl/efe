"""Hierarchical configuration for :mod:`efe_registry`.

Precedence (highest → lowest):

  1. constructor ``overrides``
  2. env variables prefixed by ``EFE_REGISTRY_``
  3. built-in defaults

Every section is a frozen pydantic model with ``extra="forbid"``.
"""

from __future__ import annotations

import os
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from efe_registry.errors import ConfigError

# ---------------------------------------------------------------------------
# Storage
# ---------------------------------------------------------------------------


class DatabaseSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = Field(
        default="sqlite+aiosqlite:///./efe_registry.sqlite",
        description="Async SQLAlchemy URL. Prod: postgresql+asyncpg://...",
    )
    pool_size: int = Field(default=20, ge=1, le=200)
    pool_max_overflow: int = Field(default=10, ge=0, le=200)
    pool_timeout_s: float = Field(default=5.0, ge=0.1, le=60.0)
    echo: bool = False


class S3Settings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    bucket_pattern: str = "efe-artifacts-{tenant}"
    object_lock_days: int = Field(default=7, ge=1, le=365)
    region: str = "eu-west-1"
    endpoint_url: str | None = None  # for MinIO / LocalStack in tests


class RedisSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = "redis://localhost:6379/0"
    pubsub_channel: str = "routing.updated"
    lua_sha_swap: str | None = None  # populated at boot after SCRIPT LOAD
    lua_sha_quota: str | None = None


# ---------------------------------------------------------------------------
# Signing
# ---------------------------------------------------------------------------


class SigningSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    # Primary KMS key ARN for key-based signing (cosign)
    kms_key_arn: str | None = Field(
        default=None,
        description="e.g. 'awskms:///alias/efe-signing-prod'.",
    )
    # Overlap rotation — secondary KMS key accepted during rotation window
    kms_key_arn_secondary: str | None = None
    # Keyless (OIDC) identity — used in CI builds
    keyless_issuer: str | None = Field(
        default=None,
        description="e.g. 'https://token.actions.githubusercontent.com'.",
    )
    keyless_cert_identity: str | None = None
    rekor_url: str = "https://rekor.sigstore.dev"
    # CLI config
    cosign_path: str = "cosign"
    cli_timeout_s: float = Field(default=15.0, ge=1.0, le=120.0)
    # Cache
    verify_cache_ttl_s: int = Field(default=60, ge=1, le=3600)
    # Required attestations on verify_image
    require_sbom: bool = True
    require_slsa: bool = True
    # Inbound NATS message integrity (HMAC on payload)
    nats_hmac_key_hex: str = Field(default="0" * 64)

    @field_validator("nats_hmac_key_hex")
    @classmethod
    def _check_hex(cls, v: str) -> str:
        if len(v) != 64:
            raise ValueError(f"nats_hmac_key_hex must be 64 hex chars, got {len(v)}")
        try:
            bytes.fromhex(v)
        except ValueError as exc:
            raise ValueError(f"nats_hmac_key_hex not hex: {exc}") from exc
        return v


# ---------------------------------------------------------------------------
# Serving (vLLM)
# ---------------------------------------------------------------------------


class VllmPoolSpec(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    tensor_parallel: Literal[1, 2, 4, 8] = 1
    max_model_len: int = Field(default=8192, ge=128, le=1_048_576)
    dtype: Literal["bfloat16", "float16", "float32"] = "bfloat16"
    gpu_memory_utilization: float = Field(default=0.9, ge=0.1, le=0.99)
    enable_lora: bool = False
    max_loras: int = Field(default=0, ge=0, le=64)
    max_lora_rank: int = Field(default=0, ge=0, le=256)


class ServingSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    pools: tuple[VllmPoolSpec, ...] = Field(
        default_factory=lambda: (
            VllmPoolSpec(
                name="gpu_a100_shared",
                tensor_parallel=1,
                max_model_len=8192,
            ),
        ),
    )
    local_root: str = "/var/efe/models"
    download_timeout_s: float = Field(default=600.0, ge=1.0, le=7200.0)
    standby_warm_enabled: bool = True
    standby_ping_interval_s: int = Field(default=60, ge=5, le=600)


# ---------------------------------------------------------------------------
# Routing
# ---------------------------------------------------------------------------


class RouterSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    cache_ttl_s: float = Field(default=1.0, ge=0.0, le=60.0)
    request_timeout_s: float = Field(default=30.0, ge=0.1, le=600.0)
    quota_default_capacity: float = Field(default=100.0, ge=1.0)
    quota_default_refill_per_s: float = Field(default=10.0, ge=0.0)
    quota_default_cost: float = Field(default=1.0, ge=0.0)


# ---------------------------------------------------------------------------
# Canary
# ---------------------------------------------------------------------------


class CanarySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    schedule_kind: Literal["geometric", "linear", "manual"] = "geometric"
    step_duration_s: int = Field(default=600, ge=30, le=86_400)
    auto_rollback: bool = True
    auto_advance: bool = True
    slo_breach_abort_threshold: int = Field(default=3, ge=1, le=20)
    tick_period_s: int = Field(default=30, ge=1, le=600)
    leader_lock_ttl_s: int = Field(default=30, ge=5, le=600)


# ---------------------------------------------------------------------------
# Rollback
# ---------------------------------------------------------------------------


class RollbackSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    hard_deadline_s: float = Field(default=60.0, ge=5.0, le=600.0)
    drain_deadline_s: float = Field(default=30.0, ge=1.0, le=300.0)
    swap_timeout_s: float = Field(default=2.0, ge=0.1, le=10.0)
    verify_deadline_s: float = Field(default=5.0, ge=0.5, le=30.0)
    verify_inflight_share_threshold: float = Field(
        default=0.99, ge=0.5, le=1.0,
    )
    idempotency_window_s: int = Field(default=60, ge=1, le=3600)


# ---------------------------------------------------------------------------
# Admission
# ---------------------------------------------------------------------------


class AdmissionSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    enabled: bool = True
    mode: Literal["enforce", "warn", "off"] = "enforce"
    break_glass_ttl_s: int = Field(default=1800, ge=60, le=86_400)
    image_glob: str = "efe-registry.io/models/**"


# ---------------------------------------------------------------------------
# NATS
# ---------------------------------------------------------------------------


class NatsSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = "nats://localhost:4222"
    topic_prefix: str = "efe"
    consumer_durable_name: str = "efe-registry-consumer"
    message_schema_version: str = "1.0"


# ---------------------------------------------------------------------------
# Contracts
# ---------------------------------------------------------------------------


class ContractSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    deprecation_grace_days: int = Field(default=90, ge=1, le=365)
    alert_days_before_sunset: tuple[int, ...] = (30, 7, 1)


# ---------------------------------------------------------------------------
# API
# ---------------------------------------------------------------------------


class ApiSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    host: str = "0.0.0.0"
    port: int = Field(default=8086, ge=1, le=65_535)
    request_timeout_s: float = Field(default=30.0, ge=0.1, le=600.0)
    require_mfa_rollback: bool = True


# ---------------------------------------------------------------------------
# Observability
# ---------------------------------------------------------------------------


class ObservabilitySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    service_name: str = "efe-registry"
    metrics_namespace: str = "efe_registry"
    enable_prometheus: bool = True
    log_level: Literal["DEBUG", "INFO", "WARNING", "ERROR"] = "INFO"
    log_json: bool = True
    otel_endpoint: str | None = None
    otel_sampling_ratio: float = Field(default=0.1, ge=0.0, le=1.0)


# ---------------------------------------------------------------------------
# Top-level
# ---------------------------------------------------------------------------


class RegistrySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    environment: Literal["dev", "staging", "prod", "test"] = "dev"
    database: DatabaseSettings = Field(default_factory=DatabaseSettings)
    s3: S3Settings = Field(default_factory=S3Settings)
    redis: RedisSettings = Field(default_factory=RedisSettings)
    signing: SigningSettings = Field(default_factory=SigningSettings)
    serving: ServingSettings = Field(default_factory=ServingSettings)
    router: RouterSettings = Field(default_factory=RouterSettings)
    canary: CanarySettings = Field(default_factory=CanarySettings)
    rollback: RollbackSettings = Field(default_factory=RollbackSettings)
    admission: AdmissionSettings = Field(default_factory=AdmissionSettings)
    nats: NatsSettings = Field(default_factory=NatsSettings)
    contracts: ContractSettings = Field(default_factory=ContractSettings)
    api: ApiSettings = Field(default_factory=ApiSettings)
    observability: ObservabilitySettings = Field(
        default_factory=ObservabilitySettings,
    )

    @classmethod
    def from_env(
        cls, overrides: dict[str, Any] | None = None,
    ) -> RegistrySettings:
        data = _from_environ("EFE_REGISTRY_")
        if overrides:
            data = _merge(data, overrides)
        try:
            return cls.model_validate(data)
        except Exception as exc:
            raise ConfigError(
                f"invalid efe-registry configuration: {exc}",
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
    "AdmissionSettings",
    "ApiSettings",
    "CanarySettings",
    "ContractSettings",
    "DatabaseSettings",
    "NatsSettings",
    "ObservabilitySettings",
    "RedisSettings",
    "RegistrySettings",
    "RollbackSettings",
    "RouterSettings",
    "S3Settings",
    "ServingSettings",
    "SigningSettings",
    "VllmPoolSpec",
]

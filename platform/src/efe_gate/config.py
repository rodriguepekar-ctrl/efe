"""Hierarchical configuration for :mod:`efe_gate`.

Precedence (highest → lowest):
  1. constructor ``overrides``
  2. env variables prefixed by ``EFE_GATE_``
  3. built-in defaults

Every section is a frozen pydantic model with ``extra="forbid"`` so
typos hard-error at boot rather than silently diverge at runtime.
"""

from __future__ import annotations

import os
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from efe_gate.errors import ConfigError


class DatabaseSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = Field(
        default="sqlite+aiosqlite:///./efe_gate.sqlite",
        description=(
            "Async SQLAlchemy URL. Prod: ``postgresql+asyncpg://...``."
        ),
    )
    pool_size: int = Field(default=20, ge=1, le=200)
    pool_max_overflow: int = Field(default=10, ge=0, le=200)
    pool_timeout_s: float = Field(default=5.0, ge=0.1, le=60.0)
    echo: bool = False
    enable_rls: bool = True


class ApiSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    host: str = "0.0.0.0"
    port: int = Field(default=8085, ge=1, le=65_535)
    request_timeout_s: float = Field(default=30.0, ge=0.1, le=600.0)


class StatsSettings(BaseModel):
    """Defaults for statistical tests."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    default_ci: float = Field(default=0.95, ge=0.5, le=0.999)
    bootstrap_resamples: int = Field(default=10_000, ge=200, le=200_000)
    rng_seed: int = Field(default=1337)
    min_sample_size: int = Field(default=200, ge=1)


class DecisionSettings(BaseModel):
    """Defaults for the decision engine."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    default_equivalence_margin: float = Field(
        default=0.01, ge=0.0, le=0.5,
        description=(
            "Default δ for TOST when a rule does not override it; in "
            "absolute metric units."
        ),
    )
    significance_level: float = Field(default=0.05, ge=1e-5, le=0.5)
    fail_fast_on_budget: bool = True


class RollbackSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    window_minutes: int = Field(default=60, ge=1, le=24 * 60)
    tick_seconds: int = Field(default=300, ge=5, le=3600)
    min_live_samples: int = Field(default=500, ge=1)
    auto_rollback_threshold_p_value: float = Field(
        default=0.95, ge=0.5, le=0.999,
    )
    auto_rollback_min_delta: float = Field(
        default=0.02, ge=0.0, le=0.5,
    )


class ReviewSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    default_timeout_hours: int = Field(default=24, ge=1, le=168)
    require_dual_signoff_prod: bool = True


class SignatureSettings(BaseModel):
    """HMAC signing keys (MVP; Ed25519 in phase 2).

    A single key is used per environment; rotation is done by adding
    a new key with a fresh ``key_id`` and migrating clients to verify
    against the new one while keeping the old in the verification
    set for a grace period.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    key_id: str = Field(default="dev-key-1")
    key_hex: str = Field(
        default=(
            # Deterministic 32-byte dev key — replaced via env in prod.
            "0" * 64
        ),
        description="64-char hex-encoded 32-byte HMAC-SHA256 key.",
    )
    algorithm: Literal["hmac-sha256", "ed25519"] = "hmac-sha256"

    @field_validator("key_hex")
    @classmethod
    def _check_hex(cls, v: str) -> str:
        if len(v) != 64:
            raise ValueError(f"key_hex must be 64 hex chars, got {len(v)}")
        try:
            bytes.fromhex(v)
        except ValueError as exc:
            raise ValueError(f"key_hex must be valid hex: {exc}") from exc
        return v


class ObservabilitySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    service_name: str = "efe-eval-gate"
    metrics_namespace: str = "efe_gate"
    enable_prometheus: bool = True
    log_level: Literal["DEBUG", "INFO", "WARNING", "ERROR"] = "INFO"
    log_json: bool = True


class RunnerSettings(BaseModel):
    """Eval job runner knobs."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    max_concurrent_runs: int = Field(default=8, ge=1, le=256)
    per_run_timeout_s: int = Field(default=1_800, ge=60, le=14_400)
    scratch_dir: str = "/var/efe/scratch/gate"


class GateSettings(BaseModel):
    """Top-level configuration bundle for M05."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    environment: Literal["dev", "staging", "prod", "test"] = "dev"
    database: DatabaseSettings = Field(default_factory=DatabaseSettings)
    api: ApiSettings = Field(default_factory=ApiSettings)
    stats: StatsSettings = Field(default_factory=StatsSettings)
    decision: DecisionSettings = Field(default_factory=DecisionSettings)
    rollback: RollbackSettings = Field(default_factory=RollbackSettings)
    review: ReviewSettings = Field(default_factory=ReviewSettings)
    signature: SignatureSettings = Field(default_factory=SignatureSettings)
    runner: RunnerSettings = Field(default_factory=RunnerSettings)
    observability: ObservabilitySettings = Field(
        default_factory=ObservabilitySettings,
    )

    @classmethod
    def from_env(
        cls, overrides: dict[str, Any] | None = None,
    ) -> GateSettings:
        data = _from_environ("EFE_GATE_")
        if overrides:
            data = _merge(data, overrides)
        try:
            return cls.model_validate(data)
        except Exception as exc:
            raise ConfigError(
                f"invalid efe-gate configuration: {exc}",
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
    "ApiSettings",
    "DatabaseSettings",
    "DecisionSettings",
    "GateSettings",
    "ObservabilitySettings",
    "ReviewSettings",
    "RollbackSettings",
    "RunnerSettings",
    "SignatureSettings",
    "StatsSettings",
]

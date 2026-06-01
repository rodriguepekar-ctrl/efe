"""Hierarchical configuration for :mod:`efe_platform`.

Precedence (highest to lowest):

1. constructor kwargs (explicit overrides in code),
2. environment variables prefixed by ``EFE_PLATFORM_``,
3. a TOML file pointed at by ``EFE_PLATFORM_CONFIG``,
4. built-in defaults.

The whole tree is a ``pydantic.BaseModel``, so everything is validated
at construction time. Config objects are frozen — mutation is a bug.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Literal, cast

import tomllib
from pydantic import BaseModel, ConfigDict, Field, field_validator

from efe_platform.errors import ConfigError

# ---------------------------------------------------------------------------
#  Sub-sections
# ---------------------------------------------------------------------------


class SwarmSettings(BaseModel):
    """How to reach the Rust efe-server."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    endpoint: str = Field(
        default="localhost:50051",
        description="host:port of the SwarmService gRPC endpoint.",
    )
    admin_endpoint: str | None = Field(
        default=None,
        description="host:port of the AdminService, if different from data plane.",
    )
    tls: bool = Field(default=False, description="Use TLS to talk to the server.")
    tls_ca_path: str | None = Field(
        default=None,
        description="Path to the CA bundle (PEM) used when TLS is enabled.",
    )
    default_deadline_ms: int = Field(
        default=5_000,
        ge=1,
        le=120_000,
        description="Default wall-clock deadline applied to Execute RPCs.",
    )
    max_retries: int = Field(
        default=3,
        ge=0,
        le=10,
        description="How many retry attempts for retryable gRPC errors.",
    )
    jwt: str | None = Field(
        default=None,
        description=(
            "JWT bearer forwarded in the ``authorization`` metadata. "
            "Leave unset in dev/mTLS deployments."
        ),
    )


class LlmSettings(BaseModel):
    """Choice of LLM backend + knobs."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    provider: Literal["claude", "openai", "mock"] = Field(
        default="mock",
        description=(
            "Which LLM backend to talk to. 'mock' is the test-only "
            "deterministic provider — never pick it in prod."
        ),
    )
    model: str = Field(
        default="claude-3-5-sonnet-latest",
        description="Model identifier passed to the provider SDK.",
    )
    max_tokens_per_turn: int = Field(
        default=1_024,
        ge=64,
        le=32_768,
    )
    temperature: float = Field(default=0.1, ge=0.0, le=2.0)
    api_key_env: str = Field(
        default="ANTHROPIC_API_KEY",
        description="Name of the environment variable carrying the provider key.",
    )


class BudgetSettings(BaseModel):
    """Per-request hard ceilings."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    max_wall_clock_ms: int = Field(default=30_000, ge=100)
    max_llm_tokens: int = Field(default=16_000, ge=256)
    max_usd: float = Field(default=0.25, ge=0.0)
    max_tool_calls: int = Field(default=8, ge=1, le=64)


class AuditSettings(BaseModel):
    """Where and how to write audit events."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    enabled: bool = True
    path: str = Field(
        default="./audit.jsonl",
        description="Absolute or relative path to the JSONL sink.",
    )
    redact_prompts: bool = Field(
        default=False,
        description="Replace raw prompts by SHA-256 hashes in audit entries.",
    )
    fsync_each: bool = Field(
        default=True,
        description="Call fsync() after each append — safer, slower.",
    )


class TelemetrySettings(BaseModel):
    """OTel + Prometheus."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    otel_endpoint: str | None = Field(
        default=None, description="OTLP gRPC endpoint (e.g. ``localhost:4317``)."
    )
    otel_service_name: str = Field(default="efe-platform")
    prometheus_enabled: bool = True


class ApiSettings(BaseModel):
    """HTTP surface."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    host: str = Field(default="0.0.0.0")
    port: int = Field(default=8080, ge=1, le=65_535)
    cors_origins: tuple[str, ...] = Field(default_factory=tuple)


# ---------------------------------------------------------------------------
#  Root
# ---------------------------------------------------------------------------


class PlatformConfig(BaseModel):
    """Top-level frozen configuration passed around the platform."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    tenant_default: str = Field(
        default="default",
        description="Tenant label used when a caller doesn't specify one.",
    )
    swarm: SwarmSettings = Field(default_factory=SwarmSettings)
    llm: LlmSettings = Field(default_factory=LlmSettings)
    budget: BudgetSettings = Field(default_factory=BudgetSettings)
    audit: AuditSettings = Field(default_factory=AuditSettings)
    telemetry: TelemetrySettings = Field(default_factory=TelemetrySettings)
    api: ApiSettings = Field(default_factory=ApiSettings)

    @field_validator("tenant_default")
    @classmethod
    def _tenant_not_empty(cls, v: str) -> str:
        if not v.strip():
            raise ValueError("tenant_default must not be empty")
        return v

    # ------------------------------------------------------------ Loaders

    @classmethod
    def from_env(cls, *, overrides: dict[str, Any] | None = None) -> PlatformConfig:
        """Build a config from env + optional TOML + explicit overrides.

        The merging order (low→high) is: defaults, TOML, env, overrides.
        Env variables use double-underscore to descend into sections:
        ``EFE_PLATFORM_SWARM__ENDPOINT=host:50051``.
        """
        data: dict[str, Any] = {}

        toml_path = os.environ.get("EFE_PLATFORM_CONFIG")
        if toml_path:
            data.update(_load_toml(Path(toml_path)))

        data = _merge(data, _from_environ("EFE_PLATFORM_"))

        if overrides:
            data = _merge(data, overrides)

        try:
            return cls.model_validate(data)
        except Exception as exc:
            raise ConfigError(f"invalid platform configuration: {exc}") from exc


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _load_toml(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise ConfigError(f"TOML config not found: {path}")
    with path.open("rb") as fh:
        return cast(dict[str, Any], tomllib.load(fh))


def _merge(a: dict[str, Any], b: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = dict(a)
    for k, v in b.items():
        existing = out.get(k)
        if isinstance(v, dict) and isinstance(existing, dict):
            out[k] = _merge(existing, v)
        else:
            out[k] = v
    return out


# Env variables we handle separately from the generic config tree — they
# must be excluded from the nested parsing or they'd be mistaken for
# regular config fields.
_RESERVED_ENV_KEYS: frozenset[str] = frozenset({"CONFIG"})


def _from_environ(prefix: str) -> dict[str, Any]:
    """Parse ``EFE_PLATFORM_<SECTION>__<KEY>=value`` into a nested dict.

    Splits on the double underscore. Leaf values are decoded as:
    ``true/false`` → bool, digits → int/float, else → str.

    Reserved keys (:data:`_RESERVED_ENV_KEYS`) are excluded — they carry
    loader-level meta like the path to a TOML file rather than config
    values the pydantic model should ingest.
    """
    out: dict[str, Any] = {}
    for raw_key, raw_val in os.environ.items():
        if not raw_key.startswith(prefix):
            continue
        tail = raw_key[len(prefix) :]
        if tail in _RESERVED_ENV_KEYS:
            continue
        key = tail.lower()
        parts = key.split("__")
        cursor = out
        for head in parts[:-1]:
            nxt = cursor.setdefault(head, {})
            if not isinstance(nxt, dict):
                # Collision: refuse silent overwrite.
                raise ConfigError(f"env conflict at {raw_key}: nested dict expected")
            cursor = nxt
        cursor[parts[-1]] = _coerce(raw_val)
    return out


def _coerce(v: str) -> Any:
    low = v.strip().lower()
    if low in {"true", "yes", "on"}:
        return True
    if low in {"false", "no", "off"}:
        return False
    # int?
    try:
        return int(v)
    except ValueError:
        pass
    # float?
    try:
        return float(v)
    except ValueError:
        pass
    return v


__all__ = [
    "ApiSettings",
    "AuditSettings",
    "BudgetSettings",
    "LlmSettings",
    "PlatformConfig",
    "SwarmSettings",
    "TelemetrySettings",
]

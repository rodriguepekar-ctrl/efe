"""Hierarchical configuration for :mod:`efe_ft`.

Precedence (highest → lowest):
  1. constructor kwargs
  2. env variables prefixed by ``EFE_FT_``
  3. built-in defaults

Uses pydantic v2 ``BaseModel`` + ``field_validator`` — no external
``pydantic-settings`` dep so the package stays lean.
"""

from __future__ import annotations

import os
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from efe_ft.errors import ConfigError


class DatabaseSettings(BaseModel):
    """Postgres connection settings."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = Field(
        default="sqlite+aiosqlite:///./efe_ft.sqlite",
        description=(
            "Async SQLAlchemy URL. Defaults to local SQLite for dev; "
            "use ``postgresql+asyncpg://...`` in production."
        ),
    )
    pool_size: int = Field(default=20, ge=1, le=200)
    pool_max_overflow: int = Field(default=10, ge=0, le=200)
    pool_timeout_s: float = Field(default=5.0, ge=0.1, le=60.0)
    echo: bool = False
    enable_rls: bool = Field(
        default=True,
        description=(
            "Apply ``SET app.tenant_id`` per-session. Disabled in "
            "unit tests that bypass the API layer."
        ),
    )


class StorageSettings(BaseModel):
    """S3 / object store settings."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    backend: Literal["local", "s3"] = "local"
    local_root: str = Field(
        default="./blob_store",
        description="Root dir when backend='local' (tests, dev).",
    )
    s3_bucket: str | None = None
    s3_region: str = "eu-west-3"
    s3_prefix: str = "efe-ft"


class IngestSettings(BaseModel):
    """Knobs for the 8-stage ingestion pipeline."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    max_text_bytes: int = Field(default=16_384, ge=16)
    minhash_num_perm: int = Field(default=128, ge=8, le=512)
    minhash_threshold: float = Field(default=0.90, ge=0.5, le=1.0)
    balance_skew_max: float = Field(default=50.0, ge=1.0)
    pii_policy_default: Literal["mask", "reject"] = "mask"
    rate_limit_per_tenant_per_sec: int = Field(default=500, ge=1)
    quota_samples_per_month: int = Field(default=1_000_000, ge=1)


class EmbedderSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    backend: Literal["mock", "sentence_transformers"] = "mock"
    model: str = "all-MiniLM-L6-v2"
    version: str = "mock-ff01a2"
    dim: int = Field(default=128, ge=8, le=4096)
    batch_size: int = Field(default=32, ge=1, le=1024)


class RedisSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = "redis://localhost:6379/0"
    use_fake: bool = Field(
        default=False,
        description="Use fakeredis (for tests). Never in prod.",
    )


class LabelingSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    session_ttl_s: int = Field(default=1_800, ge=60)
    al_alpha_default: float = Field(default=0.7, ge=0.0, le=1.0)
    al_batch_size_default: int = Field(default=10, ge=1, le=100)
    al_cooldown_s: int = Field(default=1_800, ge=0)
    dual_label_quota: float = Field(
        default=0.05, ge=0.0, le=1.0,
        description="Fraction of samples routed for 2-annotator review.",
    )
    kappa_alert_threshold: float = Field(default=0.70, ge=0.0, le=1.0)
    kappa_window: int = Field(default=50, ge=10, le=10_000)


class FtSettings(BaseModel):
    """Top-level settings — wrap all section settings."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    environment: Literal["dev", "staging", "prod", "test"] = "dev"
    database: DatabaseSettings = Field(default_factory=DatabaseSettings)
    storage: StorageSettings = Field(default_factory=StorageSettings)
    ingest: IngestSettings = Field(default_factory=IngestSettings)
    embedder: EmbedderSettings = Field(default_factory=EmbedderSettings)
    redis: RedisSettings = Field(default_factory=RedisSettings)
    labeling: LabelingSettings = Field(default_factory=LabelingSettings)

    @field_validator("environment")
    @classmethod
    def _env_nonempty(cls, v: str) -> str:
        return v.strip()

    # -------------------------------------------------- Loader

    @classmethod
    def from_env(cls, overrides: dict[str, Any] | None = None) -> FtSettings:
        data = _from_environ("EFE_FT_")
        if overrides:
            data = _merge(data, overrides)
        try:
            return cls.model_validate(data)
        except Exception as exc:
            raise ConfigError(f"invalid efe-ft configuration: {exc}") from exc


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _merge(a: dict[str, Any], b: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = dict(a)
    for k, v in b.items():
        existing = out.get(k)
        if isinstance(v, dict) and isinstance(existing, dict):
            out[k] = _merge(existing, v)
        else:
            out[k] = v
    return out


def _from_environ(prefix: str) -> dict[str, Any]:
    """Parse ``EFE_FT_<SECTION>__<KEY>=value`` into a nested dict."""
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
    "DatabaseSettings",
    "EmbedderSettings",
    "FtSettings",
    "IngestSettings",
    "LabelingSettings",
    "RedisSettings",
    "StorageSettings",
]

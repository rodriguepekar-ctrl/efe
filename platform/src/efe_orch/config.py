"""Hierarchical configuration for :mod:`efe_orch`.

Precedence (highest → lowest):
  1. constructor kwargs (``overrides=``)
  2. env variables prefixed by ``EFE_ORCH_``
  3. built-in defaults in this module

We intentionally do **not** depend on ``pydantic-settings`` — the
``_from_environ`` walker below is tiny (~30 lines) and gives us:

* ``EFE_ORCH_DATABASE__URL=sqlite://...`` — nested section access
  via ``__`` (double underscore) as the separator,
* scalar-vs-dict conflict detection (``EFE_ORCH_DATABASE=foo`` *and*
  ``EFE_ORCH_DATABASE__URL=bar`` is rejected with a clear error),
* boolean / int / float coercion with obvious rules.

Every settings subclass is a frozen pydantic model with ``extra="forbid"``
so a typo is a hard error at boot rather than a silent miss.
"""

from __future__ import annotations

import os
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from efe_orch.errors import ConfigError


class DatabaseSettings(BaseModel):
    """Async SQLAlchemy connection settings for the orchestrator DB.

    The orchestrator uses its own schema (5 tables, see
    :mod:`efe_orch.db.models`) — it *does not* share the ``training_samples``
    schema owned by M01/M02.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    url: str = Field(
        default="sqlite+aiosqlite:///./efe_orch.sqlite",
        description=(
            "Async SQLAlchemy URL. Use ``postgresql+asyncpg://...`` in "
            "production to get real advisory locks; SQLite falls back "
            "to an in-process mutex."
        ),
    )
    pool_size: int = Field(default=20, ge=1, le=200)
    pool_max_overflow: int = Field(default=10, ge=0, le=200)
    pool_timeout_s: float = Field(default=5.0, ge=0.1, le=60.0)
    echo: bool = False
    enable_rls: bool = True


class K8sSettings(BaseModel):
    """Cluster parameters used by the dispatcher."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    namespace: str = Field(
        default="efe-ft",
        description="Namespace where worker Jobs are created.",
    )
    service_account: str = Field(default="efe-worker")
    image: str = Field(
        default="registry.local/efe/worker:latest",
        description="Default worker image. May be overridden per-run.",
    )
    image_pull_policy: Literal["Always", "IfNotPresent", "Never"] = "IfNotPresent"
    backoff_limit: int = Field(default=0, ge=0, le=10)
    active_deadline_seconds: int = Field(default=14_400, ge=60)  # 4h hard cap
    ttl_seconds_after_finished: int = Field(default=604_800, ge=60)
    dry_run: bool = Field(
        default=True,
        description=(
            "When true the dispatcher never talks to a real API server; "
            "the fake client records manifests for inspection. Safe default "
            "so tests + local dev never need cluster creds."
        ),
    )
    network_policy: Literal["strict", "egress_only", "open"] = "strict"


class PoolSettings(BaseModel):
    """Static capacity declaration for one worker pool.

    Pools are declared in :class:`SchedulerSettings.pools`. The
    scheduler reads ``(dataset_size, tenant_sla)`` and picks the
    cheapest pool that can fit the run. Capacity is measured in
    concurrent runs, not CPU cores; the per-run resource request is
    pinned in :attr:`cpu_request` / :attr:`memory_gib` / :attr:`gpu`.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str = Field(min_length=1, max_length=64)
    label: str = Field(
        default="",
        description="Human-readable label, shown in metrics / logs.",
    )
    max_concurrent_runs: int = Field(default=4, ge=1, le=256)
    cpu_request: float = Field(default=4.0, ge=0.25, le=128.0)
    memory_gib: float = Field(default=16.0, ge=0.5, le=1024.0)
    gpu: str | None = Field(
        default=None,
        description=(
            "Kubernetes GPU resource key (e.g. ``nvidia.com/gpu``). "
            "``None`` means CPU-only."
        ),
    )
    gpu_count: int = Field(default=0, ge=0, le=8)
    max_dataset_rows: int = Field(default=1_000_000, ge=1)
    max_runtime_s: int = Field(default=10_800, ge=60)  # 3 hours
    cost_score: int = Field(
        default=1,
        ge=1,
        le=100,
        description=(
            "Relative cost of one run in this pool (1 = cheapest). The "
            "scheduler picks the pool with the *lowest* score that fits."
        ),
    )
    priority_classes: tuple[str, ...] = Field(
        default=("standard",),
        description="SLA classes this pool is allowed to serve.",
    )

    @model_validator(mode="after")
    def _gpu_consistency(self) -> PoolSettings:
        """Ensure ``gpu`` and ``gpu_count`` agree.

        A pool that declares a GPU resource must reserve at least one,
        and a pool without a GPU resource must have ``gpu_count = 0``.
        The scheduler relies on this invariant to filter GPU-needing
        runs; letting the two fields drift would cause silent mis-routing.
        """
        if self.gpu is not None and self.gpu_count < 1:
            raise ValueError(
                f"pool {self.name!r} declares gpu={self.gpu!r} but "
                f"gpu_count={self.gpu_count}; must be >= 1",
            )
        if self.gpu is None and self.gpu_count > 0:
            raise ValueError(
                f"pool {self.name!r} has gpu_count={self.gpu_count} "
                f"but no ``gpu`` resource key set",
            )
        return self


class SchedulerSettings(BaseModel):
    """Scheduler knobs + pool registry.

    The default pool list mirrors the architecture doc (§4.2):
    ``cpu-small`` for tiny prototypes, ``cpu-large`` for most batches,
    ``gpu-a10`` for moderate GPU needs, ``gpu-a100`` for heavy jobs.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    pools: tuple[PoolSettings, ...] = Field(
        default=(
            PoolSettings(
                name="cpu-small",
                label="CPU small (1C/4Gi)",
                max_concurrent_runs=8,
                cpu_request=1.0,
                memory_gib=4.0,
                max_dataset_rows=5_000,
                max_runtime_s=1_800,
                cost_score=1,
            ),
            PoolSettings(
                name="cpu-large",
                label="CPU large (8C/32Gi)",
                max_concurrent_runs=4,
                cpu_request=8.0,
                memory_gib=32.0,
                max_dataset_rows=200_000,
                max_runtime_s=7_200,
                cost_score=5,
            ),
            PoolSettings(
                name="gpu-a10",
                label="GPU A10 (4C/32Gi + 1x A10)",
                max_concurrent_runs=4,
                cpu_request=4.0,
                memory_gib=32.0,
                gpu="nvidia.com/gpu",
                gpu_count=1,
                max_dataset_rows=1_000_000,
                max_runtime_s=10_800,
                cost_score=20,
            ),
            PoolSettings(
                name="gpu-a100",
                label="GPU A100 (8C/80Gi + 1x A100)",
                max_concurrent_runs=2,
                cpu_request=8.0,
                memory_gib=80.0,
                gpu="nvidia.com/gpu",
                gpu_count=1,
                max_dataset_rows=10_000_000,
                max_runtime_s=14_400,
                cost_score=50,
                priority_classes=("standard", "premium"),
            ),
        ),
    )

    default_priority: Literal["standard", "premium"] = "standard"
    enable_gpu: bool = True

    @field_validator("pools")
    @classmethod
    def _unique_names(cls, v: tuple[PoolSettings, ...]) -> tuple[PoolSettings, ...]:
        names = [p.name for p in v]
        if len(names) != len(set(names)):
            raise ValueError("pool names must be unique")
        if not v:
            raise ValueError("at least one pool must be declared")
        return v


class RetrySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    max_attempts: int = Field(default=3, ge=1, le=10)
    base_backoff_s: float = Field(default=30.0, ge=0.1, le=3600.0)
    max_backoff_s: float = Field(default=1_800.0, ge=1.0, le=86400.0)
    jitter_ratio: float = Field(default=0.25, ge=0.0, le=1.0)


class HeartbeatSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    interval_s: int = Field(default=30, ge=5, le=600)
    miss_threshold: int = Field(
        default=3,
        ge=1,
        le=20,
        description=(
            "Consecutive missed heartbeats before a worker is declared "
            "dead. With interval_s=30, the default 3 misses = 90s."
        ),
    )
    reaper_scan_interval_s: int = Field(default=15, ge=1, le=300)
    reaper_batch_size: int = Field(default=100, ge=1, le=10_000)


class QuotaSettings(BaseModel):
    """Per-tenant quota defaults. Override per tenant in the DB."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    default_max_concurrent_runs: int = Field(default=4, ge=1, le=100)
    default_max_monthly_gpu_seconds: int = Field(default=3_600 * 24, ge=60)
    default_max_monthly_runs: int = Field(default=200, ge=1)


class ApiSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    host: str = "0.0.0.0"
    port: int = Field(default=8080, ge=1, le=65_535)
    request_timeout_s: float = Field(default=30.0, ge=0.1, le=600.0)
    max_logs_tail_bytes: int = Field(default=64 * 1024, ge=1024)


class ObservabilitySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    service_name: str = "efe-orchestrator"
    metrics_namespace: str = "efe_orch"
    enable_prometheus: bool = True
    enable_tracing: bool = False
    log_level: Literal["DEBUG", "INFO", "WARNING", "ERROR"] = "INFO"
    log_json: bool = True


class OrchSettings(BaseModel):
    """Top-level settings bundle for the orchestrator."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    environment: Literal["dev", "staging", "prod", "test"] = "dev"
    database: DatabaseSettings = Field(default_factory=DatabaseSettings)
    k8s: K8sSettings = Field(default_factory=K8sSettings)
    scheduler: SchedulerSettings = Field(default_factory=SchedulerSettings)
    retry: RetrySettings = Field(default_factory=RetrySettings)
    heartbeat: HeartbeatSettings = Field(default_factory=HeartbeatSettings)
    quota: QuotaSettings = Field(default_factory=QuotaSettings)
    api: ApiSettings = Field(default_factory=ApiSettings)
    observability: ObservabilitySettings = Field(default_factory=ObservabilitySettings)

    @field_validator("environment")
    @classmethod
    def _env_nonempty(cls, v: str) -> str:
        return v.strip()

    # -------------------------------------------------- Loader

    @classmethod
    def from_env(cls, overrides: dict[str, Any] | None = None) -> OrchSettings:
        data = _from_environ("EFE_ORCH_")
        if overrides:
            data = _merge(data, overrides)
        try:
            return cls.model_validate(data)
        except Exception as exc:
            raise ConfigError(f"invalid efe-orch configuration: {exc}") from exc


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
    """Parse ``EFE_ORCH_<SECTION>__<KEY>=value`` into a nested dict."""
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
    "HeartbeatSettings",
    "K8sSettings",
    "ObservabilitySettings",
    "OrchSettings",
    "PoolSettings",
    "QuotaSettings",
    "RetrySettings",
    "SchedulerSettings",
]

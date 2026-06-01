"""Configuration for :mod:`efe_worker`.

The worker is configured almost entirely from **environment variables**
set by M03 on pod boot (§17.2 of the architecture doc). Precedence:

    1. constructor ``overrides`` (tests)
    2. ``EFE_*`` env variables
    3. built-in defaults

Only one section is loaded from JSON rather than individual vars:
``hyperparameters`` (via ``EFE_HYPERPARAMETERS_JSON``) because the set
of keys is method-specific.

Every settings class is a frozen pydantic v2 model with
``extra="forbid"`` so that a misnamed key hard-errors at boot rather
than silently using a default.
"""

from __future__ import annotations

import json
import os
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from efe_worker.errors import ConfigError

MethodName = Literal["infonce", "triplet", "classif", "lora", "distill"]


class TransportSettings(BaseModel):
    """gRPC + S3 knobs."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    callback_url: str = Field(
        default="http://localhost:8080",
        description=(
            "Orchestrator endpoint. Set by M03 via EFE_CALLBACK_URL."
        ),
    )
    rpc_timeout_s: float = Field(default=5.0, ge=0.1, le=60.0)
    rpc_max_retries: int = Field(default=3, ge=0, le=10)
    heartbeat_interval_s: float = Field(default=30.0, ge=1.0, le=600.0)
    s3_endpoint: str | None = Field(
        default=None,
        description="Override for MinIO/LocalStack.",
    )
    s3_region: str = "eu-west-3"
    fake_transport: bool = Field(
        default=False,
        description="Tests only — swap all network IO for fakes.",
    )


class DataSettings(BaseModel):
    """Snapshot ingest + tokenizer knobs."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    cache_dir: str = Field(default="/var/efe/scratch")
    num_dataloader_workers: int = Field(default=4, ge=0, le=32)
    pin_memory: bool = True
    max_seq_len: int = Field(default=128, ge=8, le=8192)
    shuffle_seed: int = Field(default=42)
    verify_shard_sha256: bool = True


class CheckpointSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    local_dir: str = Field(default="/var/efe/scratch/ckpts")
    remote_uri_prefix: str = Field(
        default="s3://efe-ft/ckpts",
        description="Remote base for {remote_uri_prefix}/{run_id}/step-N.",
    )
    every_n_steps: int = Field(default=500, ge=1, le=100_000)
    keep_local: int = Field(default=3, ge=1, le=20)
    best_metric: str = Field(
        default="val/loss",
        description="Metric key used to mark a checkpoint as ``is_best``.",
    )
    best_metric_mode: Literal["min", "max"] = "min"


class TrainingSettings(BaseModel):
    """Outer-loop knobs (shared across all methods)."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    max_steps: int = Field(default=2_000, ge=1, le=1_000_000)
    batch_size: int = Field(default=32, ge=1, le=4096)
    eval_every_n_steps: int = Field(default=500, ge=1)
    learning_rate: float = Field(default=2e-5, gt=0.0, lt=1.0)
    weight_decay: float = Field(default=0.01, ge=0.0, le=1.0)
    warmup_ratio: float = Field(default=0.1, ge=0.0, le=1.0)
    grad_clip: float = Field(default=1.0, ge=0.0, le=10.0)
    early_stopping_patience: int = Field(default=3, ge=1, le=50)
    user_deadline_seconds: int | None = Field(default=None, ge=1)


class ObservabilitySettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    service_name: str = "efe-worker"
    metrics_namespace: str = "efe_worker"
    log_level: Literal["DEBUG", "INFO", "WARNING", "ERROR"] = "INFO"
    log_json: bool = True
    enable_prometheus: bool = False  # default off for tests
    enable_tracing: bool = False


class RunIdentity(BaseModel):
    """The (run_id, tenant_id, task_id, snapshot_id, worker_id) tuple.

    These fields are *mandatory* at runtime — the worker refuses to
    boot without them. Defaults are empty strings so the validator
    catches the omission cleanly.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    run_id: str = ""
    tenant_id: str = ""
    task_id: str = ""
    snapshot_id: str = ""
    worker_id: str = ""
    snapshot_uri: str = ""
    base_model: str = "sentence-transformers/all-MiniLM-L6-v2"
    recipe_version: str = "v1"
    pool_name: str = "cpu-small"

    @field_validator("run_id", "tenant_id", "task_id", "worker_id",
                      "snapshot_id")
    @classmethod
    def _nonempty(cls, v: str) -> str:
        if not v:
            raise ValueError("run identity field must be non-empty")
        return v


class WorkerSettings(BaseModel):
    """Top-level configuration bundle."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    environment: Literal["dev", "staging", "prod", "test"] = "dev"
    method_name: MethodName = "classif"
    hyperparameters: dict[str, Any] = Field(default_factory=dict)

    @field_validator("hyperparameters", mode="before")
    @classmethod
    def _sanitize_hyperparameters(cls, v: Any) -> Any:
        """Reject dunder and underscore-prefixed keys.

        Hyperparameters flow into ``method.__init__(hp)`` where a
        malicious ``__class__`` / ``__init__`` would be silently kept
        in the dict. We reject the whole dict at config-boot to fail
        loud rather than silently accept garbage. Keys must be
        non-empty strings with only ``[A-Za-z][A-Za-z0-9_]*`` shape.
        """
        if v is None:
            return {}
        if not isinstance(v, dict):
            raise ValueError("hyperparameters must be a dict")
        import re
        pat = re.compile(r"^[A-Za-z][A-Za-z0-9_]*$")
        for key in v:
            if not isinstance(key, str):
                raise ValueError(
                    f"hyperparameter key must be str, got {type(key).__name__}",
                )
            if key.startswith("_"):
                raise ValueError(
                    f"hyperparameter key {key!r} has a leading underscore "
                    f"(reserved for private attributes)",
                )
            if not pat.match(key):
                raise ValueError(
                    f"hyperparameter key {key!r} must match "
                    f"^[A-Za-z][A-Za-z0-9_]*$",
                )
        return v
    identity: RunIdentity = Field(default_factory=RunIdentity)
    transport: TransportSettings = Field(default_factory=TransportSettings)
    data: DataSettings = Field(default_factory=DataSettings)
    checkpoint: CheckpointSettings = Field(default_factory=CheckpointSettings)
    training: TrainingSettings = Field(default_factory=TrainingSettings)
    observability: ObservabilitySettings = Field(
        default_factory=ObservabilitySettings,
    )
    resume_checkpoint_uri: str | None = None
    device_preference: Literal["auto", "cpu", "cuda"] = "auto"

    # ------------------------------------------------------------ loader

    @classmethod
    def from_env(
        cls,
        overrides: dict[str, Any] | None = None,
    ) -> WorkerSettings:
        """Parse environment into a frozen settings bundle.

        The mapping is explicit (not a generic walker) so the contract
        between M03 env contract (§17.2) and this constructor is
        visible at a glance. Missing vars fall back to class defaults.
        """
        env = os.environ
        data: dict[str, Any] = {}

        # Identity (M03 dispatcher fills these unconditionally)
        identity: dict[str, Any] = {}
        for key, env_key in _IDENTITY_MAP.items():
            if env_key in env:
                identity[key] = env[env_key]
        if identity:
            data["identity"] = identity

        # Method selection
        if "EFE_METHOD_NAME" in env:
            data["method_name"] = env["EFE_METHOD_NAME"]

        # Hyperparameters
        if "EFE_HYPERPARAMETERS_JSON" in env:
            try:
                data["hyperparameters"] = json.loads(
                    env["EFE_HYPERPARAMETERS_JSON"],
                )
            except json.JSONDecodeError as exc:
                raise ConfigError(
                    f"EFE_HYPERPARAMETERS_JSON is not valid JSON: {exc}",
                ) from exc

        # Transport
        transport: dict[str, Any] = {}
        if "EFE_CALLBACK_URL" in env:
            transport["callback_url"] = env["EFE_CALLBACK_URL"]
        if "EFE_RPC_TIMEOUT_S" in env:
            transport["rpc_timeout_s"] = float(env["EFE_RPC_TIMEOUT_S"])
        if "EFE_HEARTBEAT_INTERVAL_S" in env:
            transport["heartbeat_interval_s"] = float(
                env["EFE_HEARTBEAT_INTERVAL_S"],
            )
        if "EFE_S3_ENDPOINT" in env:
            transport["s3_endpoint"] = env["EFE_S3_ENDPOINT"]
        if "EFE_FAKE_TRANSPORT" in env:
            transport["fake_transport"] = _truthy(env["EFE_FAKE_TRANSPORT"])
        if transport:
            data["transport"] = transport

        # Resume
        if "EFE_RESUME_CHECKPOINT" in env:
            data["resume_checkpoint_uri"] = env["EFE_RESUME_CHECKPOINT"]

        if "EFE_ENVIRONMENT" in env:
            data["environment"] = env["EFE_ENVIRONMENT"]
        if "EFE_DEVICE_PREFERENCE" in env:
            data["device_preference"] = env["EFE_DEVICE_PREFERENCE"]

        if overrides:
            data = _merge(data, overrides)
        try:
            return cls.model_validate(data)
        except Exception as exc:
            raise ConfigError(
                f"invalid efe-worker configuration: {exc}",
            ) from exc


_IDENTITY_MAP = {
    "run_id": "EFE_RUN_ID",
    "tenant_id": "EFE_TENANT_ID",
    "task_id": "EFE_TASK_ID",
    "snapshot_id": "EFE_SNAPSHOT_ID",
    "worker_id": "EFE_WORKER_ID",
    "snapshot_uri": "EFE_SNAPSHOT_URI",
    "base_model": "EFE_BASE_MODEL",
    "recipe_version": "EFE_RECIPE_VERSION",
    "pool_name": "EFE_POOL_NAME",
}


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _truthy(v: str) -> bool:
    return v.strip().lower() in {"true", "yes", "on", "1"}


def _merge(a: dict[str, Any], b: dict[str, Any]) -> dict[str, Any]:
    out = dict(a)
    for k, v in b.items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = _merge(out[k], v)
        else:
            out[k] = v
    return out


__all__ = [
    "CheckpointSettings",
    "DataSettings",
    "MethodName",
    "ObservabilitySettings",
    "RunIdentity",
    "TrainingSettings",
    "TransportSettings",
    "WorkerSettings",
]

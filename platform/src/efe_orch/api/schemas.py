"""Pydantic DTOs for the HTTP surface.

Kept at module scope (not inside ``build_app``) so
``from __future__ import annotations`` doesn't break pydantic's
runtime-model materialisation — identical pattern to what M01+M02
uses in ``efe_ft.api.http_schemas``.
"""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field


class SubmitRunRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tenant_id: str
    task_id: str
    submitted_by: str
    snapshot_id: str
    dataset_rows: int = Field(ge=0)
    base_model: str = "sentence-transformers/all-MiniLM-L6-v2"
    recipe_version: str = "v1"
    priority: str = "standard"
    needs_gpu: bool = False
    requested_pool: str | None = None
    hyperparameters: dict[str, Any] = Field(default_factory=dict)
    tags: dict[str, Any] = Field(default_factory=dict)
    estimated_gpu_seconds: int = Field(default=0, ge=0)


class CancelRunRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    actor: str
    reason: str = ""


class HeartbeatRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    run_id: str
    worker_id: str
    beat_seq: int = Field(ge=0)
    state_hint: str = "training"
    progress_ratio: float = Field(default=0.0, ge=0.0, le=1.0)
    step: int = Field(default=0, ge=0)
    extra: dict[str, Any] = Field(default_factory=dict)


class CheckpointRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    run_id: str
    worker_id: str
    step: int = Field(ge=0)
    blob_uri: str
    metrics: dict[str, Any] = Field(default_factory=dict)
    size_bytes: int = Field(default=0, ge=0)
    is_best: bool = False


class ResultRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    run_id: str
    worker_id: str
    artifact_url: str
    metrics: dict[str, Any] = Field(default_factory=dict)


class ErrorRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    run_id: str
    worker_id: str
    error_code: str
    error_message: str


__all__ = [
    "CancelRunRequest",
    "CheckpointRequest",
    "ErrorRequest",
    "HeartbeatRequest",
    "ResultRequest",
    "SubmitRunRequest",
]

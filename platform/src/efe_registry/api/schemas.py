"""Pydantic request / response models for the REST API."""

from __future__ import annotations

from datetime import datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


class PublishRequestModel(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tenant_id: str = Field(min_length=1)
    task_id: str = Field(min_length=1)
    semver: str = Field(min_length=1, max_length=64)
    content_hash_hex: str = Field(
        min_length=64, max_length=64,
        description="SHA-256 hex (64 chars).",
    )
    oci_ref: str = Field(min_length=1)
    s3_url: str = Field(min_length=1)
    parent_run_id: str = Field(min_length=1)
    framework: Literal["vllm", "peft", "full", "other"]
    size_bytes: int = Field(gt=0)
    parent_decision_id: str | None = None


class ArtifactView(BaseModel):
    model_config = ConfigDict(extra="forbid")

    artifact_id: str
    tenant_id: str
    task_id: str
    semver: str
    content_hash_hex: str
    state: str
    state_version: int
    created_at: datetime
    promoted_at: datetime | None
    deprecated_at: datetime | None


class RollbackRequestModel(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tenant_id: str = Field(min_length=1)
    task_id: str = Field(min_length=1)
    actor: str = Field(min_length=1)
    reason: str = Field(min_length=1, max_length=500)
    trigger_kind: Literal[
        "manual", "auto_slo", "auto_regression", "admin",
    ] = "manual"
    idempotency_key_hex: str | None = None


class RollbackResponseModel(BaseModel):
    model_config = ConfigDict(extra="forbid")

    rollback_id: str
    tenant_id: str
    task_id: str
    from_artifact_id: str
    to_artifact_id: str
    latency_ms: int
    final_state: Literal["completed", "partial", "failed"]
    phases: dict[str, float]


class CanaryView(BaseModel):
    model_config = ConfigDict(extra="forbid")

    canary_id: str
    tenant_id: str
    task_id: str
    current_percent: int
    target_percent: int
    state: str
    slo_breach_count: int
    started_at: datetime
    ended_at: datetime | None


__all__ = [
    "ArtifactView",
    "CanaryView",
    "PublishRequestModel",
    "RollbackRequestModel",
    "RollbackResponseModel",
]

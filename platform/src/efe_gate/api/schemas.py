"""Pydantic request/response DTOs for the HTTP surface."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field


class SubmitEvalRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tenant_id: str
    task_id: str
    task_type: str = "classif"
    candidate_run_id: str
    candidate_artifact_uri: str
    submitted_by: str
    golden_set_id: str | None = None
    metric_names: list[str] = Field(default_factory=list)
    tags: dict[str, Any] = Field(default_factory=dict)


class ConfirmReviewRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    reviewer_id: str
    approved: bool
    reason: str = ""


class UpsertGateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    rule: dict[str, Any]
    updated_by: str


class CreateGoldenSetRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tenant_id: str
    task_id: str
    snapshot_uri: str
    content_hash: str  # hex
    n_samples: int = Field(ge=1)
    label_balance: dict[str, int] = Field(default_factory=dict)
    fairness_attrs: dict[str, Any] = Field(default_factory=dict)
    created_by: str


class RollbackRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    actor: str
    reason: str = "manual"


__all__ = [
    "ConfirmReviewRequest",
    "CreateGoldenSetRequest",
    "RollbackRequest",
    "SubmitEvalRequest",
    "UpsertGateRequest",
]

"""Request/response pydantic schemas for the HTTP app.

Defined at **module scope** so FastAPI can resolve them under
``from __future__ import annotations`` (which turns type hints into
strings). Nesting them inside ``build_app()`` confuses FastAPI's
parameter classifier and produces spurious 422 errors.
"""

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

from efe_ft.ingest.schemas import SampleIngestPayload


class IngestBatchRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    items: list[SampleIngestPayload] = Field(..., min_length=1, max_length=10_000)


class SnapshotRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    actor_id: str = Field(..., min_length=1)
    status_filter: Literal["labelled", "pending", "any"] = "labelled"
    min_samples: int = Field(default=10, ge=1)
    train: int | None = None
    val: int | None = None
    holdout: int | None = None
    seed: int = 0


class StartSessionRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    annotator_id: str
    tenant_id: str
    task_id: str


class SubmitLabelRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    tenant_id: str
    session_id: str
    sample_id: str
    label: str
    time_ms: int = 0


class FlagRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    tenant_id: str
    session_id: str
    sample_id: str
    reason: Literal["skip", "unclear", "harmful", "pii"]


class ResolveRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    tenant_id: str
    sample_id: str
    expert_id: str
    label: str


__all__ = [
    "FlagRequest",
    "IngestBatchRequest",
    "ResolveRequest",
    "SnapshotRequest",
    "StartSessionRequest",
    "SubmitLabelRequest",
]

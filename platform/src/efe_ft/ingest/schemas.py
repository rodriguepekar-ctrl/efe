"""Pydantic models for the ingestion API."""

from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator


class SampleIngestPayload(BaseModel):
    """One sample as it arrives at the API boundary."""

    model_config = ConfigDict(extra="forbid")

    tenant_id: str = Field(..., min_length=1, max_length=128)
    task_id: str = Field(..., min_length=1, max_length=128)
    text: str = Field(..., min_length=1)
    language: str = Field(default="fr", pattern=r"^[a-z]{2}(-[A-Z]{2})?$")
    source: Literal["batch", "webhook", "labeling"] = "batch"
    origin_ref: str | None = Field(default=None, max_length=256)
    label: str | None = Field(default=None, max_length=128)
    label_confidence: float | None = Field(default=None, ge=0.0, le=1.0)
    actor_id: str = Field(..., min_length=1, max_length=128)

    @field_validator("text")
    @classmethod
    def _text_reasonable(cls, v: str) -> str:
        if not v.strip():
            raise ValueError("text must not be whitespace-only")
        return v


class BatchSummary(BaseModel):
    """Server-side result of a multi-sample ingestion."""

    model_config = ConfigDict(extra="forbid")

    batch_id: str
    n_received: int
    n_accepted: int
    n_rejected: int
    rejections: dict[str, int]  # reason -> count
    latency_ms: int


__all__ = ["BatchSummary", "SampleIngestPayload"]

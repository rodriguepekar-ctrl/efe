"""Pydantic schemas for the HTTP surface."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field


class AnswerRequest(BaseModel):
    """``POST /v1/answer`` body."""

    model_config = ConfigDict(extra="forbid")

    prompt: str = Field(..., min_length=1, max_length=32_768)
    tenant: str | None = Field(default=None, description="Overrides platform default.")
    request_id: str | None = Field(
        default=None, description="Caller-supplied correlation id."
    )
    budget_ms: int | None = Field(default=None, ge=100, le=120_000)
    budget_tokens: int | None = Field(default=None, ge=256, le=200_000)
    budget_usd: float | None = Field(default=None, ge=0.0, le=10.0)
    budget_tool_calls: int | None = Field(default=None, ge=1, le=64)


class AnswerResponse(BaseModel):
    """``POST /v1/answer`` response."""

    model_config = ConfigDict(extra="forbid")

    request_id: str
    tenant: str
    final_text: str
    recap: dict[str, Any]
    trace: list[dict[str, Any]]
    ledger: dict[str, Any]
    duration_ms: float
    ok: bool
    error: str | None = None


class HealthResponse(BaseModel):
    """``GET /healthz`` response."""

    model_config = ConfigDict(extra="forbid")

    status: str
    version: str
    upstream_swarm_reachable: bool


class CatalogTaskOut(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    description: str
    tenant: str
    cone_key: str
    dim_latent: int
    tags: list[str] = []


class CatalogResponse(BaseModel):
    model_config = ConfigDict(extra="forbid")

    version: int
    tasks: list[CatalogTaskOut]


class ProblemDetail(BaseModel):
    """RFC 7807-style error body."""

    model_config = ConfigDict(extra="forbid")

    type: str = "about:blank"
    title: str
    status: int
    detail: str | None = None
    code: str | None = None


__all__ = [
    "AnswerRequest",
    "AnswerResponse",
    "CatalogResponse",
    "CatalogTaskOut",
    "HealthResponse",
    "ProblemDetail",
]

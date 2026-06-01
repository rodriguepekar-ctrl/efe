"""Pydantic v2 schemas for M11 routes."""

from __future__ import annotations

from datetime import datetime
from typing import Any, Literal

try:  # pragma: no cover
    from pydantic import BaseModel, ConfigDict, Field
except ImportError:  # pragma: no cover
    BaseModel = object  # type: ignore[assignment,misc]

    def ConfigDict(**kwargs: Any) -> dict[str, Any]:  # type: ignore[no-redef]
        return kwargs

    def Field(*_args: Any, **_kwargs: Any) -> Any:  # type: ignore[no-redef]
        return None


__all__ = [
    "ConsentCreateRequest",
    "DsarSubmitRequest",
    "ErasureRequest",
    "ProblemDetailsSchema",
    "TenantOnboardRequest",
]


class _ForbidExtra(BaseModel):  # type: ignore[misc]
    model_config = ConfigDict(extra="forbid")


class TenantOnboardRequest(_ForbidExtra):
    tenant_id: str
    display_name: str
    tier: Literal["standard", "hipaa", "eu_sovereign"] = "standard"
    region: str = "eu-west-1"
    labels: dict[str, str] = Field(default_factory=dict)


class ConsentCreateRequest(_ForbidExtra):
    tenant_id: str
    subject_id: str
    purposes: list[str] = Field(min_length=1)
    legal_basis: Literal[
        "consent", "contract", "legal_obligation",
        "vital_interests", "public_task", "legitimate_interests",
    ] = "consent"
    consent_text: str
    locale: str = "en-US"
    collected_by: str = "api"
    expires_at: datetime | None = None


class DsarSubmitRequest(_ForbidExtra):
    tenant_id: str
    subject_id: str
    kind: Literal["access", "rectify", "erase", "portability"]


class ErasureRequest(_ForbidExtra):
    erasure_id: str
    tenant_id: str
    subject_id: str


class ProblemDetailsSchema(_ForbidExtra):
    type: str
    title: str
    status: int
    detail: str | None = None
    retryable: bool = False
    request_id: str | None = None

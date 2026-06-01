"""Pydantic REST schemas — request / response shapes.

Kept separate from FastAPI so tests can validate shapes without
importing FastAPI/starlette.
"""

from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field


class VariantSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str
    allocation_pct: int = Field(ge=0, le=100)
    payload: dict[str, Any] = Field(default_factory=dict)
    is_control: bool = False


class GuardrailSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    metric_name: str
    direction: Literal["up", "down"]
    threshold_pct: float = Field(gt=0.0)
    window_s: int = Field(default=300, ge=30, le=86_400)
    consecutive_breaches: int = Field(default=3, ge=1, le=20)


class CreateExperimentRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str
    tenant_id: str
    task_id: str
    layer: str
    kind: Literal["ab_fixed", "ab_sequential", "bandit", "holdout"]
    owner: str
    hypothesis: str
    primary_metric: str
    variants: list[VariantSchema]
    guardrails: list[GuardrailSchema] = Field(default_factory=list)
    alpha: float = Field(default=0.05, gt=0.0, lt=0.5)
    power_target: float = Field(default=0.8, gt=0.5, lt=1.0)
    minimum_detectable_effect: float = 0.0
    stopping_rule: Literal["fixed_horizon", "pocock", "obf"] = "fixed_horizon"
    planned_sample_size: int | None = None
    planned_duration_days: int | None = None
    traffic_allocation_bps: int = Field(default=10_000, ge=0, le=10_000)
    covariates_for_cuped: list[str] = Field(default_factory=list)
    notes: str | None = None


class ExperimentSummaryResponse(BaseModel):
    model_config = ConfigDict(extra="forbid")

    experiment_id: str
    name: str
    tenant_id: str
    task_id: str
    layer: str
    kind: str
    state: str
    state_version: int
    owner: str
    primary_metric: str
    alpha: float
    power_target: float
    variants: list[VariantSchema]


class AssignRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    user_id: str
    experiment_id: str
    request_id: str | None = None
    record_exposure: bool = True


class AssignReply(BaseModel):
    model_config = ConfigDict(extra="forbid")

    variant_id: str | None
    variant_name: str | None
    payload: dict[str, Any] = Field(default_factory=dict)
    holdout: bool
    forced: bool


class OutcomeRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    experiment_id: str
    user_id: str
    metric_name: str
    metric_value: float
    variant_id: str | None = None
    request_id: str | None = None
    covariate: dict[str, Any] = Field(default_factory=dict)


class AckResponse(BaseModel):
    model_config = ConfigDict(extra="forbid")

    ok: bool = True
    id: str | None = None


class AbortRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    reason: str


__all__ = [
    "AbortRequest",
    "AckResponse",
    "AssignReply",
    "AssignRequest",
    "CreateExperimentRequest",
    "ExperimentSummaryResponse",
    "GuardrailSchema",
    "OutcomeRequest",
    "VariantSchema",
]

"""Pydantic schemas for the drift REST API."""

from __future__ import annotations

from datetime import datetime
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

# ---------------------------------------------------------------------------
# Feature catalog
# ---------------------------------------------------------------------------


class FeatureSpecSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tenant_id: str
    task_id: str
    name: str
    kind: Literal["numeric", "categorical", "embedding", "binary"]
    extractor: str
    binning_strategy: Literal[
        "equifrequency", "equiwidth", "custom", "none",
    ] = "none"
    version: int = 1
    expected_cardinality: int | None = None
    retention_days: int = 90
    notes: str | None = None


class FeatureEntrySchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    feature_id: str
    tenant_id: str
    task_id: str
    name: str
    version: int
    kind: str
    extractor: str
    binning_strategy: str
    expected_cardinality: int | None = None
    retention_days: int
    enabled: bool
    notes: str | None = None


# ---------------------------------------------------------------------------
# Ingestion
# ---------------------------------------------------------------------------


class InferenceEventSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    event_id: str
    tenant_id: str
    task_id: str
    user_id: str
    observed_at: datetime
    request_id: str | None = None
    prompt_token_count: int | None = None
    response_token_count: int | None = None
    latency_ms: float | None = None
    cost_usd: float | None = None
    model_id: str | None = None
    backend_kind: str | None = None
    temperature: float | None = None
    top_p: float | None = None
    prompt_text: str | None = None
    response_text: str | None = None
    covariates: dict[str, Any] = Field(default_factory=dict)


class IngestAck(BaseModel):
    model_config = ConfigDict(extra="forbid")

    ok: bool = True
    samples_written: int = 0
    features_matched: int = 0


# ---------------------------------------------------------------------------
# Evaluations (L4 outputs)
# ---------------------------------------------------------------------------


class DriftResultSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    evaluation_id: str
    tenant_id: str
    task_id: str
    feature_id: str
    feature_name: str
    baseline_id: str
    method: str
    score: float
    severity: str
    computed_at: datetime
    window_start: datetime
    window_end: datetime
    p_value: float | None = None
    multi_scale_window: str | None = None
    details: dict[str, Any] = Field(default_factory=dict)
    sample_count_current: int
    sample_count_baseline: int


# ---------------------------------------------------------------------------
# Baselines
# ---------------------------------------------------------------------------


class BaselineSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    baseline_id: str
    tenant_id: str
    task_id: str
    feature_name: str
    feature_kind: str
    state: str
    state_version: int
    window_start: datetime
    window_end: datetime
    sample_count: int
    promoted_at: datetime | None = None
    retired_at: datetime | None = None
    planned_end_at: datetime | None = None


class EmergencyRetireRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    actor: str
    reason: str


# ---------------------------------------------------------------------------
# Incidents
# ---------------------------------------------------------------------------


class AlertIncidentSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    incident_id: str
    rule_id: str
    tenant_id: str
    feature_id: str
    feature_name: str
    method: str
    severity: str
    score_at_fire: float
    state: str
    fired_at: datetime
    acked_at: datetime | None = None
    acked_by: str | None = None
    resolved_at: datetime | None = None
    window_kind: str | None = None
    dispatch_results: dict[str, Any] = Field(default_factory=dict)


class AckIncidentRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    acked_by: str
    comment: str | None = None


# ---------------------------------------------------------------------------
# Rules
# ---------------------------------------------------------------------------


class ThresholdsSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    warning: float | None = None
    major: float | None = None
    critical: float | None = None
    orientation: Literal["higher_is_worse", "lower_is_worse"] = "higher_is_worse"


class ChannelRefSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    channel: str
    min_severity: Literal["warning", "major", "critical"] = "warning"


class AlertRuleCreateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tenant_id: str
    method: str
    thresholds: ThresholdsSchema
    channels: list[ChannelRefSchema] = Field(min_length=1)
    task_id: str | None = None
    feature_id: str | None = None
    window_kind: Literal["1h", "1d", "7d"] | None = None
    consecutive_breaches: int = Field(default=3, ge=1, le=20)
    suppression_s: int = Field(default=1800, ge=0, le=86_400)
    name: str | None = None
    notes: str | None = None


class AlertRuleSchema(BaseModel):
    model_config = ConfigDict(extra="forbid")

    rule_id: str
    tenant_id: str
    task_id: str | None = None
    feature_id: str | None = None
    method: str
    window_kind: str | None = None
    consecutive_breaches: int
    suppression_s: int
    enabled: bool
    thresholds: ThresholdsSchema
    channels: list[ChannelRefSchema]
    name: str | None = None
    notes: str | None = None


__all__ = [
    "AckIncidentRequest",
    "AlertIncidentSchema",
    "AlertRuleCreateRequest",
    "AlertRuleSchema",
    "BaselineSchema",
    "ChannelRefSchema",
    "DriftResultSchema",
    "EmergencyRetireRequest",
    "FeatureEntrySchema",
    "FeatureSpecSchema",
    "InferenceEventSchema",
    "IngestAck",
    "ThresholdsSchema",
]

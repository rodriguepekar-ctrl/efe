"""HTTP API — FastAPI factory + Pydantic schemas."""

from __future__ import annotations

from efe_drift.api.http_app import AppContext, build_app
from efe_drift.api.schemas import (
    AckIncidentRequest,
    AlertIncidentSchema,
    AlertRuleCreateRequest,
    AlertRuleSchema,
    BaselineSchema,
    ChannelRefSchema,
    DriftResultSchema,
    EmergencyRetireRequest,
    FeatureEntrySchema,
    FeatureSpecSchema,
    InferenceEventSchema,
    IngestAck,
    ThresholdsSchema,
)

__all__ = [
    "AckIncidentRequest",
    "AlertIncidentSchema",
    "AlertRuleCreateRequest",
    "AlertRuleSchema",
    "AppContext",
    "BaselineSchema",
    "ChannelRefSchema",
    "DriftResultSchema",
    "EmergencyRetireRequest",
    "FeatureEntrySchema",
    "FeatureSpecSchema",
    "InferenceEventSchema",
    "IngestAck",
    "ThresholdsSchema",
    "build_app",
]

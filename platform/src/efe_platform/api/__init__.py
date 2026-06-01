"""HTTP surface — FastAPI app + pydantic schemas + DI helpers."""

from __future__ import annotations

from efe_platform.api.dependencies import PlatformContext, get_context
from efe_platform.api.http_app import build_app, create_default_app
from efe_platform.api.schemas import AnswerRequest, AnswerResponse, HealthResponse

__all__ = [
    "AnswerRequest",
    "AnswerResponse",
    "HealthResponse",
    "PlatformContext",
    "build_app",
    "create_default_app",
    "get_context",
]

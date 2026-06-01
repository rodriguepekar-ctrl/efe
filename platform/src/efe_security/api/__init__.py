"""M11 API surface."""

from __future__ import annotations

from efe_security.api.deps import SecurityDeps
from efe_security.api.http_app import build_app
from efe_security.api.schemas import (
    ConsentCreateRequest,
    DsarSubmitRequest,
    ProblemDetailsSchema,
    TenantOnboardRequest,
)

__all__ = [
    "ConsentCreateRequest",
    "DsarSubmitRequest",
    "ProblemDetailsSchema",
    "SecurityDeps",
    "TenantOnboardRequest",
    "build_app",
]

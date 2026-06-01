"""FastAPI HTTP surface for :mod:`efe_orch`.

Public surface:

    build_app(context)   — construct the FastAPI app (optional extra
                           ``api`` must be installed).
    OrchContext          — injection bundle built by callers.
    schemas.*            — pydantic request/response models.
"""

from __future__ import annotations

from efe_orch.api.context import OrchContext
from efe_orch.api.http_app import build_app

__all__ = ["OrchContext", "build_app"]

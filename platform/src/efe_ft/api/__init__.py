"""HTTP + service façades for M01 + M02.

Submodules:
    http_app            — FastAPI application exposing M01 + M02 HTTP endpoints
    feature_store       — service façade for ingestion + snapshots
    labeling_service    — service façade wrapping LabelerService
    dependencies        — shared FastAPI Depends helpers
"""

from __future__ import annotations

from efe_ft.api.dependencies import FtContext, get_context, set_context
from efe_ft.api.feature_store import FeatureStoreFacade
from efe_ft.api.labeling_service import LabelingFacade

__all__ = [
    "FeatureStoreFacade",
    "FtContext",
    "LabelingFacade",
    "get_context",
    "set_context",
]

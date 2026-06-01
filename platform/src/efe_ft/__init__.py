"""EFE Fine-Tuning Service — M01 (Feature Store) + M02 (Labeling / AL).

This package implements the data-plane of the EFE embedder fine-tuning
service. It is **separate** from ``efe_platform`` (the swarm client
platform) on purpose — different lifecycle, different ownership.

Public surface (stable):
    efe_ft.errors        — exception hierarchy
    efe_ft.config        — pydantic-settings config
    efe_ft.db            — SQLAlchemy engine + models
    efe_ft.audit         — tamper-evident audit chain
    efe_ft.storage       — S3 / local blob store
    efe_ft.ingest        — 8-stage ingestion pipeline + quality gates
    efe_ft.snapshots     — immutable snapshot materialiser
    efe_ft.embedder      — background embedding worker
    efe_ft.labeling      — active learning, IAA, session, escalation
    efe_ft.api           — FeatureStore + Labeling service façades
    efe_ft.labeler_ui    — FastAPI + HTMX labeler web UI

All code here is typed strictly and tested with pytest-asyncio.
"""

from __future__ import annotations

from efe_ft.version import VERSION, __version__

__all__ = ["VERSION", "__version__"]

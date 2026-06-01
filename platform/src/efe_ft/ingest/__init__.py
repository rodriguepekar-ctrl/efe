"""Ingestion — 8-stage pipeline + quality gates.

Stages:
    1. Receive
    2. Auth + rate limit    (done at the API layer; pipeline assumes done)
    3. Schema validate
    4. PII scrub
    5. MinHash dedup
    6. Balance check
    7. Persist + enqueue embed
    8. Audit + return

Public surface:
    IngestPipeline       — orchestrator
    IngestInput          — pydantic request model
    IngestOutcome        — pydantic response model
    find_pii / scrub     — standalone functions
    Deduper              — MinHash LSH over Redis or in-memory
    balance_skew_check   — per-task class-balance invariant
"""

from __future__ import annotations

from efe_ft.ingest.balance import BalanceReport, balance_skew_check
from efe_ft.ingest.dedup import Deduper, InMemoryDeduper
from efe_ft.ingest.pii import Hit, find_pii, scrub
from efe_ft.ingest.pipeline import IngestInput, IngestOutcome, IngestPipeline
from efe_ft.ingest.schemas import BatchSummary, SampleIngestPayload

__all__ = [
    "BalanceReport",
    "BatchSummary",
    "Deduper",
    "Hit",
    "InMemoryDeduper",
    "IngestInput",
    "IngestOutcome",
    "IngestPipeline",
    "SampleIngestPayload",
    "balance_skew_check",
    "find_pii",
    "scrub",
]

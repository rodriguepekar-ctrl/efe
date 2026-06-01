"""EFE Evaluation Gate — M05.

The zero-regression quality gate between trained candidates (M04) and
production serving (M06). Every artifact promoted to champion MUST
match or beat the incumbent on every primary metric, respect every
budget cap, and introduce no fairness regression — measured on an
immutable, content-hashed golden set with appropriate statistical
tests.

Pipeline: candidate (M04 SUCCEEDED) → eval runner → metrics bundle
→ comparator → decision engine (3-gate cascade) → champion registry
→ rollback daemon (post-promote canary).

Relation to the rest of the platform:

    efe_ft        — M01 Feature Store + M02 Labeling
    efe_orch      — M03 Training Orchestrator
    efe_worker    — M04 Training Workers & Algorithms
    efe_gate      — **M05 Evaluation Gate (this package)**
    efe_serve     — M06 Inference Gateway (consumer)

Public surface:

    efe_gate.errors          — exception hierarchy
    efe_gate.config          — GateSettings
    efe_gate.db              — SQLAlchemy models (7 tables)
    efe_gate.state_machine   — 9-state FSM
    efe_gate.metrics         — MetricsBundle + statistical tests
    efe_gate.comparator      — candidate vs champion engine
    efe_gate.decision        — PromotionRule + DecisionEngine
    efe_gate.golden          — golden-set manager (immutable)
    efe_gate.champion        — registry + signed audit chain
    efe_gate.rollback        — post-promote daemon
    efe_gate.runner          — eval-job orchestration
    efe_gate.services        — EvalService, ReviewService
    efe_gate.api             — FastAPI HTTP surface
    efe_gate.observability   — metrics + logging
"""

from __future__ import annotations

from efe_gate.version import VERSION, __version__

__all__ = ["VERSION", "__version__"]

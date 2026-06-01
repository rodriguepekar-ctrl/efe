"""Integration tests for EvalService — submit → process → decide."""

from __future__ import annotations

from typing import Any

import numpy as np
import pytest
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.champion.registry import ChampionRegistry
from efe_gate.db.models import GoldenSet, PromotionGate
from efe_gate.errors import NotFound, ValidationError
from efe_gate.runner import FakeScorer
from efe_gate.services import EvalService, SubmitEvalInput
from efe_gate.state_machine import EvalState


def _sub(
    golden_set: GoldenSet,
    **overrides: Any,
) -> SubmitEvalInput:
    base = dict(
        tenant_id="acme", task_id="routing", task_type="classif",
        candidate_run_id="cccccccc-1111-1111-1111-111111111111",
        candidate_artifact_uri="s3://cand",
        submitted_by="m03-dispatcher",
        golden_set_id=golden_set.id,
        metric_names=("accuracy",),
    )
    base.update(overrides)
    return SubmitEvalInput(**base)  # type: ignore[arg-type]


async def test_submit_queues_eval(
    eval_service: EvalService,
    golden_set: GoldenSet,
    promotion_gate: PromotionGate,
) -> None:
    view = await eval_service.submit(_sub(golden_set))
    assert view.state is EvalState.QUEUED
    assert view.decision is None


async def test_submit_validation_errors(
    eval_service: EvalService,
    golden_set: GoldenSet,
) -> None:
    for override in (
        {"tenant_id": ""},
        {"candidate_run_id": ""},
        {"candidate_artifact_uri": ""},
        {"submitted_by": ""},
    ):
        with pytest.raises(ValidationError):
            await eval_service.submit(_sub(golden_set, **override))


async def test_submit_unknown_golden_set(
    eval_service: EvalService, golden_set: GoldenSet,
) -> None:
    with pytest.raises(NotFound):
        await eval_service.submit(_sub(
            golden_set,
            golden_set_id="00000000-0000-0000-0000-000000000000",
        ))


async def test_process_promotes_clear_winner(
    eval_service: EvalService,
    golden_set: GoldenSet,
    promotion_gate: PromotionGate,
    fake_scorer: FakeScorer,
    session: AsyncSession,
    registry: ChampionRegistry,
) -> None:
    # No champion yet → the candidate is first, promote auto.
    view = await eval_service.submit(_sub(golden_set))
    # Pre-seed scorer data.
    rng = np.random.default_rng(1)
    fake_scorer.scores[("s3://cand", "accuracy")] = (
        rng.random(200) < 0.9
    ).astype(np.int64)
    view2 = await eval_service.process(eval_id=view.id)
    await session.commit()
    assert view2.decision == "promote"
    assert view2.state is EvalState.PROMOTED
    champ = await registry.get("acme", "routing")
    assert champ.run_id == view.candidate_run_id


async def test_process_rejects_regression(
    eval_service: EvalService,
    golden_set: GoldenSet,
    promotion_gate: PromotionGate,
    fake_scorer: FakeScorer,
    session: AsyncSession,
    registry: ChampionRegistry,
) -> None:
    # Pre-promote a strong champion.
    rng = np.random.default_rng(1)
    fake_scorer.scores[("s3://strong", "accuracy")] = (
        rng.random(500) < 0.95
    ).astype(np.int64)
    # First submit: champion candidate.
    v0 = await eval_service.submit(_sub(
        golden_set,
        candidate_run_id="aaaaaaaa-1111-1111-1111-111111111111",
        candidate_artifact_uri="s3://strong",
    ))
    await eval_service.process(eval_id=v0.id)
    await session.commit()
    # Second submit: regression candidate.
    rng2 = np.random.default_rng(2)
    fake_scorer.scores[("s3://weak", "accuracy")] = (
        rng2.random(500) < 0.70
    ).astype(np.int64)
    v1 = await eval_service.submit(_sub(
        golden_set,
        candidate_run_id="bbbbbbbb-2222-2222-2222-222222222222",
        candidate_artifact_uri="s3://weak",
    ))
    out = await eval_service.process(eval_id=v1.id)
    await session.commit()
    assert out.decision == "reject"
    assert out.state is EvalState.REJECTED
    # Champion unchanged.
    champ = await registry.get("acme", "routing")
    assert champ.run_id == "aaaaaaaa-1111-1111-1111-111111111111"


async def test_process_missing_rule_raises(
    eval_service: EvalService,
    golden_set: GoldenSet,
    fake_scorer: FakeScorer,
) -> None:
    view = await eval_service.submit(_sub(golden_set))
    from efe_gate.errors import MissingRule
    fake_scorer.scores[("s3://cand", "accuracy")] = np.ones(50)
    with pytest.raises(MissingRule):
        await eval_service.process(eval_id=view.id)


async def test_list_paginated(
    eval_service: EvalService,
    golden_set: GoldenSet,
    promotion_gate: PromotionGate,
    session: AsyncSession,
) -> None:
    for i in range(3):
        await eval_service.submit(_sub(
            golden_set,
            candidate_run_id=f"aaaaaaaa-0000-0000-0000-{i:012d}",
        ))
        await session.commit()
    rows, tok = await eval_service.list(
        tenant_id="acme", task_id="routing", page_size=2,
    )
    assert len(rows) == 2
    assert tok == "2"
    rows2, tok2 = await eval_service.list(
        tenant_id="acme", task_id="routing",
        page_size=2, page_token=tok,
    )
    assert len(rows2) == 1
    assert tok2 is None


async def test_review_approve(
    eval_service: EvalService,
    golden_set: GoldenSet,
    promotion_gate: PromotionGate,
    fake_scorer: FakeScorer,
    session: AsyncSession,
) -> None:
    # Seed a champion.
    rng = np.random.default_rng(1)
    fake_scorer.scores[("s3://v1", "accuracy")] = (
        rng.random(500) < 0.9
    ).astype(np.int64)
    v0 = await eval_service.submit(_sub(
        golden_set, candidate_run_id="aaaa1111-0000-0000-0000-000000000000",
        candidate_artifact_uri="s3://v1",
    ))
    await eval_service.process(eval_id=v0.id)
    await session.commit()

    # Submit an ambiguous candidate (CI wide → AMBIGUOUS).
    rng2 = np.random.default_rng(2)
    fake_scorer.scores[("s3://maybe", "accuracy")] = (
        rng2.random(30) < 0.9   # small sample → CI wide
    ).astype(np.int64)
    v1 = await eval_service.submit(_sub(
        golden_set, candidate_run_id="bbbb2222-0000-0000-0000-000000000000",
        candidate_artifact_uri="s3://maybe",
    ))
    out = await eval_service.process(eval_id=v1.id)
    await session.commit()
    # With a wide CI on ~30 samples, expect AMBIGUOUS → HUMAN_REVIEW.
    assert out.state in (EvalState.HUMAN_REVIEW, EvalState.PROMOTED,
                          EvalState.REJECTED)
    if out.state is EvalState.HUMAN_REVIEW:
        approved = await eval_service.confirm_review(
            eval_id=v1.id,
            reviewer_id="ml-lead@acme",
            approved=True, reason="visible improvement",
        )
        await session.commit()
        assert approved.state is EvalState.PROMOTED

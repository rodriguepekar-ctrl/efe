"""FSM transition tests."""

from __future__ import annotations

import itertools

import pytest

from efe_registry.core.artifact import ArtifactState
from efe_registry.core.state_machine import (
    ARTIFACT_TRANSITIONS,
    ArtifactTransitionValidator,
    is_valid_transition,
)
from efe_registry.db.models import ArtifactStateTransition
from efe_registry.errors import (
    ArtifactImmutable,
    ArtifactNotFound,
    InvalidArtifactTransition,
)
from tests_registry.conftest import _seed_artifact


def test_whitelist_has_14_transitions():
    assert len(ARTIFACT_TRANSITIONS) == 14


def test_valid_transitions():
    for frm, to in ARTIFACT_TRANSITIONS:
        assert is_valid_transition(frm, to)


def test_unlisted_transitions_are_invalid():
    count_invalid = 0
    for frm, to in itertools.product(ArtifactState, ArtifactState):
        if frm == to:
            continue
        if (frm, to) in ARTIFACT_TRANSITIONS:
            continue
        assert not is_valid_transition(frm, to)
        count_invalid += 1
    assert count_invalid > 0


def test_purged_is_terminal():
    for to in ArtifactState:
        assert not is_valid_transition(ArtifactState.PURGED, to)


@pytest.mark.asyncio
async def test_cas_apply_happy_path(seeded_session):
    ar = await _seed_artifact(seeded_session, state=ArtifactState.UPLOADED)
    validator = ArtifactTransitionValidator(seeded_session)
    result = await validator.apply(
        artifact_id=ar.artifact_id,
        expected_state=ArtifactState.UPLOADED,
        new_state=ArtifactState.SIGNED,
        expected_version=0,
        actor="tests",
    )
    assert result.new_version == 1
    assert result.to_state == ArtifactState.SIGNED


@pytest.mark.asyncio
async def test_cas_apply_rejects_invalid_transition(seeded_session):
    ar = await _seed_artifact(seeded_session, state=ArtifactState.UPLOADED)
    validator = ArtifactTransitionValidator(seeded_session)
    with pytest.raises(InvalidArtifactTransition):
        await validator.apply(
            artifact_id=ar.artifact_id,
            expected_state=ArtifactState.UPLOADED,
            new_state=ArtifactState.CHAMPION,  # not allowed
            expected_version=0,
            actor="tests",
        )


@pytest.mark.asyncio
async def test_cas_apply_rejects_stale_version(seeded_session):
    ar = await _seed_artifact(
        seeded_session, state=ArtifactState.UPLOADED, state_version=3,
    )
    validator = ArtifactTransitionValidator(seeded_session)
    with pytest.raises(ArtifactImmutable):
        await validator.apply(
            artifact_id=ar.artifact_id,
            expected_state=ArtifactState.UPLOADED,
            new_state=ArtifactState.SIGNED,
            expected_version=0,  # stale
            actor="tests",
        )


@pytest.mark.asyncio
async def test_cas_apply_not_found(seeded_session):
    validator = ArtifactTransitionValidator(seeded_session)
    with pytest.raises(ArtifactNotFound):
        await validator.apply(
            artifact_id="00000000-0000-0000-0000-000000000000",
            expected_state=ArtifactState.UPLOADED,
            new_state=ArtifactState.SIGNED,
            expected_version=0,
            actor="tests",
        )


@pytest.mark.asyncio
async def test_transition_is_journaled(seeded_session):
    ar = await _seed_artifact(seeded_session, state=ArtifactState.UPLOADED)
    validator = ArtifactTransitionValidator(seeded_session)
    await validator.apply(
        artifact_id=ar.artifact_id,
        expected_state=ArtifactState.UPLOADED,
        new_state=ArtifactState.SIGNED,
        expected_version=0,
        actor="tests",
        reason="unit test",
    )
    # Journal record present.
    from sqlalchemy import select
    rows = list(
        (
            await seeded_session.execute(
                select(ArtifactStateTransition).where(
                    ArtifactStateTransition.artifact_id == ar.artifact_id,
                ),
            )
        ).scalars(),
    )
    assert len(rows) == 1
    assert rows[0].from_state == "uploaded"
    assert rows[0].to_state == "signed"

"""State machine whitelist + authority enforcement."""

from __future__ import annotations

import pytest

from efe_rollout.errors import (
    AllocationInvariantBroken,
    InvalidTransition,
    ValidationError,
)
from efe_rollout.experiment import (
    ExperimentFsm,
    ExperimentSpec,
    Variant,
)


def _valid_spec(**overrides):
    base = dict(
        name="t",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="hope",
        primary_metric="ctr",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True),
            Variant(name="treatment", allocation_pct=50),
        ),
    )
    base.update(overrides)
    return ExperimentSpec(**base)


def test_spec_sum_to_100():
    with pytest.raises(AllocationInvariantBroken):
        _valid_spec(variants=(
            Variant(name="a", allocation_pct=60),
            Variant(name="b", allocation_pct=60),
        ))


def test_spec_duplicate_variant_names():
    with pytest.raises(ValidationError):
        _valid_spec(variants=(
            Variant(name="a", allocation_pct=50),
            Variant(name="a", allocation_pct=50),
        ))


def test_spec_hash_stable_across_equivalent_objects():
    s1 = _valid_spec()
    s2 = _valid_spec()
    assert s1.spec_hash() == s2.spec_hash()


def test_spec_hash_changes_when_variants_reorder_is_noop():
    s1 = _valid_spec()
    s2 = _valid_spec(variants=(
        Variant(name="treatment", allocation_pct=50),
        Variant(name="control", allocation_pct=50, is_control=True),
    ))
    assert s1.spec_hash() == s2.spec_hash()


def test_fsm_allows_designed_from_draft():
    ExperimentFsm.validate("draft", "designed", actor_role="owner")


def test_fsm_rejects_running_from_draft():
    with pytest.raises(InvalidTransition):
        ExperimentFsm.validate("draft", "running", actor_role="owner")


def test_fsm_rejects_from_terminal_state():
    for terminal in ("shipped", "rollback", "inconclusive",
                     "canceled", "aborted"):
        with pytest.raises(InvalidTransition):
            ExperimentFsm.validate(terminal, "running", actor_role="admin")


def test_fsm_enforces_actor_role():
    # owner cannot abort a paused experiment (admin-only)
    with pytest.raises(InvalidTransition):
        ExperimentFsm.validate("paused", "aborted", actor_role="owner")
    # admin can
    ExperimentFsm.validate("paused", "aborted", actor_role="admin")


def test_fsm_rejects_no_op():
    with pytest.raises(InvalidTransition):
        ExperimentFsm.validate("running", "running", actor_role="owner")


def test_fsm_abort_from_running_is_system_or_admin():
    # owner cannot directly abort a running experiment (must go through
    # pause then admin-abort, or an auto-abort from the watcher)
    with pytest.raises(InvalidTransition):
        ExperimentFsm.validate("running", "aborted", actor_role="owner")
    ExperimentFsm.validate("running", "aborted", actor_role="system")
    ExperimentFsm.validate("running", "aborted", actor_role="admin")


def test_allowed_transitions_from_running():
    allowed = ExperimentFsm.allowed_transitions("running")
    assert set(allowed) == {"paused", "aborted", "analyzed"}

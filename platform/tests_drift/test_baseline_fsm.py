"""Baseline FSM — whitelist + actor roles."""

from __future__ import annotations

import pytest

from efe_drift.baseline import BaselineFsm, TERMINAL_STATES, iter_transitions
from efe_drift.errors import InvalidBaselineTransition


def test_fsm_all_transitions_reachable_from_iter():
    transitions = set(iter_transitions())
    assert ("planned", "accumulating") in transitions
    assert ("accumulating", "candidate") in transitions
    assert ("candidate", "active") in transitions
    assert ("active", "retiring") in transitions
    assert ("retiring", "retired") in transitions


def test_fsm_allowed_transitions_from_candidate():
    allowed = BaselineFsm.allowed_transitions("candidate")
    assert set(allowed) == {"active", "rejected"}


def test_fsm_rejects_from_terminal():
    for terminal in TERMINAL_STATES:
        with pytest.raises(InvalidBaselineTransition):
            BaselineFsm.validate(
                terminal, "active", actor_role="admin",
            )


def test_fsm_rejects_noop():
    with pytest.raises(InvalidBaselineTransition):
        BaselineFsm.validate("active", "active", actor_role="admin")


def test_fsm_rejects_unknown_transition():
    with pytest.raises(InvalidBaselineTransition):
        BaselineFsm.validate("planned", "active", actor_role="admin")


def test_fsm_enforces_actor_role():
    # active → retiring requires system or admin
    with pytest.raises(InvalidBaselineTransition):
        BaselineFsm.validate(
            "active", "retiring", actor_role="owner",
        )
    BaselineFsm.validate(
        "active", "retiring", actor_role="system",
    )
    BaselineFsm.validate(
        "active", "retiring", actor_role="admin",
    )


def test_fsm_emergency_retire_admin_only():
    # Emergency path is active → retired, admin only.
    with pytest.raises(InvalidBaselineTransition):
        BaselineFsm.validate(
            "active", "retired", actor_role="system",
        )
    BaselineFsm.validate(
        "active", "retired", actor_role="admin",
    )


def test_fsm_required_roles_unknown_edge_raises():
    with pytest.raises(InvalidBaselineTransition):
        BaselineFsm.required_roles("planned", "active")


def test_fsm_is_terminal():
    assert BaselineFsm.is_terminal("retired")
    assert BaselineFsm.is_terminal("rejected")
    assert not BaselineFsm.is_terminal("active")
    assert not BaselineFsm.is_terminal("retiring")

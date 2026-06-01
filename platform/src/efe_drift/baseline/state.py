"""Baseline FSM — whitelist of allowed transitions.

Strict state machine mirroring M09 §5.1. Nine edges in total (8 from
the architecture doc plus a convenience `PLANNED → REJECTED` path
used when an operator cancels a still-unpromoted candidate).

The FSM is pure : it validates transitions given ``from_state``,
``to_state`` and an actor role. It does NOT talk to any store. The
service layer (:class:`BaselineManager`) wires FSM validation
together with optimistic-lock persistence.

Actor roles :

* ``owner``  — the team that registered the feature; can drive the
  happy path and reject candidates.
* ``admin`` — platform admin; can emergency-retire, overrule an owner
  rejection, cancel a planned baseline.
* ``system`` — automated actors (scheduler, rotation job).

Terminal states : ``retired``, ``rejected``.
"""

from __future__ import annotations

from collections.abc import Iterable

from efe_drift.errors import InvalidBaselineTransition
from efe_drift.records import BaselineState

# Canonical terminal states.
TERMINAL_STATES: frozenset[BaselineState] = frozenset({
    "retired", "rejected",
})


# (from, to) → required actor roles
_TRANSITIONS: dict[
    tuple[BaselineState, BaselineState], frozenset[str]
] = {
    ("planned", "accumulating"): frozenset({"system", "admin"}),
    ("planned", "rejected"): frozenset({"admin", "owner"}),
    ("accumulating", "candidate"): frozenset({"system", "admin"}),
    ("accumulating", "rejected"): frozenset({"admin"}),
    ("candidate", "active"): frozenset({"owner", "admin"}),
    ("candidate", "rejected"): frozenset({"owner", "admin"}),
    ("active", "retiring"): frozenset({"system", "admin"}),
    ("active", "retired"): frozenset({"admin"}),                 # emergency
    ("retiring", "retired"): frozenset({"system", "admin"}),
}


class BaselineFsm:
    """Pure-Python FSM for :class:`BaselineState` transitions.

    :meth:`validate` either returns ``None`` (transition allowed) or
    raises :class:`InvalidBaselineTransition`. The service layer is
    expected to compose this with an optimistic-lock CAS.
    """

    @staticmethod
    def is_terminal(state: BaselineState) -> bool:
        return state in TERMINAL_STATES

    @staticmethod
    def allowed_transitions(
        state: BaselineState,
    ) -> tuple[BaselineState, ...]:
        return tuple(
            tgt for (src, tgt) in _TRANSITIONS if src == state
        )

    @classmethod
    def required_roles(
        cls,
        from_state: BaselineState,
        to_state: BaselineState,
    ) -> frozenset[str]:
        key = (from_state, to_state)
        if key not in _TRANSITIONS:
            raise InvalidBaselineTransition(
                f"{from_state!r} → {to_state!r} is not whitelisted",
            )
        return _TRANSITIONS[key]

    @classmethod
    def validate(
        cls,
        from_state: BaselineState,
        to_state: BaselineState,
        *,
        actor_role: str = "owner",
    ) -> None:
        if from_state == to_state:
            raise InvalidBaselineTransition(
                f"no-op transition {from_state!r} → {to_state!r}",
            )
        if cls.is_terminal(from_state):
            raise InvalidBaselineTransition(
                f"state {from_state!r} is terminal — cannot transition",
            )
        required = cls.required_roles(from_state, to_state)
        if actor_role not in required:
            raise InvalidBaselineTransition(
                f"transition {from_state!r} → {to_state!r} requires "
                f"one of {sorted(required)}, got {actor_role!r}",
                detail={
                    "required": sorted(required),
                    "actor": actor_role,
                },
            )


def iter_transitions() -> Iterable[tuple[BaselineState, BaselineState]]:
    """Enumerate whitelisted transitions — handy for test coverage."""
    return _TRANSITIONS.keys()


__all__ = [
    "TERMINAL_STATES",
    "BaselineFsm",
    "iter_transitions",
]

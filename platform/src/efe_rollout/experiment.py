"""Canonical experiment domain types + FSM.

Core value objects used throughout the rollout module:

* :class:`Variant` — one arm of an experiment (control, treatment...).
* :class:`GuardrailSpec` — a secondary-metric guardrail with threshold.
* :class:`ExperimentSpec` — immutable bundle describing an experiment.
  It is the thing whose SHA-256 gets sealed into a pre-registration
  (invariant **I6**).
* :class:`Experiment` — runtime model: spec + current state + state
  version + timestamps. Carries the FSM.
* :class:`ExperimentFsm` — the strict whitelist of state transitions
  documented in M08 §5.1. Unlike a generic lookup table, the FSM also
  records the actor authority required for each transition so the
  service layer can authorise before calling :meth:`transition`.

The module is self-contained — importable without SQLAlchemy / FastAPI
/ numpy so a client library pinning ``efe-rollout`` can consume types
without heavy deps.
"""

from __future__ import annotations

from dataclasses import dataclass, field, replace
from datetime import datetime
from typing import Any, Literal

from efe_rollout.errors import (
    AllocationInvariantBroken,
    InvalidTransition,
    NoVariantsDefined,
    ValidationError,
)
from efe_rollout.hashing import canonical_json_bytes, content_hash

ExperimentKind = Literal["ab_fixed", "ab_sequential", "bandit", "holdout"]

ExperimentState = Literal[
    "draft",
    "designed",
    "registered",
    "running",
    "paused",
    "aborted",
    "analyzed",
    "shipped",
    "rollback",
    "inconclusive",
    "canceled",
]

StoppingRule = Literal["fixed_horizon", "pocock", "obf"]

# Terminal (non-transitionable) states.
TERMINAL_STATES: frozenset[ExperimentState] = frozenset(
    {"canceled", "aborted", "shipped", "rollback", "inconclusive"},
)


@dataclass(frozen=True, slots=True)
class Variant:
    """One arm of an experiment.

    ``allocation_pct`` must be in ``[0, 100]``. The *sum* of allocations
    over an experiment's variants is enforced to equal 100 by
    :meth:`ExperimentSpec._validate`. ``payload`` is the arm-specific
    configuration (model id, prompt template, post-process flag, ...)
    — opaque to the rollout module, consumed by the caller.
    """

    name: str
    allocation_pct: int
    payload: dict[str, Any] = field(default_factory=dict)
    is_control: bool = False
    variant_id: str | None = None

    def __post_init__(self) -> None:
        if not self.name or not self.name.strip():
            raise ValidationError("variant name must be non-empty")
        if not 0 <= self.allocation_pct <= 100:
            raise ValidationError(
                f"allocation_pct {self.allocation_pct} not in [0,100]",
            )


@dataclass(frozen=True, slots=True)
class GuardrailSpec:
    """Secondary-metric guardrail.

    ``direction``:
      * ``"up"`` — the metric is *bad* when it increases (latency,
        errors, cost, thumbs_down).
      * ``"down"`` — the metric is *bad* when it decreases (CTR,
        thumbs_up, retention).

    ``threshold_pct`` is interpreted as the fractional regression
    relative to the control arm that counts as a breach. For example,
    with direction ``"up"`` and threshold ``30``, ``candidate / control
    ≥ 1.30`` triggers a breach.
    """

    metric_name: str
    direction: Literal["up", "down"]
    threshold_pct: float
    window_s: int = 300
    consecutive_breaches: int = 3

    def __post_init__(self) -> None:
        if self.threshold_pct <= 0:
            raise ValidationError(
                f"guardrail threshold_pct must be positive, got {self.threshold_pct}",
            )
        if self.window_s <= 0:
            raise ValidationError(
                f"guardrail window_s must be positive, got {self.window_s}",
            )
        if self.consecutive_breaches <= 0:
            raise ValidationError(
                "consecutive_breaches must be positive, got "
                f"{self.consecutive_breaches}",
            )


@dataclass(frozen=True, slots=True)
class ExperimentSpec:
    """Immutable specification of an experiment.

    Two specs are considered equal (for hash purposes) iff every field
    below matches byte-for-byte after canonical JSON encoding.
    Mutating even a typo-fix requires a new experiment id.
    """

    name: str
    tenant_id: str
    task_id: str
    layer: str
    kind: ExperimentKind
    owner: str
    hypothesis: str
    primary_metric: str
    variants: tuple[Variant, ...]
    guardrails: tuple[GuardrailSpec, ...] = ()
    alpha: float = 0.05
    power_target: float = 0.8
    minimum_detectable_effect: float = 0.0
    stopping_rule: StoppingRule = "fixed_horizon"
    planned_sample_size: int | None = None
    planned_duration_days: int | None = None
    traffic_allocation_bps: int = 10_000
    covariates_for_cuped: tuple[str, ...] = ()
    notes: str | None = None

    def __post_init__(self) -> None:
        self._validate()

    def _validate(self) -> None:
        if not self.name.strip():
            raise ValidationError("experiment name must be non-empty")
        if not self.tenant_id.strip():
            raise ValidationError("tenant_id must be non-empty")
        if not self.task_id.strip():
            raise ValidationError("task_id must be non-empty")
        if not self.layer.strip():
            raise ValidationError("layer must be non-empty")
        if not 0.0 < self.alpha < 0.5:
            raise ValidationError(f"alpha {self.alpha} outside (0, 0.5)")
        if not 0.5 < self.power_target < 1.0:
            raise ValidationError(
                f"power_target {self.power_target} outside (0.5, 1)",
            )
        if not 0 <= self.traffic_allocation_bps <= 10_000:
            raise ValidationError(
                f"traffic_allocation_bps {self.traffic_allocation_bps} "
                "outside [0, 10000]",
            )
        if not self.variants:
            raise NoVariantsDefined(
                f"experiment '{self.name}' has no variants",
            )
        total = sum(v.allocation_pct for v in self.variants)
        if total != 100:
            raise AllocationInvariantBroken(
                f"variant allocations sum to {total}, expected 100",
                detail={
                    "variants": [
                        {"name": v.name, "pct": v.allocation_pct}
                        for v in self.variants
                    ],
                },
            )
        controls = [v for v in self.variants if v.is_control]
        if len(controls) > 1:
            raise ValidationError(
                f"at most one variant may be marked control, got {len(controls)}",
            )
        seen_names: set[str] = set()
        for v in self.variants:
            if v.name in seen_names:
                raise ValidationError(
                    f"duplicate variant name: {v.name!r}",
                )
            seen_names.add(v.name)

    # ---- derived helpers ------------------------------------------------

    def control(self) -> Variant | None:
        for v in self.variants:
            if v.is_control:
                return v
        return None

    def variants_sorted(self) -> tuple[Variant, ...]:
        """Stable-name-ordered variants (the assigner needs this)."""
        return tuple(sorted(self.variants, key=lambda v: v.name))

    def to_canonical_dict(self) -> dict[str, Any]:
        """Shape used by :func:`content_hash` for the spec seal.

        Variant ids (if any) are *not* hashed because they are assigned
        by the DB only after :meth:`ExperimentFsm.to_designed`.
        """
        return {
            "name": self.name,
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "layer": self.layer,
            "kind": self.kind,
            "owner": self.owner,
            "hypothesis": self.hypothesis,
            "primary_metric": self.primary_metric,
            "alpha": self.alpha,
            "power_target": self.power_target,
            "minimum_detectable_effect": self.minimum_detectable_effect,
            "stopping_rule": self.stopping_rule,
            "planned_sample_size": self.planned_sample_size,
            "planned_duration_days": self.planned_duration_days,
            "traffic_allocation_bps": self.traffic_allocation_bps,
            "covariates_for_cuped": list(self.covariates_for_cuped),
            "notes": self.notes,
            "variants": [
                {
                    "name": v.name,
                    "allocation_pct": v.allocation_pct,
                    "payload": v.payload,
                    "is_control": v.is_control,
                }
                for v in self.variants_sorted()
            ],
            "guardrails": [
                {
                    "metric_name": g.metric_name,
                    "direction": g.direction,
                    "threshold_pct": g.threshold_pct,
                    "window_s": g.window_s,
                    "consecutive_breaches": g.consecutive_breaches,
                }
                for g in sorted(
                    self.guardrails, key=lambda g: g.metric_name,
                )
            ],
        }

    def spec_hash(self) -> bytes:
        return content_hash(self.to_canonical_dict())

    def canonical_bytes(self) -> bytes:
        return canonical_json_bytes(self.to_canonical_dict())


@dataclass(frozen=True, slots=True)
class Experiment:
    """Runtime view: spec + FSM state + state_version + timestamps."""

    experiment_id: str
    spec: ExperimentSpec
    state: ExperimentState
    state_version: int = 0
    created_at: datetime | None = None
    designed_at: datetime | None = None
    registered_at: datetime | None = None
    started_at: datetime | None = None
    ended_at: datetime | None = None
    planned_end_at: datetime | None = None

    def with_state(
        self,
        new_state: ExperimentState,
        *,
        now: datetime,
        state_version: int | None = None,
    ) -> Experiment:
        """Return a new :class:`Experiment` with the state advanced.

        Does **not** validate the transition — the FSM does that. This
        helper just applies the state change + relevant timestamp.
        """
        next_version = (
            state_version if state_version is not None else self.state_version + 1
        )
        updates: dict[str, Any] = {
            "state": new_state,
            "state_version": next_version,
        }
        if new_state == "designed":
            updates["designed_at"] = now
        elif new_state == "registered":
            updates["registered_at"] = now
        elif new_state == "running":
            if self.started_at is None:
                updates["started_at"] = now
        elif new_state in {"aborted", "shipped", "rollback",
                           "inconclusive", "analyzed", "canceled"}:
            updates["ended_at"] = now
        return replace(self, **updates)


# ---------------------------------------------------------------------------
# FSM
# ---------------------------------------------------------------------------


# (from_state, to_state) -> authority required (one of: 'owner', 'admin',
# 'system', 'any').
_TRANSITIONS: dict[
    tuple[ExperimentState, ExperimentState], frozenset[str]
] = {
    ("draft", "designed"): frozenset({"owner"}),
    ("draft", "canceled"): frozenset({"owner", "admin"}),
    ("designed", "registered"): frozenset({"owner"}),
    ("designed", "canceled"): frozenset({"owner", "admin"}),
    ("registered", "running"): frozenset({"owner"}),
    ("registered", "canceled"): frozenset({"owner", "admin"}),
    ("running", "paused"): frozenset({"owner", "admin"}),
    ("running", "aborted"): frozenset({"system", "admin"}),
    ("running", "analyzed"): frozenset({"system", "owner"}),
    ("paused", "running"): frozenset({"owner", "admin"}),
    ("paused", "aborted"): frozenset({"admin"}),
    ("analyzed", "shipped"): frozenset({"owner"}),
    ("analyzed", "rollback"): frozenset({"owner"}),
    ("analyzed", "inconclusive"): frozenset({"owner"}),
}


class ExperimentFsm:
    """Pure-Python FSM — no I/O, no persistence.

    Given a source + target state and an ``actor_role`` (``'owner'``,
    ``'admin'``, ``'system'``), :meth:`validate` either returns None
    (transition allowed) or raises :class:`InvalidTransition`. The
    service layer composes this with an optimistic-lock persistence
    step.
    """

    @staticmethod
    def is_terminal(state: ExperimentState) -> bool:
        return state in TERMINAL_STATES

    @staticmethod
    def allowed_transitions(
        state: ExperimentState,
    ) -> tuple[ExperimentState, ...]:
        return tuple(
            tgt for (src, tgt) in _TRANSITIONS if src == state
        )

    @classmethod
    def validate(
        cls,
        from_state: ExperimentState,
        to_state: ExperimentState,
        *,
        actor_role: str = "owner",
    ) -> None:
        if from_state == to_state:
            raise InvalidTransition(
                f"no-op transition {from_state!r} → {to_state!r}",
            )
        if cls.is_terminal(from_state):
            raise InvalidTransition(
                f"state {from_state!r} is terminal; cannot transition",
            )
        key = (from_state, to_state)
        if key not in _TRANSITIONS:
            raise InvalidTransition(
                f"{from_state!r} → {to_state!r} is not a whitelisted transition",
            )
        required = _TRANSITIONS[key]
        if actor_role not in required:
            raise InvalidTransition(
                f"transition {from_state!r} → {to_state!r} requires one of "
                f"{sorted(required)}, got {actor_role!r}",
                detail={"required": sorted(required), "actor": actor_role},
            )


__all__ = [
    "TERMINAL_STATES",
    "Experiment",
    "ExperimentFsm",
    "ExperimentKind",
    "ExperimentSpec",
    "ExperimentState",
    "GuardrailSpec",
    "StoppingRule",
    "Variant",
]

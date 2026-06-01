"""Regression tests for bugs discovered during the M08 surgical audit.

Each test pins the fix for a specific bug category so future
refactors can't silently re-introduce them.

Bug A — layer-slot reservation must happen BEFORE the FSM transition.
Bug H — pre-registration resume path when transition crashed.
Bug P — NATS envelope missing fields must surface as NatsMessageIntegrity.
Bug (cache ghost-variant) — stale cache variant_id must not leak as
  ``(variant=None, from_cache=True)``.
Bug E — analyzer O(n²) cluster-robust user-id lookup → O(n).
Bug (bandit race) — concurrent apply_batch must not lose updates.
Bug (guardrail aging) — counter must age-out across long tick gaps.
Bug (guardrail zero-baseline) — ``down`` direction with mean_control=0
  must not fabricate a breach.
"""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone

import pytest

from efe_rollout.assignment import (
    HashAssigner,
    HoldoutFilter,
    InMemoryAssignmentCache,
)
from efe_rollout.bandits import (
    BanditUpdater,
    InMemoryBanditStateStore,
)
from efe_rollout.clock import FakeClock
from efe_rollout.errors import (
    LayerSlotBusy,
    NatsMessageIntegrity,
    PreRegistrationSealed,
)
from efe_rollout.experiment import ExperimentSpec, GuardrailSpec, Variant
from efe_rollout.guardrails import GuardrailRuntime, GuardrailWatcher
from efe_rollout.layers import LayerRegistry
from efe_rollout.nats import EnvelopeSigner, EnvelopeVerifier
from efe_rollout.records import Outcome
from efe_rollout.registry import ExperimentRegistry, InMemoryRegistryStore


def _spec(name: str = "e", layer: str = "prompt") -> ExperimentSpec:
    return ExperimentSpec(
        name=name,
        tenant_id="acme",
        task_id="summarize",
        layer=layer,
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True),
            Variant(name="treatment", allocation_pct=50),
        ),
    )


def _registry(clock: FakeClock) -> ExperimentRegistry:
    return ExperimentRegistry(
        store=InMemoryRegistryStore(),
        signing_key=b"k" * 32,
        signing_key_id="kms/alias-1",
        clock=clock,
    )


# ---------------------------------------------------------------------------
# Bug A — layer-slot reservation ordering
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_layer_slot_busy_rolls_back_state_transition():
    """A failed layer reservation must leave the experiment state unchanged.

    Prior to the audit fix, the state machine advanced to ``running``
    before the layer-slot check, so a :class:`LayerSlotBusy` reached
    the caller with the experiment in a half-transitioned state.
    """
    reg = _registry(FakeClock())
    a = await reg.create(_spec("a"))
    b = await reg.create(_spec("b"))
    # Move A to running — owns the prompt slot.
    await reg.transition(a.experiment_id, to_state="designed",
                         actor_role="owner")
    await reg.pre_register(a.experiment_id)
    await reg.transition(a.experiment_id, to_state="running",
                         actor_role="owner")
    # Try to move B into running on the same slot.
    await reg.transition(b.experiment_id, to_state="designed",
                         actor_role="owner")
    await reg.pre_register(b.experiment_id)
    with pytest.raises(LayerSlotBusy):
        await reg.transition(b.experiment_id, to_state="running",
                             actor_role="owner")
    # B must still be in 'registered' (not 'running').
    b_after = await reg.get(b.experiment_id)
    assert b_after.state == "registered", (
        f"state rolled forward despite LayerSlotBusy: {b_after.state!r}"
    )


# ---------------------------------------------------------------------------
# Bug H — pre-registration resume after transition crash
# ---------------------------------------------------------------------------


class _FailingTransitionStore(InMemoryRegistryStore):
    """Fails the first ``designed → registered`` transition so we can
    simulate the half-sealed-half-transitioned partial-failure case."""

    def __init__(self) -> None:
        super().__init__()
        self._crash_next = False

    def arm_crash(self) -> None:
        self._crash_next = True

    async def transition(self, *args, **kwargs):
        if (
            self._crash_next
            and kwargs.get("from_state") == "designed"
            and kwargs.get("to_state") == "registered"
        ):
            self._crash_next = False
            raise RuntimeError("simulated DB outage")
        return await super().transition(*args, **kwargs)


@pytest.mark.asyncio
async def test_pre_register_resume_path_drives_transition_forward():
    store = _FailingTransitionStore()
    reg = ExperimentRegistry(
        store=store,
        signing_key=b"k" * 32,
        signing_key_id="kms/alias-1",
        clock=FakeClock(),
    )
    exp = await reg.create(_spec("e1"))
    await reg.transition(
        exp.experiment_id, to_state="designed", actor_role="owner",
    )
    store.arm_crash()
    with pytest.raises(RuntimeError):
        await reg.pre_register(exp.experiment_id)
    # Stuck in designed + prereg sealed. Second call completes the
    # transition instead of raising PreRegistrationSealed.
    prereg = await reg.pre_register(exp.experiment_id)
    assert prereg.experiment_id == exp.experiment_id
    after = await reg.get(exp.experiment_id)
    assert after.state == "registered"
    # Third call is a pure no-op — still raises sealed because state
    # is now 'registered'.
    with pytest.raises(PreRegistrationSealed):
        await reg.pre_register(exp.experiment_id)


# ---------------------------------------------------------------------------
# Bug P — NATS envelope missing-field hardening
# ---------------------------------------------------------------------------


def test_envelope_verify_rejects_missing_required_fields():
    key = b"k" * 32
    clock = FakeClock()
    verifier = EnvelopeVerifier(
        hmac_key=key, skew_window_s=300, clock=clock,
    )
    # Produce a valid envelope first.
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    good = signer.sign(
        event_type="t", tenant_id="acme", payload={"a": 1},
    ).to_dict()
    # Drop each required field in turn.
    for field in ("schema_version", "event_type", "event_id",
                  "occurred_at", "tenant_id", "hmac"):
        stripped = {k: v for k, v in good.items() if k != field}
        with pytest.raises(NatsMessageIntegrity):
            verifier.verify(stripped)


def test_envelope_verify_rejects_non_string_hmac():
    key = b"k" * 32
    clock = FakeClock()
    verifier = EnvelopeVerifier(
        hmac_key=key, skew_window_s=300, clock=clock,
    )
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    env = signer.sign(
        event_type="t", tenant_id="acme", payload={},
    ).to_dict()
    env["hmac"] = 42  # type mismatch
    with pytest.raises(NatsMessageIntegrity):
        verifier.verify(env)


# ---------------------------------------------------------------------------
# Bug (cache ghost-variant)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_stale_cache_variant_does_not_leak_as_holdout():
    """A cached variant_id absent from the live spec must fall through.

    Prior to the fix, the assigner returned
    ``(variant=None, from_cache=True)`` which callers would interpret
    as a legitimate holdout — a silent bug that would shift users off
    the experiment they were supposed to be on.
    """
    spec = ExperimentSpec(
        name="v2",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True,
                    variant_id="v-new-control"),
            Variant(name="treatment", allocation_pct=50,
                    variant_id="v-new-treatment"),
        ),
    )
    cache = InMemoryAssignmentCache()
    # Pre-populate cache with a ghost variant id from a prior spec.
    await cache.put(
        experiment_id="exp-1", user_id="u-1",
        variant_id="v-OLD-ghost",
    )
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(0),
        cache=cache,
    )
    decision = await assigner.assign(
        user_id="u-1", experiment_id="exp-1", spec=spec,
    )
    # Must NOT be reported as "from_cache holdout".
    assert decision.variant is not None
    assert decision.variant.variant_id in (
        "v-new-control", "v-new-treatment",
    )
    assert decision.reason == "hash"
    assert not decision.from_cache


# ---------------------------------------------------------------------------
# Bug (bandit race) — concurrent apply_batch must not drop updates
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_bandit_apply_batch_is_atomic_under_concurrency():
    store = InMemoryBanditStateStore()
    await store.init("exp-1", ["a", "b"])
    updater = BanditUpdater(store=store)

    def mk(variant: str, idx: int) -> Outcome:
        return Outcome(
            outcome_id=f"o-{variant}-{idx}",
            experiment_id="exp-1",
            tenant_id="acme",
            user_id=f"u-{idx}",
            variant_id=variant,
            metric_name="ctr",
            metric_value=1.0,
            observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        )

    batch_a = [mk("a", i) for i in range(100)]
    batch_b = [mk("b", i) for i in range(100)]

    # Fire both in parallel — with the lock, total pulls_added should
    # equal exactly 200 (100 + 100), not less due to a lost write.
    s1, s2 = await asyncio.gather(
        updater.apply_batch(
            experiment_id="exp-1", outcomes=batch_a,
            primary_metric="ctr",
        ),
        updater.apply_batch(
            experiment_id="exp-1", outcomes=batch_b,
            primary_metric="ctr",
        ),
    )
    final = await store.load("exp-1")
    assert final["a"].n_pulls == 100
    assert final["b"].n_pulls == 100
    assert s1.processed_outcomes + s2.processed_outcomes == 200


# ---------------------------------------------------------------------------
# Bug (guardrail aging) — counter resets on long tick gap
# ---------------------------------------------------------------------------


def _outcome(
    value: float, *, variant: str, clock: FakeClock,
    metric: str = "latency_ms",
) -> Outcome:
    return Outcome(
        outcome_id=f"o-{variant}-{value}",
        experiment_id="e-1",
        tenant_id="acme",
        user_id="u",
        variant_id=variant,
        metric_name=metric,
        metric_value=value,
        observed_at=clock.now(),
    )


def test_guardrail_counter_ages_out_even_without_intervening_ticks():
    clock = FakeClock()
    watcher = GuardrailWatcher(aging_s=10, clock=clock)
    spec = GuardrailSpec(
        metric_name="latency_ms", direction="up", threshold_pct=30.0,
        window_s=3_600, consecutive_breaches=5,
    )
    runtime = GuardrailRuntime(spec=spec)
    # First breach at t=0.
    watcher.tick(
        runtime,
        control_outcomes=[_outcome(100.0, variant="c", clock=clock)],
        treatment_outcomes=[_outcome(200.0, variant="t", clock=clock)],
    )
    assert runtime.breaches_so_far == 1

    # Jump 1h forward — no ticks in between.
    clock.advance(3_600)
    # Age-out at start of next breach tick must reset counter to 0,
    # then the breach increments it to 1 (not 2).
    result = watcher.tick(
        runtime,
        control_outcomes=[_outcome(100.0, variant="c", clock=clock)],
        treatment_outcomes=[_outcome(200.0, variant="t", clock=clock)],
    )
    assert result.breached
    assert runtime.breaches_so_far == 1, (
        f"counter should have aged out then incremented to 1, "
        f"got {runtime.breaches_so_far}"
    )


def test_guardrail_zero_baseline_down_direction_no_false_breach():
    clock = FakeClock()
    watcher = GuardrailWatcher(aging_s=86_400, clock=clock)
    spec = GuardrailSpec(
        metric_name="ctr", direction="down", threshold_pct=20.0,
        window_s=3_600, consecutive_breaches=2,
    )
    runtime = GuardrailRuntime(spec=spec)
    # Control mean is 0 (all users failed to click in window).
    result = watcher.tick(
        runtime,
        control_outcomes=[_outcome(0.0, variant="c", metric="ctr",
                                   clock=clock)],
        treatment_outcomes=[_outcome(0.0, variant="t", metric="ctr",
                                     clock=clock)],
    )
    assert not result.breached
    assert result.reason == "zero_baseline_no_ratio"
    assert runtime.breaches_so_far == 0


def test_guardrail_zero_baseline_up_direction_treats_nonzero_as_breach():
    """For ``direction=up`` a non-zero treatment against a zero
    baseline IS a legitimate regression (0 errors → N errors)."""
    clock = FakeClock()
    watcher = GuardrailWatcher(aging_s=86_400, clock=clock)
    spec = GuardrailSpec(
        metric_name="error_rate", direction="up", threshold_pct=20.0,
        window_s=3_600, consecutive_breaches=1,
    )
    runtime = GuardrailRuntime(spec=spec)
    result = watcher.tick(
        runtime,
        control_outcomes=[_outcome(0.0, variant="c", metric="error_rate",
                                   clock=clock)],
        treatment_outcomes=[_outcome(0.1, variant="t", metric="error_rate",
                                     clock=clock)],
    )
    assert result.breached
    assert result.trips_abort


# ---------------------------------------------------------------------------
# Prereg constant-time verify — must not leak expected digest
# ---------------------------------------------------------------------------


def test_prereg_error_detail_does_not_leak_expected_len():
    """The :class:`PreRegistrationSignatureInvalid` detail dict must
    not include the expected HMAC length, which was previously leaked
    and could marginally aid timing analysis."""
    from efe_rollout.prereg import PreRegistration, verify

    prereg = PreRegistration(
        experiment_id="e-1",
        spec_hash=b"\x00" * 32,
        signature=b"wrong-sig-same-length-as-the-real-one!" * 2,
        plan_json='{"x": 1}',
        signing_key_id="kms/alias-1",
        created_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
    )
    try:
        verify(prereg, signing_key=b"k" * 32)
    except Exception as exc:
        detail = getattr(exc, "detail", {}) or {}
        assert "expected_len" not in detail
        assert "actual_len" not in detail


# ---------------------------------------------------------------------------
# Assigner: from_cache + variant integrity
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_cache_hit_path_never_returns_variant_is_none_with_from_cache_true():
    """Invariant: a from_cache decision always has a real variant OR is
    a proper holdout_cache decision. The combination
    ``(variant=None, from_cache=True, holdout=False)`` is forbidden."""
    spec = ExperimentSpec(
        name="t",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True,
                    variant_id="v-c"),
            Variant(name="treatment", allocation_pct=50,
                    variant_id="v-t"),
        ),
    )
    cache = InMemoryAssignmentCache()
    # Ghost variant scenario.
    await cache.put(
        experiment_id="exp", user_id="u",
        variant_id="v-ghost",
    )
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(0),
        cache=cache,
    )
    decision = await assigner.assign(
        user_id="u", experiment_id="exp", spec=spec,
    )
    # Forbidden state check.
    forbidden = (
        decision.variant is None
        and decision.from_cache
        and not decision.holdout
    )
    assert not forbidden, f"forbidden state: {decision!r}"

"""Guardrail rolling window + auto-abort trip logic."""

from __future__ import annotations



from efe_rollout.clock import FakeClock
from efe_rollout.experiment import GuardrailSpec
from efe_rollout.guardrails import GuardrailRuntime, GuardrailWatcher
from efe_rollout.records import Outcome


def _outcome(value: float, *, variant: str, metric: str,
             clock: FakeClock) -> Outcome:
    return Outcome(
        outcome_id=f"o-{metric}-{variant}-{value}",
        experiment_id="exp-1",
        tenant_id="acme",
        user_id="u",
        variant_id=variant,
        metric_name=metric,
        metric_value=value,
        observed_at=clock.now(),
    )


def test_guardrail_no_breach_when_ratios_within_bounds():
    clock = FakeClock()
    watcher = GuardrailWatcher(aging_s=86_400, clock=clock)
    spec = GuardrailSpec(
        metric_name="latency_ms",
        direction="up",
        threshold_pct=30.0,
        window_s=300,
        consecutive_breaches=3,
    )
    runtime = GuardrailRuntime(spec=spec)
    controls = [_outcome(100.0, variant="c", metric="latency_ms",
                         clock=clock)] * 5
    trts = [_outcome(105.0, variant="t", metric="latency_ms",
                     clock=clock)] * 5
    result = watcher.tick(
        runtime,
        control_outcomes=controls,
        treatment_outcomes=trts,
    )
    assert not result.breached
    assert runtime.breaches_so_far == 0


def test_guardrail_trips_abort_after_consecutive_breaches():
    clock = FakeClock()
    watcher = GuardrailWatcher(aging_s=86_400, clock=clock)
    spec = GuardrailSpec(
        metric_name="latency_ms",
        direction="up",
        threshold_pct=30.0,
        window_s=300,
        consecutive_breaches=3,
    )
    runtime = GuardrailRuntime(spec=spec)
    for i in range(3):
        clock.advance(10)  # stay inside window
        result = watcher.tick(
            runtime,
            control_outcomes=[_outcome(
                100.0, variant="c", metric="latency_ms", clock=clock,
            )],
            treatment_outcomes=[_outcome(
                200.0, variant="t", metric="latency_ms", clock=clock,
            )],
        )
        assert result.breached
        if i < 2:
            assert not result.trips_abort
    # Third breach trips abort.
    assert result.trips_abort
    assert runtime.breaches_so_far == 3


def test_guardrail_resets_on_non_breach():
    clock = FakeClock()
    watcher = GuardrailWatcher(aging_s=86_400, clock=clock)
    spec = GuardrailSpec(
        metric_name="latency_ms",
        direction="up",
        threshold_pct=30.0,
        window_s=300,
        consecutive_breaches=3,
    )
    runtime = GuardrailRuntime(spec=spec)
    # Breach
    watcher.tick(
        runtime,
        control_outcomes=[_outcome(100.0, variant="c",
                                    metric="latency_ms", clock=clock)],
        treatment_outcomes=[_outcome(200.0, variant="t",
                                      metric="latency_ms", clock=clock)],
    )
    assert runtime.breaches_so_far == 1
    # A subsequent OK tick does NOT reset, but keeps counter intact.
    clock.advance(5)
    watcher.tick(
        runtime,
        control_outcomes=[_outcome(100.0, variant="c",
                                    metric="latency_ms", clock=clock)],
        treatment_outcomes=[_outcome(110.0, variant="t",
                                      metric="latency_ms", clock=clock)],
    )
    assert runtime.breaches_so_far == 1


def test_guardrail_down_direction():
    clock = FakeClock()
    watcher = GuardrailWatcher(aging_s=86_400, clock=clock)
    spec = GuardrailSpec(
        metric_name="ctr",
        direction="down",
        threshold_pct=20.0,
        window_s=300,
        consecutive_breaches=2,
    )
    runtime = GuardrailRuntime(spec=spec)
    res = watcher.tick(
        runtime,
        control_outcomes=[_outcome(1.0, variant="c", metric="ctr",
                                    clock=clock)],
        treatment_outcomes=[_outcome(0.7, variant="t", metric="ctr",
                                      clock=clock)],
    )
    assert res.breached


def test_guardrail_ages_out_counter():
    clock = FakeClock()
    watcher = GuardrailWatcher(aging_s=10, clock=clock)
    spec = GuardrailSpec(
        metric_name="x", direction="up", threshold_pct=30.0,
        window_s=10_000, consecutive_breaches=5,
    )
    runtime = GuardrailRuntime(spec=spec)
    # Produce a breach.
    watcher.tick(
        runtime,
        control_outcomes=[_outcome(100.0, variant="c", metric="x",
                                    clock=clock)],
        treatment_outcomes=[_outcome(200.0, variant="t", metric="x",
                                      clock=clock)],
    )
    assert runtime.breaches_so_far == 1
    # 20 s later, benign tick — counter should age out.
    clock.advance(20)
    res = watcher.tick(
        runtime,
        control_outcomes=[_outcome(100.0, variant="c", metric="x",
                                    clock=clock)],
        treatment_outcomes=[_outcome(100.0, variant="t", metric="x",
                                      clock=clock)],
    )
    assert not res.breached
    assert runtime.breaches_so_far == 0


def test_guardrail_handles_insufficient_data():
    clock = FakeClock()
    watcher = GuardrailWatcher(aging_s=86_400, clock=clock)
    spec = GuardrailSpec(
        metric_name="x", direction="up", threshold_pct=30.0,
        window_s=300, consecutive_breaches=3,
    )
    runtime = GuardrailRuntime(spec=spec)
    res = watcher.tick(
        runtime,
        control_outcomes=[],
        treatment_outcomes=[],
    )
    assert not res.breached
    assert res.reason == "insufficient_window_data"

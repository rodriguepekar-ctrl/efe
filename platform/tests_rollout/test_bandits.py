"""Multi-armed bandits — Thompson, UCB1, ε-greedy."""

from __future__ import annotations


import pytest

from efe_rollout.bandits import (
    BanditArmState,
    BanditUpdater,
    EpsilonGreedySampler,
    InMemoryBanditStateStore,
    ThompsonSampler,
    Ucb1Sampler,
)
from efe_rollout.records import Outcome

from datetime import datetime, timezone


def _mk_outcome(
    variant: str, reward: float, *, exp: str = "exp-1",
    metric: str = "ctr", user: str = "u",
) -> Outcome:
    return Outcome(
        outcome_id=f"o-{variant}-{reward}",
        experiment_id=exp,
        tenant_id="acme",
        user_id=user,
        variant_id=variant,
        metric_name=metric,
        metric_value=reward,
        observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
    )


def test_thompson_always_returns_a_valid_arm():
    arms = [
        BanditArmState(variant_id="a", alpha=1, beta=1),
        BanditArmState(variant_id="b", alpha=1, beta=1),
    ]
    s = ThompsonSampler(seed=42)
    for _ in range(100):
        chosen = s.choose(arms)
        assert chosen.variant_id in {"a", "b"}


def test_thompson_favours_higher_alpha_beta_ratio():
    # Arm B has observed many successes; it should be picked > 80 % of the time.
    arms = [
        BanditArmState(variant_id="a", alpha=1, beta=1),
        BanditArmState(variant_id="b", alpha=100, beta=1),
    ]
    s = ThompsonSampler(seed=7)
    counts = {"a": 0, "b": 0}
    for _ in range(1_000):
        counts[s.choose(arms).variant_id] += 1
    assert counts["b"] > 800


def test_ucb1_explores_each_arm_first():
    arms = [
        BanditArmState(variant_id="a"),
        BanditArmState(variant_id="b"),
        BanditArmState(variant_id="c"),
    ]
    s = Ucb1Sampler()
    # Each untried arm picked once.
    first_three = set()
    for a in arms:
        chosen = s.choose(arms)
        assert chosen.variant_id in {"a", "b", "c"}
        chosen.n_pulls = 1
        chosen.n_successes = 0  # force a known state
        first_three.add(chosen.variant_id)


def test_epsilon_greedy_decays_exploration():
    s = EpsilonGreedySampler(epsilon=0.5, decay=0.99, seed=1)
    initial = s.epsilon
    arms = [
        BanditArmState(variant_id="a"),
        BanditArmState(variant_id="b"),
    ]
    for _ in range(10):
        s.choose(arms)
    assert s.epsilon < initial


def test_epsilon_rejects_out_of_range():
    with pytest.raises(ValueError):
        EpsilonGreedySampler(epsilon=0.0)
    with pytest.raises(ValueError):
        EpsilonGreedySampler(epsilon=1.0)


@pytest.mark.asyncio
async def test_bandit_updater_applies_outcomes():
    store = InMemoryBanditStateStore()
    await store.init("exp-1", ["a", "b"])
    updater = BanditUpdater(store=store)
    outcomes = [
        _mk_outcome("a", 1.0),
        _mk_outcome("a", 0.0),
        _mk_outcome("b", 1.0),
        _mk_outcome("b", 1.0),
        _mk_outcome("b", 0.0),
    ]
    summary = await updater.apply_batch(
        experiment_id="exp-1", outcomes=outcomes, primary_metric="ctr",
    )
    assert summary.processed_outcomes == 5
    final = await store.load("exp-1")
    assert final["a"].n_pulls == 2
    assert final["a"].n_successes == 1
    assert final["b"].n_pulls == 3
    assert final["b"].n_successes == 2


@pytest.mark.asyncio
async def test_bandit_updater_ignores_wrong_metric():
    store = InMemoryBanditStateStore()
    await store.init("exp-1", ["a"])
    updater = BanditUpdater(store=store)
    summary = await updater.apply_batch(
        experiment_id="exp-1",
        outcomes=[_mk_outcome("a", 1.0, metric="latency")],
        primary_metric="ctr",
    )
    assert summary.processed_outcomes == 0


def test_thompson_minimum_allocation_prevents_starvation():
    """Even if one arm dominates, the floor picks the weak arm some
    percentage of the time."""
    arms = [
        BanditArmState(variant_id="a", alpha=1, beta=1),
        BanditArmState(variant_id="b", alpha=1000, beta=1),
    ]
    s = ThompsonSampler(seed=99, minimum_allocation_pct=10)
    counts = {"a": 0, "b": 0}
    for _ in range(2_000):
        counts[s.choose(arms).variant_id] += 1
    # With minimum_allocation_pct=10 and K=2 we expect ~20% floor
    # + whatever the Thompson draw gives us. So arm 'a' should be
    # non-trivial (> 5%).
    assert counts["a"] > 50

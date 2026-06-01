"""α-spending functions — Pocock and O'Brien-Fleming."""

from __future__ import annotations

import pytest

from efe_rollout.analysis.sequential import AlphaSpending


def test_pocock_cumulative_alpha_is_bounded_by_alpha():
    sp = AlphaSpending(kind="pocock", alpha=0.05, total_looks=5)
    assert sp.alpha_spent(5) <= 0.05 + 1e-9


def test_obf_is_conservative_early_liberal_late():
    sp = AlphaSpending(kind="obf", alpha=0.05, total_looks=5)
    early = sp.alpha_spent(1)
    late = sp.alpha_spent(5)
    # OBF spends almost no alpha at peek 1, most of it at peek K.
    assert early < 0.01
    assert late > 0.04


def test_critical_z_decreases_over_peeks_obf():
    sp = AlphaSpending(kind="obf", alpha=0.05, total_looks=5)
    # early peek needs a very extreme z-score
    z_early = sp.critical_z(1)
    z_late = sp.critical_z(5)
    assert z_early > z_late
    assert z_late < 2.0


def test_pocock_critical_z_roughly_constant():
    sp = AlphaSpending(kind="pocock", alpha=0.05, total_looks=5)
    zs = [sp.critical_z(k) for k in range(1, 6)]
    # Pocock is a flat boundary.
    assert max(zs) - min(zs) < 1e-9


def test_should_stop_on_large_z():
    sp = AlphaSpending(kind="obf", alpha=0.05, total_looks=3)
    assert sp.should_stop(3, z_stat=5.0)
    assert not sp.should_stop(1, z_stat=1.0)


def test_rejects_invalid_peek():
    sp = AlphaSpending(kind="pocock", alpha=0.05, total_looks=3)
    with pytest.raises(ValueError):
        sp.alpha_spent(0)
    with pytest.raises(ValueError):
        sp.alpha_spent(4)

"""Stratified dispatcher — exact distribution at low percentages."""

from __future__ import annotations

import pytest

from efe_registry.core.artifact import ArtifactRole
from efe_registry.errors import RoutingInvariantBroken
from efe_registry.router.dispatcher import StratifiedDispatcher
from efe_registry.router.routing_table import RoutingEntry


def test_exact_distribution_at_1_percent():
    d = StratifiedDispatcher()
    entries = (
        RoutingEntry("champ", 99, ArtifactRole.CHAMPION),
        RoutingEntry("cand", 1, ArtifactRole.CANDIDATE),
    )
    counts = {"champ": 0, "cand": 0}
    for _ in range(1000):
        pick = d.pick(entries)
        counts[pick.endpoint_id] += 1
    # At 1% with stratified dispatch on 1000 calls we get EXACTLY 10.
    assert counts["cand"] == 10


def test_exact_distribution_at_5_percent():
    d = StratifiedDispatcher()
    entries = (
        RoutingEntry("champ", 95, ArtifactRole.CHAMPION),
        RoutingEntry("cand", 5, ArtifactRole.CANDIDATE),
    )
    n = 500
    cand = sum(
        1 for _ in range(n)
        if d.pick(entries).endpoint_id == "cand"
    )
    assert cand == 25


def test_rejects_non_100_sum():
    d = StratifiedDispatcher()
    entries = (
        RoutingEntry("champ", 50, ArtifactRole.CHAMPION),
        RoutingEntry("cand", 30, ArtifactRole.CANDIDATE),
    )
    with pytest.raises(RoutingInvariantBroken):
        d.pick(entries)


def test_excludes_shadow_and_standby():
    d = StratifiedDispatcher()
    entries = (
        RoutingEntry("champ", 100, ArtifactRole.CHAMPION),
        RoutingEntry("sh", 0, ArtifactRole.SHADOW),
        RoutingEntry("sw", 0, ArtifactRole.STANDBY_WARM),
    )
    picks = {d.pick(entries).endpoint_id for _ in range(50)}
    assert picks == {"champ"}


def test_rejects_if_no_dispatchable_entries():
    d = StratifiedDispatcher()
    with pytest.raises(RoutingInvariantBroken):
        d.pick((
            RoutingEntry("sh", 100, ArtifactRole.SHADOW),
        ))

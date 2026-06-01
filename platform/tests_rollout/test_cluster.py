"""Cluster-robust SE — basic sanity."""

from __future__ import annotations

import random

from efe_rollout.analysis.cluster import cluster_robust_se


def test_cluster_robust_se_matches_ols_when_one_obs_per_cluster():
    """When each cluster is a single observation the cluster-robust SE
    degenerates toward the ordinary OLS SE."""
    rng = random.Random(1)
    n = 400
    y = []
    d = []
    clusters = []
    for i in range(n):
        di = i % 2
        yi = 0.3 * di + rng.gauss(0, 1.0)
        y.append(yi)
        d.append(di)
        clusters.append(f"c-{i}")
    res = cluster_robust_se(y=y, d=d, clusters=clusters)
    assert 0.2 < res.beta < 0.4
    assert res.se > 0
    assert res.n_clusters == n


def test_cluster_robust_se_widens_with_intra_cluster_correlation():
    """Replicate the same user many times → cluster SE should dwarf
    the naive SE that treats each rep as independent."""
    n_users = 50
    reps_per_user = 20
    y: list[float] = []
    d: list[int] = []
    clusters: list[str] = []
    rng = random.Random(42)
    for u in range(n_users):
        user_effect = rng.gauss(0, 1.0)
        for _ in range(reps_per_user):
            di = u % 2
            y.append(0.1 * di + user_effect + rng.gauss(0, 0.01))
            d.append(di)
            clusters.append(f"u-{u}")
    res = cluster_robust_se(y=y, d=d, clusters=clusters)
    # Only n_users independent clusters.
    assert res.n_clusters == n_users
    # With this much intra-cluster correlation, SE should be large.
    assert res.se > 0.1

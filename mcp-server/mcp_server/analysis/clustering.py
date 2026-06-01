"""Clustering helpers — k-means / hierarchical for `efe_suggest_classes`.

scikit-learn is loaded lazily so the default `pip install` (without
the `[advanced]` extra) doesn't pull it. Callers wrap calls in
try/except ImportError and fall back to a deterministic naïve method.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np


@dataclass(frozen=True)
class ClusterResult:
    cluster_id: int
    member_indices: list[int]
    centroid: np.ndarray
    intra_cohesion: float          # mean cosine intra-cluster
    nearest_examples: list[int]    # top-5 examples nearest to centroid


def _l2_normalize(v: np.ndarray) -> np.ndarray:
    n = np.linalg.norm(v)
    return v / n if n > 0 else v


def _intra_cohesion(centroid: np.ndarray, members: np.ndarray) -> float:
    if len(members) == 0:
        return 0.0
    sims = members @ centroid
    return float(np.mean(sims))


def cluster_kmeans(
    *,
    embeddings: np.ndarray,
    n_clusters: int,
    random_state: int = 42,
) -> list[ClusterResult]:
    """Cluster L2-normalized embeddings into n_clusters using MiniBatchKMeans.

    Falls back to a naïve random-pick + nearest-centroid loop if
    scikit-learn isn't available (less precise but always works).
    """
    n = len(embeddings)
    if n == 0:
        return []
    if n_clusters > n:
        n_clusters = n
    n_clusters = max(2, n_clusters)

    try:
        from sklearn.cluster import MiniBatchKMeans

        model = MiniBatchKMeans(
            n_clusters=n_clusters,
            random_state=random_state,
            batch_size=min(256, n),
            n_init=3,
        )
        labels = model.fit_predict(embeddings)
        centroids = np.array(
            [_l2_normalize(model.cluster_centers_[i]) for i in range(n_clusters)],
            dtype=np.float32,
        )
    except ImportError:
        # Deterministic fallback : pick `n_clusters` initial points spaced
        # by max-cosine-distance, then assign each point to its nearest.
        rng = np.random.default_rng(random_state)
        idx = list(rng.choice(n, size=1))
        for _ in range(n_clusters - 1):
            sims_to_chosen = embeddings @ embeddings[idx].T
            dist = 1.0 - sims_to_chosen.max(axis=1)
            for j in idx:
                dist[j] = -1.0
            next_pick = int(np.argmax(dist))
            idx.append(next_pick)
        centroids = np.array([_l2_normalize(embeddings[i]) for i in idx], dtype=np.float32)
        sims = embeddings @ centroids.T
        labels = np.argmax(sims, axis=1)

    results: list[ClusterResult] = []
    for cid in range(n_clusters):
        members = np.where(labels == cid)[0].tolist()
        if not members:
            continue
        member_arr = embeddings[members]
        recomputed = _l2_normalize(member_arr.mean(axis=0))
        cohesion = _intra_cohesion(recomputed, member_arr)
        # 5 nearest examples to centroid
        sims = member_arr @ recomputed
        order = np.argsort(-sims)[:5]
        nearest = [members[int(i)] for i in order]
        results.append(
            ClusterResult(
                cluster_id=cid,
                member_indices=members,
                centroid=recomputed,
                intra_cohesion=round(cohesion, 4),
                nearest_examples=nearest,
            )
        )
    return results


def detect_outliers(
    *,
    embeddings: np.ndarray,
    clusters: list[ClusterResult],
    sigma_threshold: float = 1.5,
) -> list[int]:
    """Return indices whose distance to nearest centroid is > sigma_threshold σ."""
    if not clusters or len(embeddings) == 0:
        return []
    centroids = np.array([c.centroid for c in clusters])
    sims = embeddings @ centroids.T
    nearest_sim = sims.max(axis=1)
    distances = 1.0 - nearest_sim
    if len(distances) < 3:
        return []
    mean = float(np.mean(distances))
    std = float(np.std(distances))
    threshold = mean + sigma_threshold * std
    return [int(i) for i, d in enumerate(distances) if d > threshold]

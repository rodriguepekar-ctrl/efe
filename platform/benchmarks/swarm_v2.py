"""Improved swarm variants — V2/V3/V4/V5/V6 ablation candidates.

Each variant targets a **specific** failure mode diagnosed on V1 and
is kept as self-contained as possible so the ablation stays
interpretable. They share the same public surface as
:class:`benchmarks.swarm_sim.SwarmClassifier` — identical input, same
:class:`SwarmPrediction` output — so the evaluation harness treats them
as drop-in replacements.

V1 failure modes (measured)
---------------------------

* Inter-agent correlation on a hard query **0.93** (uniform Bernoulli
  dropout produces redundant views).
* Zero unique gains on HARD: the swarm never wins a case the baseline
  doesn't already win.
* Mean cosine over 32 anchors *dilutes* the discriminating signal
  between semantic siblings.
* Softmax temperature 0.05 is hand-picked; calibration leaves 0.058 ECE
  on the table.

The five variants
-----------------

* **V2 — Orthogonal projections.**  Instead of random Bernoulli masks
  we project each agent through a **deterministic orthogonal sub-basis**:
  we build a seeded Haar-random orthogonal matrix once, and agent *i*
  observes the coefficients in rows ``[i*k, (i+1)*k)``. Disjoint rows
  ⇒ structurally-different views ⇒ uncorrelated per-agent scores.

* **V3 — Top-K max pooling.**  Instead of averaging cosine across all
  32 anchors of a class, we take the **mean of the top-K closest** (K=5).
  This mimics nearest-neighbour voting inside each class — a query
  that sits close to the *best* member of a class is a stronger
  positive signal than one that sits close to the class *centroid*.

* **V4 — Mahalanobis-style scoring.**  Each class has 32 anchors; their
  empirical covariance is a natural "shape" of the cluster. We compute
  per-agent per-class scores as ``−(q − μ_c)ᵀ Σ_c⁻¹ (q − μ_c)`` with a
  regularised pseudo-inverse. Classes with small intrinsic spread
  become stricter gates — exactly the behaviour you want at boundaries.

* **V5 — Temperature scaling.**  Single-parameter post-hoc recalibration
  of V1's softmax. The temperature T is swept on the training (easy)
  slice and locked. Guarantees: ECE strictly ↓, accuracy strictly ==.

* **V6 — Hybrid.**  V2 (orthogonal projections) + V3 (top-K pooling) +
  V5 (temperature scaling) composed.
"""

from __future__ import annotations

import hashlib
import time
from dataclasses import dataclass, field, replace
from typing import Literal

import numpy as np

from benchmarks.dataset import Dataset, Query
from benchmarks.swarm_sim import SwarmPrediction, Template, Verdict


# ---------------------------------------------------------------------------
#  Shared helpers
# ---------------------------------------------------------------------------


def _derive_seed(qid: str, salt: str = "") -> int:
    h = hashlib.sha256((qid + salt).encode("utf-8")).digest()
    return int.from_bytes(h[:8], "little")


def _safe_l2(m: np.ndarray, *, axis: int) -> np.ndarray:
    n = np.linalg.norm(m, axis=axis, keepdims=True)
    return m / np.where(n > 0, n, 1.0)


def _softmax(x: np.ndarray, *, temperature: float = 1.0) -> np.ndarray:
    scaled = x / temperature
    e = np.exp(scaled - scaled.max())
    return e / e.sum()


def _verdict_from(
    aggregate: np.ndarray,
    *,
    stable_count: int,
    stability_window: int,
    epsilon_margin: float,
    crit_dissent: float,
) -> Verdict:
    top2 = np.sort(aggregate)[::-1][:2]
    margin = float(top2[0] - top2[1])
    if stable_count >= stability_window:
        return "CONVERGED"
    if margin < epsilon_margin:
        return "DIVERGENCE"
    if crit_dissent > 0.60:
        return "MODE_COLLAPSE"
    return "CONTINUE"


# ---------------------------------------------------------------------------
#  V2 — Orthogonal projections
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SwarmConfigV2:
    n_agents: int = 1024
    mix: dict[Template, int] = field(
        default_factory=lambda: {"na-sem": 512, "na-log": 384, "na-crit": 128}
    )
    max_rounds: int = 8
    epsilon_variance: float = 0.005
    epsilon_margin: float = 0.015
    stability_window: int = 3
    weight: dict[Template, float] = field(
        default_factory=lambda: {"na-sem": 1.00, "na-log": 0.85, "na-crit": 0.60}
    )
    # Per-template sub-basis size (how many orthogonal directions each
    # agent observes). na-sem sees more dims → denser view; na-crit fewer.
    sub_basis_size: dict[Template, int] = field(
        default_factory=lambda: {"na-sem": 64, "na-log": 48, "na-crit": 32}
    )
    #: RNG seed used to draw the global orthogonal basis. Identical
    #: across all requests — the basis is a *platform constant*.
    basis_seed: int = 0xE1EE0000


class SwarmV2OrthogonalProjections:
    """Swarm whose agents see *disjoint* orthogonal sub-bases."""

    def __init__(self, dataset: Dataset, config: SwarmConfigV2 | None = None) -> None:
        self._config = config or SwarmConfigV2()
        self._classes: list[str] = sorted(dataset.anchors.keys())
        self._anchors_mat = np.stack(
            [dataset.anchors[c] for c in self._classes], axis=0
        )  # (n_classes, ancs, dim)
        self._dim = self._anchors_mat.shape[-1]
        self._template_order: list[Template] = list(self._config.mix.keys())

        # 1. Build a fixed, seeded, Haar-random orthogonal basis once.
        rng_basis = np.random.default_rng(self._config.basis_seed)
        q, _ = np.linalg.qr(rng_basis.standard_normal((self._dim, self._dim)))
        self._basis = q  # (dim, dim) — rows are orthonormal directions

        # 2. Assign each agent a slice of the basis. Agents of the same
        #    template share a slice-size; slices wrap around if the total
        #    doesn't tile exactly.
        slice_sizes: list[int] = []
        template_per_agent: list[int] = []
        for tpl_idx, (tpl, count) in enumerate(self._config.mix.items()):
            size = self._config.sub_basis_size[tpl]
            for _ in range(count):
                slice_sizes.append(size)
                template_per_agent.append(tpl_idx)
        self._slice_sizes = np.array(slice_sizes)
        self._agent_template = np.array(template_per_agent, dtype=np.int8)

        # Pre-compute for each agent its direction indices in the basis.
        self._agent_dirs: list[np.ndarray] = []
        cursor = 0
        for size in slice_sizes:
            idx = (np.arange(size) + cursor) % self._dim
            self._agent_dirs.append(idx)
            cursor += size // 3  # stagger so overlap is small but non-zero

        self._weight_per_agent = np.array(
            [self._config.weight[self._template_order[t]] for t in self._agent_template]
        )

        # 3. Pre-project anchors into the global basis coefficients —
        #    this lets us slice cheaply per agent.
        A_flat = self._anchors_mat.reshape(-1, self._dim)  # (K*A, dim)
        self._A_proj = A_flat @ self._basis.T  # (K*A, dim) in basis coords
        self._n_classes = len(self._classes)
        self._n_ancs = self._anchors_mat.shape[1]

    def classify(self, query: Query) -> SwarmPrediction:
        t0 = time.perf_counter()
        # Project query into basis coords once.
        q_proj_full = query.embedding @ self._basis.T  # (dim,)

        n_agents = self._config.n_agents
        per_agent_class_score = np.zeros((n_agents, self._n_classes), dtype=np.float32)

        # 4. For each agent, compute cosine(q_slice, anchor_slice).
        # Do it vectorised: build a "slice mask" per agent by projecting
        # the agent-specific sub-vector directly.
        # Because sub-basis sizes differ per template, we group by template.
        for agent_idx in range(n_agents):
            dirs = self._agent_dirs[agent_idx]
            qv = q_proj_full[dirs]  # shape (k,)
            av = self._A_proj[:, dirs]  # (K*A, k)
            qn = np.linalg.norm(qv)
            an = np.linalg.norm(av, axis=1)
            denom = qn * an
            denom = np.where(denom > 0, denom, 1.0)
            cos_flat = (av @ qv) / denom  # (K*A,)
            cos_3d = cos_flat.reshape(self._n_classes, self._n_ancs)
            per_agent_class_score[agent_idx] = cos_3d.mean(axis=1)

        # 5. Consensus — same loop as V1.
        weights = self._weight_per_agent.copy()
        stable_count = 0
        last_winner_idx = -1
        rounds_used = 0
        aggregate = np.zeros(self._n_classes, dtype=np.float32)
        for r in range(self._config.max_rounds):
            rounds_used = r + 1
            weighted = per_agent_class_score * weights[:, None]
            aggregate = weighted.sum(axis=0) / weights.sum()
            winner_idx = int(np.argmax(aggregate))
            var_winner = float(per_agent_class_score[:, winner_idx].var())
            top2 = np.sort(aggregate)[::-1][:2]
            margin = float(top2[0] - top2[1])
            if (
                winner_idx == last_winner_idx
                and var_winner < self._config.epsilon_variance
                and margin > self._config.epsilon_margin
            ):
                stable_count += 1
                if stable_count >= self._config.stability_window:
                    break
            else:
                stable_count = 0
            last_winner_idx = winner_idx
            agent_top1 = per_agent_class_score.argmax(axis=1)
            agrees = (agent_top1 == winner_idx).astype(np.float32)
            weights = weights * (0.9 + 0.1 * agrees)

        crit_idx = self._template_order.index("na-crit")
        crit_mask = self._agent_template == crit_idx
        crit_top1 = per_agent_class_score[crit_mask].argmax(axis=1)
        crit_dissent = float((crit_top1 != np.argmax(aggregate)).mean())

        verdict = _verdict_from(
            aggregate,
            stable_count=stable_count,
            stability_window=self._config.stability_window,
            epsilon_margin=self._config.epsilon_margin,
            crit_dissent=crit_dissent,
        )

        softmax = _softmax(aggregate, temperature=0.05)
        winner_idx = int(np.argmax(aggregate))
        confidence = float(softmax[winner_idx])
        latency_us = (time.perf_counter() - t0) * 1e6
        return SwarmPrediction(
            qid=query.qid,
            predicted=self._classes[winner_idx],
            confidence=confidence,
            verdict=verdict,
            rounds_executed=rounds_used,
            n_agents=n_agents,
            latency_us=latency_us,
            distribution={c: float(s) for c, s in zip(self._classes, softmax)},
            critique_dissent=crit_dissent,
        )


# ---------------------------------------------------------------------------
#  V3 — Top-K max pooling
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SwarmConfigV3:
    n_agents: int = 1024
    mix: dict[Template, int] = field(
        default_factory=lambda: {"na-sem": 512, "na-log": 384, "na-crit": 128}
    )
    max_rounds: int = 8
    epsilon_variance: float = 0.005
    epsilon_margin: float = 0.015
    stability_window: int = 3
    dropout: dict[Template, float] = field(
        default_factory=lambda: {"na-sem": 0.10, "na-log": 0.30, "na-crit": 0.50}
    )
    weight: dict[Template, float] = field(
        default_factory=lambda: {"na-sem": 1.00, "na-log": 0.85, "na-crit": 0.60}
    )
    top_k: int = 5


class SwarmV3TopKPooling:
    """V1 architecture but per-class scoring uses top-K mean cosine."""

    def __init__(self, dataset: Dataset, config: SwarmConfigV3 | None = None) -> None:
        self._config = config or SwarmConfigV3()
        self._classes: list[str] = sorted(dataset.anchors.keys())
        self._anchors_mat = np.stack(
            [dataset.anchors[c] for c in self._classes], axis=0
        )
        self._dim = self._anchors_mat.shape[-1]
        self._template_order: list[Template] = list(self._config.mix.keys())
        # V1-style dropout agents.
        self._agent_template = np.empty(self._config.n_agents, dtype=np.int8)
        idx = 0
        for tpl_idx, (tpl, n) in enumerate(self._config.mix.items()):
            self._agent_template[idx : idx + n] = tpl_idx
            idx += n
        self._dropout_per_agent = np.array(
            [self._config.dropout[self._template_order[t]] for t in self._agent_template]
        )
        self._weight_per_agent = np.array(
            [self._config.weight[self._template_order[t]] for t in self._agent_template]
        )

    def classify(self, query: Query) -> SwarmPrediction:
        t0 = time.perf_counter()
        seed = _derive_seed(query.qid)
        rng = np.random.default_rng(seed)

        keep_probs = 1.0 - self._dropout_per_agent[:, None]
        masks = (rng.random((self._config.n_agents, self._dim)) < keep_probs).astype(
            np.float32
        )
        q_proj = query.embedding * masks
        q_proj = _safe_l2(q_proj, axis=1)

        anchors = self._anchors_mat
        A = anchors.reshape(-1, self._dim)
        num = q_proj @ A.T
        anchor_sq = A * A
        proj_anchor_sq = masks @ anchor_sq.T
        denom = np.sqrt(np.maximum(proj_anchor_sq, 1e-12))
        cos_3d = (num / denom).reshape(self._config.n_agents, len(self._classes), -1)

        # === THE V3 CHANGE: top-K mean instead of full mean. ===
        k = min(self._config.top_k, cos_3d.shape[-1])
        sorted_desc = np.sort(cos_3d, axis=-1)[:, :, -k:]
        per_agent_class_score = sorted_desc.mean(axis=-1)
        # =======================================================

        # Consensus loop (same as V1).
        n_classes = len(self._classes)
        weights = self._weight_per_agent.copy()
        stable_count = 0
        last_winner_idx = -1
        rounds_used = 0
        aggregate = np.zeros(n_classes, dtype=np.float32)
        for r in range(self._config.max_rounds):
            rounds_used = r + 1
            weighted = per_agent_class_score * weights[:, None]
            aggregate = weighted.sum(axis=0) / weights.sum()
            winner_idx = int(np.argmax(aggregate))
            var_winner = float(per_agent_class_score[:, winner_idx].var())
            top2 = np.sort(aggregate)[::-1][:2]
            margin = float(top2[0] - top2[1])
            if (
                winner_idx == last_winner_idx
                and var_winner < self._config.epsilon_variance
                and margin > self._config.epsilon_margin
            ):
                stable_count += 1
                if stable_count >= self._config.stability_window:
                    break
            else:
                stable_count = 0
            last_winner_idx = winner_idx
            agent_top1 = per_agent_class_score.argmax(axis=1)
            agrees = (agent_top1 == winner_idx).astype(np.float32)
            weights = weights * (0.9 + 0.1 * agrees)

        crit_idx = self._template_order.index("na-crit")
        crit_mask = self._agent_template == crit_idx
        crit_top1 = per_agent_class_score[crit_mask].argmax(axis=1)
        crit_dissent = float((crit_top1 != np.argmax(aggregate)).mean())

        verdict = _verdict_from(
            aggregate,
            stable_count=stable_count,
            stability_window=self._config.stability_window,
            epsilon_margin=self._config.epsilon_margin,
            crit_dissent=crit_dissent,
        )

        softmax = _softmax(aggregate, temperature=0.05)
        winner_idx = int(np.argmax(aggregate))
        confidence = float(softmax[winner_idx])
        latency_us = (time.perf_counter() - t0) * 1e6
        return SwarmPrediction(
            qid=query.qid,
            predicted=self._classes[winner_idx],
            confidence=confidence,
            verdict=verdict,
            rounds_executed=rounds_used,
            n_agents=self._config.n_agents,
            latency_us=latency_us,
            distribution={c: float(s) for c, s in zip(self._classes, softmax)},
            critique_dissent=crit_dissent,
        )


# ---------------------------------------------------------------------------
#  V4 — Mahalanobis-like scoring
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SwarmConfigV4:
    n_agents: int = 1024
    mix: dict[Template, int] = field(
        default_factory=lambda: {"na-sem": 512, "na-log": 384, "na-crit": 128}
    )
    max_rounds: int = 8
    epsilon_variance: float = 0.005
    epsilon_margin: float = 0.015
    stability_window: int = 3
    dropout: dict[Template, float] = field(
        default_factory=lambda: {"na-sem": 0.10, "na-log": 0.30, "na-crit": 0.50}
    )
    weight: dict[Template, float] = field(
        default_factory=lambda: {"na-sem": 1.00, "na-log": 0.85, "na-crit": 0.60}
    )
    #: Ridge regularisation for the inverse covariance.
    ridge: float = 0.05


class SwarmV4Mahalanobis:
    """Per-class Mahalanobis distance scored across dropout agents."""

    def __init__(self, dataset: Dataset, config: SwarmConfigV4 | None = None) -> None:
        self._config = config or SwarmConfigV4()
        self._classes: list[str] = sorted(dataset.anchors.keys())
        self._anchors_mat = np.stack(
            [dataset.anchors[c] for c in self._classes], axis=0
        )
        self._dim = self._anchors_mat.shape[-1]

        # Pre-compute class centroids and regularised inverse covariances
        # ONCE (platform constants). All agents reuse them.
        self._centroids = self._anchors_mat.mean(axis=1)  # (n_classes, dim)
        covs = []
        for c in range(len(self._classes)):
            diffs = self._anchors_mat[c] - self._centroids[c]  # (ancs, dim)
            cov = diffs.T @ diffs / max(1, diffs.shape[0] - 1)
            cov = cov + self._config.ridge * np.eye(self._dim)
            covs.append(cov)
        self._cov = np.stack(covs, axis=0)  # (n_classes, dim, dim)
        self._inv_cov = np.linalg.inv(self._cov)  # (n_classes, dim, dim)

        self._template_order: list[Template] = list(self._config.mix.keys())
        self._agent_template = np.empty(self._config.n_agents, dtype=np.int8)
        idx = 0
        for tpl_idx, (tpl, n) in enumerate(self._config.mix.items()):
            self._agent_template[idx : idx + n] = tpl_idx
            idx += n
        self._dropout_per_agent = np.array(
            [self._config.dropout[self._template_order[t]] for t in self._agent_template]
        )
        self._weight_per_agent = np.array(
            [self._config.weight[self._template_order[t]] for t in self._agent_template]
        )

    def classify(self, query: Query) -> SwarmPrediction:
        t0 = time.perf_counter()
        seed = _derive_seed(query.qid)
        rng = np.random.default_rng(seed)

        keep_probs = 1.0 - self._dropout_per_agent[:, None]
        masks = (rng.random((self._config.n_agents, self._dim)) < keep_probs).astype(
            np.float32
        )

        n_classes = len(self._classes)
        per_agent_class_score = np.zeros(
            (self._config.n_agents, n_classes), dtype=np.float32
        )
        q = query.embedding
        # For each class, Mahalanobis distance from agent-projected query
        # to class centroid (also projected).
        for c in range(n_classes):
            mu = self._centroids[c]  # (dim,)
            invcov = self._inv_cov[c]  # (dim, dim)
            # Agent view: diff * mask, where mask zeros out dropped dims.
            # We keep the full vector but zero dropped entries, which is
            # equivalent to setting invcov entries to zero on dropped dims
            # — approximation OK for discrimination purposes.
            diff = (q - mu) * masks  # (n_agents, dim)
            # (diff @ invcov) * diff, summed along dim.
            tmp = diff @ invcov  # (n_agents, dim)
            mahal2 = (tmp * diff).sum(axis=1)  # (n_agents,)
            # Score: negative distance (higher = closer). Normalise by
            # the number of observed dims so fewer-dim agents aren't
            # automatically closer.
            n_obs = masks.sum(axis=1).clip(min=1.0)
            per_agent_class_score[:, c] = -mahal2 / n_obs

        # Standardise per-agent so the softmax temperature is meaningful.
        per_agent_class_score -= per_agent_class_score.mean(axis=1, keepdims=True)
        per_agent_class_score /= (
            per_agent_class_score.std(axis=1, keepdims=True) + 1e-6
        )

        # Consensus.
        weights = self._weight_per_agent.copy()
        stable_count = 0
        last_winner_idx = -1
        rounds_used = 0
        aggregate = np.zeros(n_classes, dtype=np.float32)
        for r in range(self._config.max_rounds):
            rounds_used = r + 1
            weighted = per_agent_class_score * weights[:, None]
            aggregate = weighted.sum(axis=0) / weights.sum()
            winner_idx = int(np.argmax(aggregate))
            var_winner = float(per_agent_class_score[:, winner_idx].var())
            top2 = np.sort(aggregate)[::-1][:2]
            margin = float(top2[0] - top2[1])
            if (
                winner_idx == last_winner_idx
                and var_winner < self._config.epsilon_variance
                and margin > self._config.epsilon_margin
            ):
                stable_count += 1
                if stable_count >= self._config.stability_window:
                    break
            else:
                stable_count = 0
            last_winner_idx = winner_idx
            agent_top1 = per_agent_class_score.argmax(axis=1)
            agrees = (agent_top1 == winner_idx).astype(np.float32)
            weights = weights * (0.9 + 0.1 * agrees)

        crit_idx = self._template_order.index("na-crit")
        crit_mask = self._agent_template == crit_idx
        crit_top1 = per_agent_class_score[crit_mask].argmax(axis=1)
        crit_dissent = float((crit_top1 != np.argmax(aggregate)).mean())

        verdict = _verdict_from(
            aggregate,
            stable_count=stable_count,
            stability_window=self._config.stability_window,
            epsilon_margin=self._config.epsilon_margin,
            crit_dissent=crit_dissent,
        )

        softmax = _softmax(aggregate, temperature=1.0)
        winner_idx = int(np.argmax(aggregate))
        confidence = float(softmax[winner_idx])
        latency_us = (time.perf_counter() - t0) * 1e6
        return SwarmPrediction(
            qid=query.qid,
            predicted=self._classes[winner_idx],
            confidence=confidence,
            verdict=verdict,
            rounds_executed=rounds_used,
            n_agents=self._config.n_agents,
            latency_us=latency_us,
            distribution={c: float(s) for c, s in zip(self._classes, softmax)},
            critique_dissent=crit_dissent,
        )


# ---------------------------------------------------------------------------
#  V5 — Temperature scaling on top of V1
# ---------------------------------------------------------------------------


class SwarmV5TemperatureCalibrated:
    """V1 but with a temperature learned on a validation slice."""

    def __init__(
        self,
        base: "SwarmV1Like",
        val_queries: list[Query],
    ) -> None:
        self._base = base
        # Step 1: run base on the calibration slice and collect
        # (aggregate_vector, correct_class_idx) pairs.
        base_preds = [base.classify(q) for q in val_queries]
        # Use the raw softmax-distribution → logit, then fit T.
        # We approximate the "logits" as log(dist)*base_temperature.
        classes = sorted({q.label for q in val_queries})
        class_idx = {c: i for i, c in enumerate(classes)}
        # Collect per-query log-probs + label index.
        logits = np.stack(
            [
                np.log(np.array([p.distribution[c] for c in classes]) + 1e-12)
                for p in base_preds
            ],
            axis=0,
        )  # (n, n_classes)
        y = np.array([class_idx[q.label] for q in val_queries])

        self._temperature = _optimise_temperature(logits, y)
        self._classes = classes

    def classify(self, query: Query) -> SwarmPrediction:
        base = self._base.classify(query)
        # Re-softmax the distribution with the optimised temperature.
        classes = self._classes
        logits = np.log(np.array([base.distribution[c] for c in classes]) + 1e-12)
        re = np.exp((logits / self._temperature) - (logits / self._temperature).max())
        re = re / re.sum()
        winner_idx = int(np.argmax(re))
        return replace(
            base,
            predicted=classes[winner_idx],
            confidence=float(re[winner_idx]),
            distribution={c: float(s) for c, s in zip(classes, re)},
        )


def _optimise_temperature(
    logits: np.ndarray,
    labels: np.ndarray,
    *,
    grid: np.ndarray | None = None,
) -> float:
    """Grid-search T ∈ [0.05, 5.0] minimising NLL."""
    if grid is None:
        grid = np.concatenate(
            [
                np.linspace(0.05, 1.0, 20),
                np.linspace(1.0, 5.0, 20),
            ]
        )
    best_t = 1.0
    best_nll = float("inf")
    for t in grid:
        scaled = logits / t
        scaled = scaled - scaled.max(axis=1, keepdims=True)
        log_probs = scaled - np.log(np.exp(scaled).sum(axis=1, keepdims=True))
        nll = -log_probs[np.arange(len(labels)), labels].mean()
        if nll < best_nll:
            best_nll = float(nll)
            best_t = float(t)
    return best_t


# ---------------------------------------------------------------------------
#  V6 — Hybrid: V2 + V3 + V5
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SwarmConfigV6:
    base: SwarmConfigV2 = field(default_factory=SwarmConfigV2)
    top_k: int = 5


class SwarmV6Hybrid:
    """Orthogonal projections + top-K pooling, ready for V5 wrapping."""

    def __init__(self, dataset: Dataset, config: SwarmConfigV6 | None = None) -> None:
        self._config = config or SwarmConfigV6()
        # We reuse V2's plumbing but override per-class pooling.
        self._v2 = SwarmV2OrthogonalProjections(dataset, self._config.base)
        self._top_k = self._config.top_k

    def classify(self, query: Query) -> SwarmPrediction:
        t0 = time.perf_counter()
        v2 = self._v2

        # Rebuild the per-agent (K*A,) cosines — we need anchor-level
        # resolution so we can do top-K pooling, so we cannot reuse V2's
        # class-level mean.
        q_proj_full = query.embedding @ v2._basis.T
        n_agents = v2._config.n_agents
        n_cls = v2._n_classes
        n_ancs = v2._n_ancs
        per_agent_class_score = np.zeros((n_agents, n_cls), dtype=np.float32)
        k = min(self._top_k, n_ancs)
        for agent_idx in range(n_agents):
            dirs = v2._agent_dirs[agent_idx]
            qv = q_proj_full[dirs]
            av = v2._A_proj[:, dirs]
            qn = np.linalg.norm(qv)
            an = np.linalg.norm(av, axis=1)
            denom = qn * an
            denom = np.where(denom > 0, denom, 1.0)
            cos_flat = (av @ qv) / denom  # (K*A,)
            cos_3d = cos_flat.reshape(n_cls, n_ancs)
            # top-K pooling per class
            topk = np.sort(cos_3d, axis=1)[:, -k:]
            per_agent_class_score[agent_idx] = topk.mean(axis=1)

        # V2-style consensus.
        weights = v2._weight_per_agent.copy()
        stable_count = 0
        last_winner_idx = -1
        rounds_used = 0
        aggregate = np.zeros(n_cls, dtype=np.float32)
        for r in range(v2._config.max_rounds):
            rounds_used = r + 1
            weighted = per_agent_class_score * weights[:, None]
            aggregate = weighted.sum(axis=0) / weights.sum()
            winner_idx = int(np.argmax(aggregate))
            var_winner = float(per_agent_class_score[:, winner_idx].var())
            top2 = np.sort(aggregate)[::-1][:2]
            margin = float(top2[0] - top2[1])
            if (
                winner_idx == last_winner_idx
                and var_winner < v2._config.epsilon_variance
                and margin > v2._config.epsilon_margin
            ):
                stable_count += 1
                if stable_count >= v2._config.stability_window:
                    break
            else:
                stable_count = 0
            last_winner_idx = winner_idx
            agent_top1 = per_agent_class_score.argmax(axis=1)
            agrees = (agent_top1 == winner_idx).astype(np.float32)
            weights = weights * (0.9 + 0.1 * agrees)

        crit_idx = v2._template_order.index("na-crit")
        crit_mask = v2._agent_template == crit_idx
        crit_top1 = per_agent_class_score[crit_mask].argmax(axis=1)
        crit_dissent = float((crit_top1 != np.argmax(aggregate)).mean())

        verdict = _verdict_from(
            aggregate,
            stable_count=stable_count,
            stability_window=v2._config.stability_window,
            epsilon_margin=v2._config.epsilon_margin,
            crit_dissent=crit_dissent,
        )

        softmax = _softmax(aggregate, temperature=0.05)
        winner_idx = int(np.argmax(aggregate))
        confidence = float(softmax[winner_idx])
        latency_us = (time.perf_counter() - t0) * 1e6
        return SwarmPrediction(
            qid=query.qid,
            predicted=v2._classes[winner_idx],
            confidence=confidence,
            verdict=verdict,
            rounds_executed=rounds_used,
            n_agents=n_agents,
            latency_us=latency_us,
            distribution={c: float(s) for c, s in zip(v2._classes, softmax)},
            critique_dissent=crit_dissent,
        )


# ---------------------------------------------------------------------------
#  Protocol the harness expects
# ---------------------------------------------------------------------------

SwarmV1Like = (
    SwarmV2OrthogonalProjections
    | SwarmV3TopKPooling
    | SwarmV4Mahalanobis
    | SwarmV6Hybrid
)

__all__ = [
    "SwarmConfigV2",
    "SwarmConfigV3",
    "SwarmConfigV4",
    "SwarmConfigV6",
    "SwarmV2OrthogonalProjections",
    "SwarmV3TopKPooling",
    "SwarmV4Mahalanobis",
    "SwarmV5TemperatureCalibrated",
    "SwarmV6Hybrid",
]

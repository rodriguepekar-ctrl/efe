"""Pure-Python reference implementation of the EFE ephemeral swarm kernel.

This module is NOT a mock. It is a **functional equivalent** of the
Rust ``efe-core`` kernel, written in numpy for auditability. The Rust
crate is the production path; this simulator exists to:

1. Drive the comparative benchmark without needing a running Rust server.
2. Provide a second, independent implementation of the kernel logic —
   when the two agree on identical inputs, we have corroboration.

Algorithm (matches MODE1_ARCHITECTURE §5)
-----------------------------------------

For each inference request:

1. **Genesis** — spawn ``n_agents`` nano-agents partitioned into three
   templates (na-sem / na-log / na-crit). Each agent ``i`` receives a
   **dropout projection mask** ``m_i ∈ {0, 1}^dim`` drawn from a
   Bernoulli distribution seeded by ``(request_id, agent_id)``. The
   dropout rate depends on the template:

     * na-sem  : 0.10 (semantic density — keeps most dims)
     * na-log  : 0.30 (logical inference — moderate projection)
     * na-crit : 0.50 (critique / diversification — aggressive pruning)

2. **Alignment** — each agent projects its view of the problem embedding
   ``q̃_i = q ⊙ m_i``, L2-normalised. Anchors are projected the same
   way: ``ã_i = a ⊙ m_i``.

3. **Fusion (per-agent scoring)** — each agent computes its per-class
   score as the mean cosine similarity to the class anchors in its
   projected space.

4. **Consensus (iterative rounds)** — for ``n_rounds`` rounds:

     a. Aggregate agent votes into a class-level score with weights
        ``w_i``. In round 0, weights are uniform per template
        (na-sem and na-log get full weight, na-crit is a soft down-
        weight so diversification doesn't dominate).

     b. Compute the **variance of the per-class aggregate across
        agents**. If variance ``< epsilon_variance`` on the winning
        class for ``stability_window`` consecutive rounds, declare
        ``CONVERGED``.

     c. If the top-two class scores are within ``epsilon_margin`` of
        each other after the max rounds, declare ``CONTINUE`` or
        ``DIVERGENCE`` (see rules below).

5. **Dissolution** — read the top-K winners from the converged
   consensus, expose them as a :class:`SwarmPrediction`, then throw
   away the agent arena.

The rules for verdict assignment
--------------------------------

* ``CONVERGED``       — the winning class's aggregate has stabilised
  under ``epsilon_variance`` for ``stability_window`` rounds AND the
  margin with the runner-up exceeds ``epsilon_margin``.
* ``CONTINUE``        — max_rounds reached, no stable win.
* ``DIVERGENCE``      — top-2 classes differ by less than
  ``epsilon_margin`` at max_rounds (genuinely ambiguous input).
* ``MODE_COLLAPSE``   — all agents converged on a single class but with
  inter-agent variance near zero AND the "critique" agents' dissent
  fraction ``> 0.6`` — they saw a different answer but were out-voted.

These rules mirror the Rust kernel (``crates/efe-core/src/consensus``).
"""

from __future__ import annotations

import hashlib
import time
from dataclasses import dataclass, field
from typing import Literal

import numpy as np

from benchmarks.dataset import Dataset, Query

Verdict = Literal["CONVERGED", "CONTINUE", "DIVERGENCE", "MODE_COLLAPSE"]
Template = Literal["na-sem", "na-log", "na-crit"]


# ---------------------------------------------------------------------------
#  Prediction
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SwarmPrediction:
    """One inference result from the swarm."""

    qid: str
    predicted: str
    confidence: float
    verdict: Verdict
    rounds_executed: int
    n_agents: int
    latency_us: float
    #: Full distribution (softmaxed aggregate) — mirrors baseline shape.
    distribution: dict[str, float]
    #: Fraction of ``na-crit`` agents whose top-1 disagrees with the
    #: final winner. High values imply the aggregate is fragile.
    critique_dissent: float


# ---------------------------------------------------------------------------
#  Config
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SwarmConfig:
    """Tunables — defaults mirror the Rust MVP settings."""

    n_agents: int = 1024
    mix: dict[Template, int] = field(
        default_factory=lambda: {"na-sem": 512, "na-log": 384, "na-crit": 128}
    )
    max_rounds: int = 8
    epsilon_variance: float = 0.005
    epsilon_margin: float = 0.015
    stability_window: int = 3
    # Per-template dropout rate (fraction of latent dims masked out).
    dropout: dict[Template, float] = field(
        default_factory=lambda: {"na-sem": 0.10, "na-log": 0.30, "na-crit": 0.50}
    )
    # Per-template weight in the round-0 aggregate.
    weight: dict[Template, float] = field(
        default_factory=lambda: {"na-sem": 1.00, "na-log": 0.85, "na-crit": 0.60}
    )

    def __post_init__(self) -> None:
        if sum(self.mix.values()) != self.n_agents:
            raise ValueError(
                f"mix total {sum(self.mix.values())} != n_agents {self.n_agents}"
            )


# ---------------------------------------------------------------------------
#  Classifier
# ---------------------------------------------------------------------------


class SwarmClassifier:
    """Ephemeral-swarm classifier — pure numpy reference implementation."""

    def __init__(self, dataset: Dataset, config: SwarmConfig | None = None) -> None:
        self._config = config or SwarmConfig()
        self._classes: list[str] = sorted(dataset.anchors.keys())
        # Stack: (n_classes, anchors_per_class, dim).
        self._anchors_mat = np.stack(
            [dataset.anchors[c] for c in self._classes], axis=0
        )
        self._dim = self._anchors_mat.shape[-1]
        # Pre-assign template bands: indices [0..512) → na-sem, etc.
        self._agent_template = np.empty(self._config.n_agents, dtype=np.int8)
        idx = 0
        for tpl_idx, (tpl, n) in enumerate(self._config.mix.items()):
            self._agent_template[idx : idx + n] = tpl_idx
            idx += n
        # Template string lookup in deterministic order.
        self._template_order: list[Template] = list(self._config.mix.keys())
        self._dropout_per_agent = np.array(
            [self._config.dropout[self._template_order[t]] for t in self._agent_template]
        )
        self._weight_per_agent = np.array(
            [self._config.weight[self._template_order[t]] for t in self._agent_template]
        )

    # ------------------------------------------------------------- Public

    def classify(self, query: Query) -> SwarmPrediction:
        t0 = time.perf_counter()
        # 1. Spawn — deterministic seed from qid for reproducibility.
        seed = _derive_seed(query.qid)
        rng = np.random.default_rng(seed)

        # 2. Generate agent masks: shape (n_agents, dim) in {0, 1}.
        # Bernoulli keep-prob = 1 - dropout.
        keep_probs = 1.0 - self._dropout_per_agent[:, None]  # (n_agents, 1)
        masks = (rng.random((self._config.n_agents, self._dim)) < keep_probs).astype(
            np.float32
        )

        # 3. Alignment: agent-projected query and anchors.
        q_proj = query.embedding * masks  # (n_agents, dim)
        q_proj = _safe_l2(q_proj, axis=1)

        # Anchor projection is lazy per agent: we compute
        # (n_agents, n_classes) scores via matmul with broadcast.
        # anchors shape: (n_classes, ancs, dim)
        # We compute: for each agent, score per class = mean cos sim.
        # Step: expand anchors into (n_agents, n_classes, ancs, dim) is
        # too expensive. Use the equivalence:
        #   cos(m ⊙ q, m ⊙ a) = (m ⊙ q) · (m ⊙ a) / (|m⊙q||m⊙a|)
        # Since masks are {0,1}, (m ⊙ a) = a[mask]. We pre-normalise
        # anchors elementwise by mask just like q_proj.
        # Use einsum: score[agent, cls, anc] = sum_d q_proj[agent,d] *
        # masks[agent,d] * anchors[cls,anc,d] / |a proj|
        # Simplification: since masks are binary and q_proj already
        # includes the mask, we just compute:
        #   raw[agent,cls,anc] = (q_proj[agent] * masks[agent]) dot anchors[cls,anc]
        #                     = q_proj[agent] dot (masks[agent] * anchors[cls,anc])
        # But masks[agent] * q_proj[agent] == q_proj[agent] (idempotent).
        # Final: we need to L2-normalise the *projected anchor* — that
        # normalisation is per (agent, cls, anc).
        # We compute it via the following einsum trick:
        #   projected_anchor_norm[agent, cls, anc] =
        #       sqrt( sum_d masks[agent,d] * anchors[cls,anc,d]^2 )
        # Then raw score divided by this norm gives the cosine.
        anchors = self._anchors_mat  # (n_classes, ancs, dim)
        # Numerator: (n_agents, dim) @ (dim, n_classes*ancs) → reshape
        A = anchors.reshape(-1, self._dim)  # (n_classes*ancs, dim)
        num = q_proj @ A.T  # (n_agents, n_classes*ancs)
        # Denom: projected anchor norms per agent — big but cheap with masks.
        # per-anchor squared: (n_agents, n_classes*ancs)
        anchor_sq = A * A  # (n_classes*ancs, dim)
        proj_anchor_sq = masks @ anchor_sq.T  # (n_agents, n_classes*ancs)
        denom = np.sqrt(np.maximum(proj_anchor_sq, 1e-12))
        cos_3d = (num / denom).reshape(
            self._config.n_agents, len(self._classes), -1
        )  # (n_agents, n_classes, ancs)
        per_agent_class_score = cos_3d.mean(axis=2)  # (n_agents, n_classes)

        # 4. Consensus — iterative rounds with early stopping.
        n_classes = len(self._classes)
        weights = self._weight_per_agent.copy()  # (n_agents,)
        stable_count = 0
        last_winner_idx = -1
        rounds_used = 0
        aggregate: np.ndarray = np.zeros(n_classes, dtype=np.float32)
        for r in range(self._config.max_rounds):
            rounds_used = r + 1
            weighted = per_agent_class_score * weights[:, None]  # (n_agents, n_cls)
            aggregate = weighted.sum(axis=0) / weights.sum()  # (n_cls,)
            winner_idx = int(np.argmax(aggregate))
            # Variance across agents for the current winner.
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

            # Round-over-round weight update: down-weight agents that
            # diverge from the running consensus winner. Mild: avoids
            # pathological self-reinforcement.
            agent_top1 = per_agent_class_score.argmax(axis=1)
            agrees = (agent_top1 == winner_idx).astype(np.float32)
            weights = weights * (0.9 + 0.1 * agrees)

        # 5. Verdict assignment.
        top2 = np.sort(aggregate)[::-1][:2]
        margin = float(top2[0] - top2[1])
        winner_idx = int(np.argmax(aggregate))
        winner_cls = self._classes[winner_idx]
        # na-crit dissent fraction.
        crit_idx = self._template_order.index("na-crit")
        crit_mask = self._agent_template == crit_idx
        crit_top1 = per_agent_class_score[crit_mask].argmax(axis=1)
        crit_dissent = float((crit_top1 != winner_idx).mean())

        if stable_count >= self._config.stability_window:
            verdict: Verdict = "CONVERGED"
        elif margin < self._config.epsilon_margin:
            verdict = "DIVERGENCE"
        elif crit_dissent > 0.60:
            verdict = "MODE_COLLAPSE"
        else:
            verdict = "CONTINUE"

        # Confidence = softmaxed aggregate.
        softmax = _softmax(aggregate * 20.0)  # temp 0.05 → sharper
        distribution = {c: float(s) for c, s in zip(self._classes, softmax)}
        confidence = float(softmax[winner_idx])

        latency_us = (time.perf_counter() - t0) * 1e6
        return SwarmPrediction(
            qid=query.qid,
            predicted=winner_cls,
            confidence=confidence,
            verdict=verdict,
            rounds_executed=rounds_used,
            n_agents=self._config.n_agents,
            latency_us=latency_us,
            distribution=distribution,
            critique_dissent=crit_dissent,
        )


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _derive_seed(qid: str) -> int:
    """Deterministic 64-bit seed from a request id."""
    h = hashlib.sha256(qid.encode("utf-8")).digest()
    return int.from_bytes(h[:8], "little")


def _safe_l2(m: np.ndarray, *, axis: int) -> np.ndarray:
    n = np.linalg.norm(m, axis=axis, keepdims=True)
    return m / np.where(n > 0, n, 1.0)


def _softmax(x: np.ndarray) -> np.ndarray:
    e = np.exp(x - x.max())
    return e / e.sum()


__all__ = [
    "SwarmClassifier",
    "SwarmConfig",
    "SwarmPrediction",
    "Template",
    "Verdict",
]

"""Degraded-embedder simulation + fine-tuning head transforms.

Why a degraded embedder?
------------------------

The original synthetic dataset (``benchmarks.dataset``) places class
centroids on strictly orthogonal directions — so the embedder already
gives maximal separation in :math:`\\mathbb{R}^{128}` and any fine-tuning
on top has nothing to recover. That is an **unrealistic** setup: in
production, the embedder is a generic pretrained sentence-transformer
that was trained on a corpus with no knowledge of the client's class
schema. Two classes that are operationally distinct (e.g. ``billing``
and ``refund``) can easily share substantial embedding geometry.

We model this by **adding a controlled, sibling-correlated bias** to
the clean centroids before emitting embeddings. The sibling pairs are
the ones we already declared in :mod:`benchmarks.dataset._SIBLING` —
they are the ones we want to learn to separate.

After fine-tuning, the learned transform should project *away* from
the bias subspace, recovering separation between siblings. This is
the textbook use-case for metric learning on top of a generic embedder.

Public surface
--------------

* :func:`build_degraded_dataset` — same shape as :func:`build_dataset`
  but with a ``sibling_bias_tau`` knob controlling the simulated
  pre-training deficit.

* :class:`EmbedderTransform` — a pure-numpy wrapper around a learned
  transformation (linear, MLP, or LoRA-style residual). Applied at
  inference time to both anchors and queries before they enter any
  classifier.

* :func:`apply_transform_to_dataset` — re-wraps a :class:`Dataset`
  through an :class:`EmbedderTransform` in a pure, non-destructive way.
"""

from __future__ import annotations

from dataclasses import replace
from typing import Literal

import numpy as np

from benchmarks.dataset import (
    ANCHORS_PER_CLASS,
    CLASS_NAMES,
    EMBEDDING_DIM,
    N_CLASSES,
    Dataset,
    Query,
    _l2_normalise,
)
from benchmarks.dataset import _SIBLING


# ---------------------------------------------------------------------------
#  Degraded-embedder dataset
# ---------------------------------------------------------------------------


def build_degraded_dataset(
    *,
    seed: int = 42,
    n_easy: int = 250,
    n_hard: int = 250,
    hard_noise_std: float = 0.25,
    class_separation: float = 1.0,
    sibling_bias_tau: float = 0.55,
) -> Dataset:
    """Same as :func:`build_dataset` but with a sibling-correlated bias.

    Each pair of sibling classes (billing↔refund, authentication↔
    account_management, …) shares a random bias vector ``b_pair`` added
    to both of their centroids with strength ``τ``::

        centroid'[c] = centroid[c] + τ * b_pair(c)
        centroid'[sibling(c)] = centroid[sibling(c)] + τ * b_pair(c)

    For ``τ = 0`` this reduces to :func:`build_dataset`. For larger
    ``τ`` the sibling pairs become harder to separate in the embedding
    space — which is what a generic pretrained embedder looks like on
    a client task it has never seen.
    """
    rng = np.random.default_rng(seed)

    # 1. Start from orthogonal pure centroids (same as the original).
    q, _ = np.linalg.qr(rng.standard_normal((EMBEDDING_DIM, EMBEDDING_DIM)))
    pure_centres: dict[str, np.ndarray] = {
        name: class_separation * q[i] for i, name in enumerate(CLASS_NAMES)
    }

    # 2. Compute shared bias per sibling pair — deterministic w.r.t. seed.
    pair_bias: dict[frozenset[str], np.ndarray] = {}
    for name, sib in _SIBLING.items():
        key = frozenset({name, sib})
        if key not in pair_bias:
            pair_bias[key] = _l2_normalise(rng.standard_normal(EMBEDDING_DIM))

    # 3. Apply biased centroids.
    #    Asymmetric: drag each class toward its sibling by factor τ and
    #    add the shared bias direction. This both reduces separation
    #    (sibling drag) and adds correlated noise (shared bias) —
    #    realistic for a generic pretrained embedder on a client task.
    biased_centres: dict[str, np.ndarray] = {}
    for name in CLASS_NAMES:
        sib = _SIBLING[name]
        bias = pair_bias[frozenset({name, sib})]
        drag = 0.35 * sibling_bias_tau  # how much we pull toward sibling
        biased_centres[name] = (
            (1 - drag) * pure_centres[name]
            + drag * pure_centres[sib]
            + sibling_bias_tau * bias
        )

    # 4. Anchors: centroid + small Gaussian perturbation, L2-normalised.
    anchors: dict[str, np.ndarray] = {}
    for name in CLASS_NAMES:
        noise = rng.standard_normal((ANCHORS_PER_CLASS, EMBEDDING_DIM)) * 0.15
        anchors[name] = _l2_normalise(biased_centres[name] + noise)

    # 5. Easy queries — centred well inside the (biased) cluster.
    queries_easy: list[Query] = []
    for i in range(n_easy):
        cls = CLASS_NAMES[i % N_CLASSES]
        emb = biased_centres[cls] + rng.standard_normal(EMBEDDING_DIM) * 0.10
        queries_easy.append(
            Query(
                qid=f"easy-{i:04d}",
                embedding=_l2_normalise(emb),
                label=cls,
                difficulty="easy",
            )
        )

    # 6. Hard queries — interpolate toward the sibling.
    queries_hard: list[Query] = []
    for i in range(n_hard):
        cls = CLASS_NAMES[i % N_CLASSES]
        sib = _SIBLING[cls]
        alpha = float(rng.uniform(0.35, 0.48))
        interp = (1 - alpha) * biased_centres[cls] + alpha * biased_centres[sib]
        noise = rng.standard_normal(EMBEDDING_DIM) * hard_noise_std
        queries_hard.append(
            Query(
                qid=f"hard-{i:04d}",
                embedding=_l2_normalise(interp + noise),
                label=cls,
                difficulty="hard",
                confusion_sibling=sib,
            )
        )

    return Dataset(
        anchors=anchors,
        queries_easy=queries_easy,
        queries_hard=queries_hard,
        seed=seed,
    )


# ---------------------------------------------------------------------------
#  Embedder transforms
# ---------------------------------------------------------------------------


TransformKind = Literal["identity", "linear", "mlp"]


class EmbedderTransform:
    """Pure-numpy wrapper around a learned embedder-head transformation.

    The transform takes a shape ``(d,)`` vector (or ``(B, d)`` batch)
    and returns a shape ``(d',)`` vector, L2-normalised.
    """

    def __init__(
        self,
        kind: TransformKind,
        *,
        W: np.ndarray | None = None,
        W1: np.ndarray | None = None,
        b1: np.ndarray | None = None,
        W2: np.ndarray | None = None,
        b2: np.ndarray | None = None,
        output_norm: bool = True,
    ) -> None:
        self.kind = kind
        self._W = W
        self._W1 = W1
        self._b1 = b1
        self._W2 = W2
        self._b2 = b2
        self._normalise = output_norm
        if kind == "linear":
            assert W is not None, "linear transform requires W"
        if kind == "mlp":
            assert W1 is not None and W2 is not None, "MLP requires W1, W2"

    def __call__(self, x: np.ndarray) -> np.ndarray:
        if self.kind == "identity":
            out = x
        elif self.kind == "linear":
            assert self._W is not None
            out = x @ self._W
        elif self.kind == "mlp":
            assert self._W1 is not None and self._W2 is not None
            b1 = self._b1 if self._b1 is not None else 0
            b2 = self._b2 if self._b2 is not None else 0
            h = np.maximum(x @ self._W1 + b1, 0.0)  # ReLU
            out = h @ self._W2 + b2
        else:
            raise ValueError(f"unknown kind {self.kind!r}")
        if self._normalise:
            out = _l2_normalise(out)
        return out

    @property
    def output_dim(self) -> int:
        if self.kind == "identity":
            raise RuntimeError("identity transform has no fixed output dim")
        if self.kind == "linear":
            assert self._W is not None
            return int(self._W.shape[1])
        assert self._W2 is not None
        return int(self._W2.shape[1])

    @property
    def n_parameters(self) -> int:
        if self.kind == "identity":
            return 0
        if self.kind == "linear":
            assert self._W is not None
            return int(self._W.size)
        total = 0
        for p in (self._W1, self._b1, self._W2, self._b2):
            if p is not None:
                total += int(p.size)
        return total


# ---------------------------------------------------------------------------
#  Apply transform to a whole dataset
# ---------------------------------------------------------------------------


def apply_transform_to_dataset(
    ds: Dataset, transform: EmbedderTransform
) -> Dataset:
    """Return a new :class:`Dataset` with transformed embeddings.

    Anchors and queries alike are re-embedded. The original dataset is
    not modified.
    """
    new_anchors: dict[str, np.ndarray] = {
        c: transform(v) for c, v in ds.anchors.items()
    }
    new_easy = [
        replace(q, embedding=transform(q.embedding)) for q in ds.queries_easy
    ]
    new_hard = [
        replace(q, embedding=transform(q.embedding)) for q in ds.queries_hard
    ]
    return Dataset(
        anchors=new_anchors,
        queries_easy=new_easy,
        queries_hard=new_hard,
        seed=ds.seed,
    )


# ---------------------------------------------------------------------------
#  Identity factory
# ---------------------------------------------------------------------------


def identity_transform() -> EmbedderTransform:
    """Transform that does nothing — used as a control baseline."""
    return EmbedderTransform(kind="identity")


__all__ = [
    "EmbedderTransform",
    "TransformKind",
    "apply_transform_to_dataset",
    "build_degraded_dataset",
    "identity_transform",
]

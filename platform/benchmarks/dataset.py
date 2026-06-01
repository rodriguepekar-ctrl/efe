"""Reproducible synthetic dataset of customer-support queries.

Design constraints
------------------

* **Deterministic** — every embedding is produced by a seeded RNG, so
  repeated runs of the benchmark give *identical* numbers.

* **Controlled difficulty** — two difficulty tiers:

  1. ``easy``  — the query embedding sits clearly in one class cluster,
     well separated from its neighbours.
  2. ``hard``  — the query embedding is placed on the **decision
     boundary** between two semantically related classes, with
     additive Gaussian noise so the label is ambiguous.

  The hard tier is where the swarm's multi-agent consensus is
  expected to out-perform the naïve baseline.

* **Honest** — we deliberately use a reproducible pseudo-embedder
  instead of a downloadable sentence-transformer. This removes
  "black-box qualité du modèle" from the comparison — both the
  baseline and the swarm see the **same** embeddings and we measure
  the **classifier** differential, not the embedder differential.

The dataset shape
-----------------

* 8 classes (support categories).
* 32 **anchor** embeddings per class — the "training" set.
* ``n_test`` query embeddings per difficulty tier (default 250 each,
  500 total). Each query has a canonical ground-truth label.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Final

import numpy as np

# ---------------------------------------------------------------------------
#  Public shape
# ---------------------------------------------------------------------------


EMBEDDING_DIM: Final[int] = 128
N_CLASSES: Final[int] = 8
ANCHORS_PER_CLASS: Final[int] = 32
CLASS_NAMES: Final[tuple[str, ...]] = (
    "billing",
    "authentication",
    "shipping",
    "refund",
    "technical_bug",
    "account_management",
    "complaint",
    "general_inquiry",
)

#: For each class, the **closest** sibling in semantic space.
#: The hard-tier queries are placed on the decision boundary between
#: these pairs — these are where naïve cosine fails most often.
_SIBLING: Final[dict[str, str]] = {
    "billing": "refund",
    "refund": "billing",
    "authentication": "account_management",
    "account_management": "authentication",
    "shipping": "complaint",
    "complaint": "shipping",
    "technical_bug": "general_inquiry",
    "general_inquiry": "technical_bug",
}


# ---------------------------------------------------------------------------
#  Data structures
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class Dataset:
    """A fully-materialised benchmark dataset."""

    #: ``class_id -> array[ANCHORS_PER_CLASS, EMBEDDING_DIM]``.
    anchors: dict[str, np.ndarray]

    #: Test queries, each tagged ``easy`` / ``hard``.
    queries_easy: list["Query"]
    queries_hard: list["Query"]

    #: Absolute RNG seed used to build the set.
    seed: int

    def all_queries(self) -> list["Query"]:
        return list(self.queries_easy) + list(self.queries_hard)


@dataclass(frozen=True, slots=True)
class Query:
    """A single benchmark query."""

    qid: str
    embedding: np.ndarray
    label: str
    difficulty: str  # "easy" | "hard"
    # For hard queries, the sibling class the query is pulled toward.
    confusion_sibling: str | None = None


# ---------------------------------------------------------------------------
#  Construction
# ---------------------------------------------------------------------------


def build_dataset(
    *,
    seed: int = 42,
    n_easy: int = 250,
    n_hard: int = 250,
    hard_noise_std: float = 0.25,
    class_separation: float = 1.0,
) -> Dataset:
    """Materialise the full benchmark dataset.

    Parameters
    ----------
    seed
        Absolute RNG seed. Used for class-centre sampling, anchor
        perturbation, and query generation — identical seeds produce
        identical datasets down to the bit.
    n_easy, n_hard
        Number of queries per difficulty tier.
    hard_noise_std
        Standard deviation of the Gaussian noise added to hard-tier
        queries. Higher ⇒ harder.
    class_separation
        Scale factor on the class-centre norms — larger values push
        classes further apart in latent space.
    """
    rng = np.random.default_rng(seed)

    # 1. Class centres: 8 orthogonal-ish directions in R^EMBEDDING_DIM.
    #    We use the rows of a random orthogonal matrix, scaled.
    q, _ = np.linalg.qr(rng.standard_normal((EMBEDDING_DIM, EMBEDDING_DIM)))
    centres: dict[str, np.ndarray] = {}
    for i, name in enumerate(CLASS_NAMES):
        centres[name] = class_separation * q[i]

    # 2. Anchors per class: centre + small Gaussian perturbation.
    anchors: dict[str, np.ndarray] = {}
    for name in CLASS_NAMES:
        noise = rng.standard_normal((ANCHORS_PER_CLASS, EMBEDDING_DIM)) * 0.15
        anchors[name] = _l2_normalise(centres[name] + noise)

    # 3. Easy queries: a fresh perturbation around the centre, smaller
    #    than the anchor spread so they sit comfortably inside the cluster.
    queries_easy: list[Query] = []
    for i in range(n_easy):
        cls = CLASS_NAMES[i % N_CLASSES]
        emb = centres[cls] + rng.standard_normal(EMBEDDING_DIM) * 0.10
        queries_easy.append(
            Query(
                qid=f"easy-{i:04d}",
                embedding=_l2_normalise(emb),
                label=cls,
                difficulty="easy",
            )
        )

    # 4. Hard queries: interpolate toward the sibling centre + add
    #    Gaussian noise. α=0.5 puts the point exactly on the decision
    #    boundary; we sample α ~ Uniform(0.35, 0.48) to stay **inside**
    #    the correct class but close to the boundary.
    queries_hard: list[Query] = []
    for i in range(n_hard):
        cls = CLASS_NAMES[i % N_CLASSES]
        sib = _SIBLING[cls]
        alpha = float(rng.uniform(0.35, 0.48))
        interp = (1 - alpha) * centres[cls] + alpha * centres[sib]
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
#  Helpers
# ---------------------------------------------------------------------------


def _l2_normalise(v: np.ndarray) -> np.ndarray:
    """L2-normalise a vector or each row of a 2-D matrix."""
    if v.ndim == 1:
        n = np.linalg.norm(v)
        return v / n if n > 0 else v
    norms = np.linalg.norm(v, axis=1, keepdims=True)
    # Avoid division by zero; anchors with zero norm never happen in practice.
    return v / np.where(norms > 0, norms, 1.0)


__all__ = [
    "ANCHORS_PER_CLASS",
    "CLASS_NAMES",
    "Dataset",
    "EMBEDDING_DIM",
    "N_CLASSES",
    "Query",
    "build_dataset",
]

"""Inter-annotator agreement — Cohen's κ (pairwise) + Fleiss' κ (N-rater).

Both implementations are pure numpy, no external deps. They match
the reference formulas from Cohen 1960 and Fleiss 1971.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass

import numpy as np

from efe_ft.errors import LabelingError


@dataclass(frozen=True, slots=True)
class KappaReport:
    """Structured IAA result — ready for audit or API response."""

    kappa: float
    n_agreements: int
    n_samples: int
    n_classes: int

    @property
    def agreement_rate(self) -> float:
        return self.n_agreements / max(1, self.n_samples)

    def to_dict(self) -> dict[str, float | int]:
        return {
            "kappa": round(self.kappa, 4),
            "agreement_rate": round(self.agreement_rate, 4),
            "n_agreements": self.n_agreements,
            "n_samples": self.n_samples,
            "n_classes": self.n_classes,
        }


def cohen_kappa(a: Iterable[str], b: Iterable[str]) -> KappaReport:
    """Pairwise Cohen κ.

    ``a`` and ``b`` are parallel sequences of categorical labels
    (strings). Returns a :class:`KappaReport`.
    """
    a_list = list(a)
    b_list = list(b)
    if len(a_list) != len(b_list):
        raise LabelingError(
            f"cohen_kappa: sequences differ in length "
            f"({len(a_list)} vs {len(b_list)})",
        )
    n = len(a_list)
    if n == 0:
        return KappaReport(kappa=0.0, n_agreements=0, n_samples=0, n_classes=0)
    classes = sorted(set(a_list) | set(b_list))
    k = len(classes)
    cls_idx = {c: i for i, c in enumerate(classes)}
    conf = np.zeros((k, k), dtype=np.int64)
    for ai, bi in zip(a_list, b_list, strict=True):
        conf[cls_idx[ai], cls_idx[bi]] += 1
    agreements = int(conf.trace())
    po = agreements / n
    pa = conf.sum(axis=1) / n
    pb = conf.sum(axis=0) / n
    pe = float((pa * pb).sum())
    kappa = 1.0 if pe == 1.0 else (po - pe) / (1.0 - pe)
    return KappaReport(
        kappa=float(kappa), n_agreements=agreements,
        n_samples=n, n_classes=k,
    )


def pairwise_kappas(
    labels_by_annotator: dict[str, dict[str, str]],
) -> dict[tuple[str, str], KappaReport]:
    """Compute Cohen κ for every pair of annotators over their overlap.

    Parameters
    ----------
    labels_by_annotator
        ``{annotator_id: {sample_id: label}}``.

    Returns
    -------
    dict[(a, b), KappaReport] with keys ordered ``(a, b)`` where
    ``a < b``. Pairs with no overlap are omitted.
    """
    annotators = sorted(labels_by_annotator.keys())
    out: dict[tuple[str, str], KappaReport] = {}
    for i, a in enumerate(annotators):
        for b in annotators[i + 1 :]:
            shared = set(labels_by_annotator[a]) & set(labels_by_annotator[b])
            if not shared:
                continue
            a_labels = [labels_by_annotator[a][s] for s in sorted(shared)]
            b_labels = [labels_by_annotator[b][s] for s in sorted(shared)]
            out[(a, b)] = cohen_kappa(a_labels, b_labels)
    return out


def fleiss_kappa(ratings: np.ndarray) -> KappaReport:
    """Fleiss κ for multi-annotator agreement.

    ``ratings`` is shape ``(N_items, N_categories)`` holding the COUNT
    of annotators that voted for each category on each item. All rows
    must sum to the same total (same number of raters per item).
    """
    if ratings.ndim != 2:
        raise LabelingError(
            f"fleiss_kappa: ratings must be 2-D, got {ratings.shape}",
        )
    n_items, n_cats = ratings.shape
    if n_items == 0:
        return KappaReport(kappa=0.0, n_agreements=0, n_samples=0, n_classes=n_cats)
    row_sums = ratings.sum(axis=1)
    if not (row_sums == row_sums[0]).all():
        raise LabelingError(
            "fleiss_kappa: all items must have the same number of raters",
        )
    n_raters = int(row_sums[0])
    if n_raters < 2:
        raise LabelingError("fleiss_kappa: need at least 2 raters per item")

    # P_i: agreement rate per item (Eq. 3 of Fleiss 1971)
    p_i = ((ratings ** 2).sum(axis=1) - n_raters) / (n_raters * (n_raters - 1))
    p_bar = float(p_i.mean())
    # Category marginals
    p_j = ratings.sum(axis=0) / (n_items * n_raters)
    p_e = float((p_j ** 2).sum())
    kappa = 1.0 if p_e == 1.0 else (p_bar - p_e) / (1.0 - p_e)
    # Agreements: count items where all raters pick the same class.
    agreements = int(((ratings == n_raters).any(axis=1)).sum())
    return KappaReport(
        kappa=float(kappa), n_agreements=agreements,
        n_samples=n_items, n_classes=n_cats,
    )


__all__ = [
    "KappaReport",
    "cohen_kappa",
    "fleiss_kappa",
    "pairwise_kappas",
]

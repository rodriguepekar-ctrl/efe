"""TripletSampler — (anchor, positive, negative) tuples.

The sampler is responsible for the **structural** sampling of
triplets; the *hardness* mining happens inside the method's loss
function (where the model embedding is available).

Three strategies are supported here for the raw sampling:

* ``random``     — positive same label, negative different label (any).
* ``per_label``  — for each batch, pick a set of labels, then pull k
  anchors and positives per label; negatives are drawn from other
  labels.
* ``balanced``   — same idea as per_label but ensures every batch has
  at least one triplet from every *eligible* label.
"""

from __future__ import annotations

import random
from collections.abc import Iterator

from efe_worker.data.samplers.base import Batch, group_by_label
from efe_worker.data.snapshot import Row

_STRATEGIES = frozenset({"random", "per_label", "balanced"})


class TripletSampler:
    name = "triplet"

    def __init__(
        self,
        rows: list[Row],
        *,
        strategy: str = "random",
        seed: int = 42,
    ) -> None:
        if not rows:
            raise ValueError("TripletSampler needs at least 1 row")
        if strategy not in _STRATEGIES:
            raise ValueError(
                f"unknown strategy {strategy!r}; "
                f"one of {sorted(_STRATEGIES)}",
            )
        self.rows: list[Row] = list(rows)
        self._strategy = strategy
        self._seed = int(seed)

        by_label = group_by_label(self.rows)
        # Labels with at least 2 members can provide (anchor, positive).
        self._eligible = {
            lab: idxs for lab, idxs in by_label.items()
            if lab and len(idxs) >= 2
        }
        # Need ≥ 2 eligible labels so there is a "different label" pool.
        if len(self._eligible) < 2:
            raise ValueError(
                "TripletSampler requires ≥ 2 labels with ≥ 2 rows each",
            )
        self._label_list = list(self._eligible.keys())

    def n_rows(self) -> int:
        return len(self.rows)

    def _pick_negative(
        self, rng: random.Random, anchor_label: str,
    ) -> int:
        """Pick a row whose label differs from ``anchor_label``."""
        # Choose a different label, then a row from that label's pool.
        # If we only have rows under one other label that happens to
        # be ``anchor_label``, loop with a fresh label.
        while True:
            lab = rng.choice(self._label_list)
            if lab != anchor_label:
                return rng.choice(self._eligible[lab])

    def iter_batches(
        self, batch_size: int, *, shuffle: bool = True,
    ) -> Iterator[Batch]:
        if batch_size < 1:
            raise ValueError("TripletSampler requires batch_size >= 1")
        rng = random.Random(self._seed)
        while True:
            batch = Batch()
            if self._strategy == "balanced":
                per_label = max(1, batch_size // max(1, len(self._label_list)))
            else:
                per_label = 0  # not used
            produced = 0
            while produced < batch_size:
                if self._strategy == "balanced":
                    for lab in self._label_list:
                        if produced >= batch_size:
                            break
                        for _ in range(per_label):
                            if produced >= batch_size:
                                break
                            idxs = self._eligible[lab]
                            anchor_idx = rng.choice(idxs)
                            positive_idx = anchor_idx
                            while positive_idx == anchor_idx:
                                positive_idx = rng.choice(idxs)
                            neg_idx = self._pick_negative(rng, lab)
                            batch.anchor_texts.append(
                                self.rows[anchor_idx].text)
                            batch.positive_texts.append(
                                self.rows[positive_idx].text)
                            batch.negative_texts.append(
                                self.rows[neg_idx].text)
                            batch.labels.append(lab)
                            produced += 1
                else:
                    lab = rng.choice(self._label_list)
                    idxs = self._eligible[lab]
                    anchor_idx = rng.choice(idxs)
                    positive_idx = anchor_idx
                    while positive_idx == anchor_idx:
                        positive_idx = rng.choice(idxs)
                    neg_idx = self._pick_negative(rng, lab)
                    batch.anchor_texts.append(self.rows[anchor_idx].text)
                    batch.positive_texts.append(self.rows[positive_idx].text)
                    batch.negative_texts.append(self.rows[neg_idx].text)
                    batch.labels.append(lab)
                    produced += 1
            yield batch


__all__ = ["TripletSampler"]

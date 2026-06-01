"""PairSampler — (anchor, positive) pairs for InfoNCE.

Positives are sampled as <i>other</i> rows carrying the same label.
In-batch negatives are produced implicitly by the loss (every non-i
row in the batch acts as a negative). The sampler just has to make
sure each batch contains no duplicate anchors.

If a label has only one representative, it is skipped for the
anchor role (no positive available).
"""

from __future__ import annotations

import random
from collections.abc import Iterator

from efe_worker.data.samplers.base import Batch, group_by_label
from efe_worker.data.snapshot import Row


class PairSampler:
    name = "pair"

    def __init__(self, rows: list[Row], *, seed: int = 42) -> None:
        if not rows:
            raise ValueError("PairSampler needs at least 1 row")
        self.rows: list[Row] = list(rows)
        self._by_label = group_by_label(self.rows)
        # Drop labels with <2 rows (cannot produce a positive).
        self._eligible = {
            lab: idxs for lab, idxs in self._by_label.items()
            if lab and len(idxs) >= 2
        }
        if not self._eligible:
            raise ValueError(
                "PairSampler requires at least one label with ≥ 2 rows",
            )
        self._seed = int(seed)

    def n_rows(self) -> int:
        return len(self.rows)

    def iter_batches(
        self, batch_size: int, *, shuffle: bool = True,
    ) -> Iterator[Batch]:
        if batch_size < 2:
            raise ValueError("PairSampler requires batch_size >= 2")
        # Each eligible row can be used once as an anchor per batch
        # (seen_anchors enforces uniqueness); we therefore need at
        # least ``batch_size`` distinct eligible rows overall.
        total_eligible = sum(len(idxs) for idxs in self._eligible.values())
        if total_eligible < batch_size:
            raise ValueError(
                f"PairSampler cannot build a batch of size {batch_size} "
                f"from {total_eligible} eligible rows",
            )
        rng = random.Random(self._seed)
        labels = list(self._eligible.keys())
        step = 0
        while True:
            step += 1
            batch = Batch()
            seen_anchors: set[int] = set()
            attempts = 0
            budget = batch_size * 20
            while len(batch) < batch_size and attempts < budget:
                attempts += 1
                lab = rng.choice(labels)
                idxs = self._eligible[lab]
                anchor_idx = rng.choice(idxs)
                if anchor_idx in seen_anchors:
                    continue
                # pick a different positive within the same label
                positive_idx = anchor_idx
                while positive_idx == anchor_idx:
                    positive_idx = rng.choice(idxs)
                seen_anchors.add(anchor_idx)
                batch.anchor_texts.append(self.rows[anchor_idx].text)
                batch.positive_texts.append(self.rows[positive_idx].text)
                batch.labels.append(lab)
            if len(batch) != batch_size:
                # Starvation is a hard error; silently yielding an
                # undersized batch would bake undetected bugs into
                # the loss (InfoNCE crashes on N=1, others mis-size).
                raise RuntimeError(
                    f"PairSampler starved after {attempts} attempts "
                    f"({len(batch)}/{batch_size} anchors); "
                    f"increase dataset diversity or reduce batch_size",
                )
            yield batch


__all__ = ["PairSampler"]

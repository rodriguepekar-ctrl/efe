"""ClassBalancedSampler — forces representation from every class.

Used by the Classification method (C). For each batch:

    1. Pick ``k_way`` distinct classes (without replacement).
    2. Sample ``n_shot`` rows from each.
    3. Shuffle the resulting ``k_way * n_shot`` rows.

If ``k_way`` is not supplied we default to ``min(n_labels, batch_size)``
with ``n_shot = batch_size // k_way`` (clipped to ≥ 1). The leftover
slots are filled with random rows from random classes.
"""

from __future__ import annotations

import random
from collections.abc import Iterator

from efe_worker.data.samplers.base import (
    Batch,
    group_by_label,
    label_index_map,
)
from efe_worker.data.snapshot import Row


class ClassBalancedSampler:
    name = "class_balanced"

    def __init__(
        self,
        rows: list[Row],
        *,
        k_way: int | None = None,
        n_shot: int | None = None,
        seed: int = 42,
    ) -> None:
        if not rows:
            raise ValueError("ClassBalancedSampler needs at least 1 row")
        # Drop rows without a label.
        labelled = [r for r in rows if r.label is not None]
        if not labelled:
            raise ValueError(
                "ClassBalancedSampler requires at least one labelled row",
            )
        self.rows: list[Row] = labelled
        self._by_label = group_by_label(self.rows)
        # Drop the "" key if any slipped through.
        self._by_label.pop("", None)
        self._label_list = sorted(self._by_label.keys())
        if not self._label_list:
            raise ValueError(
                "ClassBalancedSampler: no non-empty labels",
            )
        self._label_map = label_index_map(self.rows)
        self._k_way = k_way
        self._n_shot = n_shot
        self._seed = int(seed)

    def n_rows(self) -> int:
        return len(self.rows)

    @property
    def n_classes(self) -> int:
        return len(self._label_map)

    @property
    def label_map(self) -> dict[str, int]:
        return dict(self._label_map)

    def iter_batches(
        self, batch_size: int, *, shuffle: bool = True,
    ) -> Iterator[Batch]:
        if batch_size < 1:
            raise ValueError("batch_size must be >= 1")
        rng = random.Random(self._seed)
        k_way = self._k_way or min(len(self._label_list), batch_size)
        n_shot = self._n_shot or max(1, batch_size // k_way)
        while True:
            batch = Batch()
            # 1. pick k_way classes
            if k_way >= len(self._label_list):
                classes = list(self._label_list)
            else:
                classes = rng.sample(self._label_list, k_way)
            selected: list[int] = []
            for lab in classes:
                pool = self._by_label[lab]
                k = min(n_shot, len(pool))
                selected.extend(rng.sample(pool, k))
            # 2. fill remainder
            while len(selected) < batch_size:
                lab = rng.choice(self._label_list)
                selected.append(rng.choice(self._by_label[lab]))
            # 3. trim
            selected = selected[:batch_size]
            # 4. shuffle
            if shuffle:
                rng.shuffle(selected)
            for idx in selected:
                r = self.rows[idx]
                batch.anchor_texts.append(r.text)
                batch.labels.append(r.label or "")
                batch.label_ids.append(self._label_map[r.label or ""])
            yield batch


__all__ = ["ClassBalancedSampler"]

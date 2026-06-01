"""RawSampler — random shuffle with optional label IDs.

Used by LoRA (D) and Distillation (E) where the method controls
everything (batch composition doesn't need special structure).
"""

from __future__ import annotations

import random
from collections.abc import Iterator

from efe_worker.data.samplers.base import Batch, label_index_map
from efe_worker.data.snapshot import Row


class RawSampler:
    name = "raw"

    def __init__(self, rows: list[Row], *, seed: int = 42) -> None:
        if not rows:
            raise ValueError("RawSampler needs at least 1 row")
        self.rows: list[Row] = list(rows)
        self._label_map = label_index_map(self.rows)
        self._seed = int(seed)

    def n_rows(self) -> int:
        return len(self.rows)

    @property
    def label_map(self) -> dict[str, int]:
        return dict(self._label_map)

    def iter_batches(
        self, batch_size: int, *, shuffle: bool = True,
    ) -> Iterator[Batch]:
        if batch_size < 1:
            raise ValueError("batch_size must be >= 1")
        rng = random.Random(self._seed)
        order = list(range(len(self.rows)))
        while True:
            if shuffle:
                rng.shuffle(order)
            for start in range(0, len(order), batch_size):
                chunk = order[start : start + batch_size]
                if len(chunk) < batch_size:
                    continue
                batch = Batch()
                for idx in chunk:
                    r = self.rows[idx]
                    batch.anchor_texts.append(r.text)
                    if r.label is not None:
                        batch.labels.append(r.label)
                        batch.label_ids.append(self._label_map[r.label])
                yield batch


__all__ = ["RawSampler"]

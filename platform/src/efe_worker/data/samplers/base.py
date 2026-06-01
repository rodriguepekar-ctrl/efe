"""Base sampler — protocol + :class:`Batch` dataclass."""

from __future__ import annotations

from collections.abc import Iterator
from dataclasses import dataclass, field
from typing import Any, Protocol

from efe_worker.data.snapshot import Row


@dataclass
class Batch:
    """A method-agnostic batch structure.

    Every sampler yields :class:`Batch` instances; each method reads
    the fields it cares about:

    * ``anchor_texts``   — always populated (shape: (N,))
    * ``positive_texts`` — populated by PairSampler, TripletSampler
    * ``negative_texts`` — populated by TripletSampler
    * ``labels``         — populated by ClassBalancedSampler, RawSampler
                           (if rows have labels)
    * ``meta``           — free-form dict for custom payloads
    """

    anchor_texts: list[str] = field(default_factory=list)
    positive_texts: list[str] = field(default_factory=list)
    negative_texts: list[str] = field(default_factory=list)
    labels: list[str] = field(default_factory=list)
    label_ids: list[int] = field(default_factory=list)
    meta: dict[str, Any] = field(default_factory=dict)

    def __len__(self) -> int:
        return len(self.anchor_texts)

    def as_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "anchor": list(self.anchor_texts),
        }
        if self.positive_texts:
            out["positive"] = list(self.positive_texts)
        if self.negative_texts:
            out["negative"] = list(self.negative_texts)
        if self.label_ids:
            out["label"] = list(self.label_ids)
        return out


class BaseSampler(Protocol):
    """Every sampler conforms to this narrow surface.

    Implementations carry the underlying :class:`Row` list and any
    indexing structures they need. ``iter_batches`` yields an
    unbounded stream; the driver stops after ``max_steps`` regardless.
    """

    name: str
    rows: list[Row]

    def iter_batches(
        self, batch_size: int, *, shuffle: bool = True,
    ) -> Iterator[Batch]:  # pragma: no cover — protocol
        ...

    def n_rows(self) -> int:  # pragma: no cover — protocol
        ...


# ---------------------------------------------------------------------------
# shared helpers
# ---------------------------------------------------------------------------


def group_by_label(rows: list[Row]) -> dict[str, list[int]]:
    """Return ``{label → [row indices]}``.

    Rows without a label are grouped under ``""`` which callers may
    filter out.
    """
    out: dict[str, list[int]] = {}
    for idx, r in enumerate(rows):
        out.setdefault(r.label or "", []).append(idx)
    return out


def label_index_map(rows: list[Row]) -> dict[str, int]:
    """Stable ``{label → int id}`` sorted alphabetically."""
    labels = sorted({r.label for r in rows if r.label is not None})
    return {lab: i for i, lab in enumerate(labels)}


__all__ = ["BaseSampler", "Batch", "group_by_label", "label_index_map"]

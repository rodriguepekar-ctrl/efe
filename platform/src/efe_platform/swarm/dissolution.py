"""Client-side decoding of the ``DecodedOutput`` wire message.

The Rust server already performs the heavy lifting (centroid
extraction, optional text decoding). This module takes the
near-final wire payload and exposes two clean affordances:

* :func:`decode_output` — turn a raw :class:`DecodedPayload` into a
  :class:`DecodedOutput` with a ``kind`` discriminant and stable
  accessors (``as_text()`` / ``as_centroid()``).

* :func:`centroid_statistics` — summary stats (norm, mean, stddev) for
  audit / monitoring when the caller opts into ``pass_through`` mode.

No network I/O happens here.
"""

from __future__ import annotations

import math
import statistics
from dataclasses import dataclass
from typing import Literal

from efe_platform.swarm.protocol import DecodedPayload

DecodedKind = Literal["text", "centroid"]


@dataclass(frozen=True, slots=True)
class DecodedOutput:
    """Decoded swarm output exposed to application callers."""

    kind: DecodedKind
    text: str | None = None
    centroid: tuple[float, ...] | None = None

    def as_text(self) -> str:
        if self.kind != "text" or self.text is None:
            raise ValueError("decoded output is not text")
        return self.text

    def as_centroid(self) -> tuple[float, ...]:
        if self.kind != "centroid" or self.centroid is None:
            raise ValueError("decoded output is not a centroid")
        return self.centroid


def decode_output(payload: DecodedPayload) -> DecodedOutput:
    """Project a :class:`DecodedPayload` into a :class:`DecodedOutput`.

    The two types look similar — but :class:`DecodedOutput` is what
    application code consumes: it has a discriminant and accessor
    methods, whereas :class:`DecodedPayload` is a faithful wire mirror.
    """
    if payload.is_text():
        assert payload.text is not None  # invariant of DecodedPayload
        return DecodedOutput(kind="text", text=payload.text)
    assert payload.centroid is not None
    return DecodedOutput(kind="centroid", centroid=payload.centroid)


# ---------------------------------------------------------------------------
#  Centroid statistics
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class CentroidStats:
    """Cheap summary statistics of a centroid vector."""

    dim: int
    l2_norm: float
    mean: float
    stddev: float
    min: float
    max: float

    def to_dict(self) -> dict[str, float | int]:
        return {
            "dim": self.dim,
            "l2_norm": round(self.l2_norm, 6),
            "mean": round(self.mean, 6),
            "stddev": round(self.stddev, 6),
            "min": round(self.min, 6),
            "max": round(self.max, 6),
        }


def centroid_statistics(values: tuple[float, ...]) -> CentroidStats:
    """Compute summary stats for an audit entry.

    Raises :class:`ValueError` on empty / non-finite values.
    """
    if not values:
        raise ValueError("empty centroid")
    if not all(math.isfinite(v) for v in values):
        raise ValueError("centroid contains non-finite values")
    l2 = math.sqrt(sum(v * v for v in values))
    m = statistics.fmean(values)
    s = statistics.pstdev(values) if len(values) > 1 else 0.0
    return CentroidStats(
        dim=len(values),
        l2_norm=l2,
        mean=m,
        stddev=s,
        min=min(values),
        max=max(values),
    )


__all__ = [
    "CentroidStats",
    "DecodedKind",
    "DecodedOutput",
    "centroid_statistics",
    "decode_output",
]

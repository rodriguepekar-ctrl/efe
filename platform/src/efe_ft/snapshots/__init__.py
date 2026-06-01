"""Immutable snapshot materialisation.

A snapshot is a frozen view over ``training_samples``: given filters
(status, label constraints, partitioning) the materialiser computes
the full list of ``sample_ids``, hashes it canonically, and writes a
new :class:`Snapshot` row plus its members.

Two snapshots with identical ``sample_ids`` (order-independent) have
the same ``content_hash`` — this is the basis of training
reproducibility.
"""

from __future__ import annotations

from efe_ft.snapshots.materialiser import (
    MaterialisationSpec,
    Partitioning,
    materialise,
)

__all__ = ["MaterialisationSpec", "Partitioning", "materialise"]

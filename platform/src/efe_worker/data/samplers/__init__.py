"""Samplers — row-level strategies for batching rows per method.

Each method has a preferred sampler:

    InfoNCE   (A) → PairSampler
    Triplet   (B) → TripletSampler
    Classif   (C) → ClassBalancedSampler
    LoRA      (D) → RawSampler
    Distill   (E) → RawSampler

All samplers implement :class:`BaseSampler` so the driver is
method-agnostic: it just calls ``.iter_batches(batch_size)``.
"""

from __future__ import annotations

from efe_worker.data.samplers.base import BaseSampler, Batch
from efe_worker.data.samplers.class_balanced import ClassBalancedSampler
from efe_worker.data.samplers.pair import PairSampler
from efe_worker.data.samplers.raw import RawSampler
from efe_worker.data.samplers.triplet import TripletSampler

__all__ = [
    "BaseSampler",
    "Batch",
    "ClassBalancedSampler",
    "PairSampler",
    "RawSampler",
    "TripletSampler",
]

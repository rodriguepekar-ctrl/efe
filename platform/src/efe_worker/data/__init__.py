"""Data layer — snapshot consumption, tokenisation, samplers.

Public surface:

    SnapshotManifest, ShardRef, Snapshot, fetch_snapshot
    Tokenizer, TokenizedBatch, FakeTokenizer, HuggingFaceTokenizer
    PairSampler, TripletSampler, ClassBalancedSampler, RawSampler
"""

from __future__ import annotations

from efe_worker.data.samplers import (
    BaseSampler,
    ClassBalancedSampler,
    PairSampler,
    RawSampler,
    TripletSampler,
)
from efe_worker.data.snapshot import (
    ShardRef,
    Snapshot,
    SnapshotManifest,
    fetch_snapshot,
)
from efe_worker.data.tokenizer import (
    FakeTokenizer,
    HuggingFaceTokenizer,
    TokenizedBatch,
    Tokenizer,
)

__all__ = [
    "BaseSampler",
    "ClassBalancedSampler",
    "FakeTokenizer",
    "HuggingFaceTokenizer",
    "PairSampler",
    "RawSampler",
    "ShardRef",
    "Snapshot",
    "SnapshotManifest",
    "TokenizedBatch",
    "Tokenizer",
    "TripletSampler",
    "fetch_snapshot",
]

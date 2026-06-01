"""Tests for every sampler."""

from __future__ import annotations

from itertools import islice

import pytest

from efe_worker.data.samplers import (
    ClassBalancedSampler,
    PairSampler,
    RawSampler,
    TripletSampler,
)
from efe_worker.data.samplers.base import (
    Batch,
    group_by_label,
    label_index_map,
)
from efe_worker.data.snapshot import Row


def _rows() -> list[Row]:
    labs = ["a", "a", "a", "b", "b", "b", "c", "c", "c", "c"]
    return [Row(text=f"t{i}", label=lab) for i, lab in enumerate(labs)]


# ---------- helpers ---------- #


def test_group_by_label() -> None:
    rows = _rows()
    g = group_by_label(rows)
    assert set(g.keys()) == {"a", "b", "c"}
    assert len(g["a"]) == 3


def test_label_index_map() -> None:
    m = label_index_map(_rows())
    assert m == {"a": 0, "b": 1, "c": 2}


def test_batch_as_dict() -> None:
    b = Batch(anchor_texts=["x"], label_ids=[0])
    d = b.as_dict()
    assert d["anchor"] == ["x"]
    assert d["label"] == [0]


# ---------- Pair ---------- #


def test_pair_sampler_basic() -> None:
    s = PairSampler(_rows())
    it = s.iter_batches(4)
    batch = next(it)
    assert len(batch) == 4
    assert len(batch.positive_texts) == 4


def test_pair_sampler_rejects_tiny_batch() -> None:
    s = PairSampler(_rows())
    with pytest.raises(ValueError):
        next(s.iter_batches(1))


def test_pair_sampler_rejects_empty() -> None:
    with pytest.raises(ValueError):
        PairSampler([])


def test_pair_sampler_needs_labelled_pairs() -> None:
    with pytest.raises(ValueError):
        PairSampler([Row(text="a", label=None)])


# ---------- Triplet ---------- #


def test_triplet_sampler_basic() -> None:
    s = TripletSampler(_rows())
    b = next(s.iter_batches(6))
    assert len(b) == 6
    for a, p, n, lab in zip(
        b.anchor_texts, b.positive_texts, b.negative_texts, b.labels,
        strict=False,
    ):
        assert a != n


def test_triplet_sampler_balanced() -> None:
    s = TripletSampler(_rows(), strategy="balanced")
    b = next(s.iter_batches(6))
    assert len(b) == 6
    # Should cover multiple labels
    assert len(set(b.labels)) >= 2


def test_triplet_sampler_unknown_strategy() -> None:
    with pytest.raises(ValueError):
        TripletSampler(_rows(), strategy="galaxy_brain")


def test_triplet_sampler_needs_two_labels() -> None:
    rows = [Row(text=f"t{i}", label="a") for i in range(5)]
    with pytest.raises(ValueError):
        TripletSampler(rows)


# ---------- ClassBalanced ---------- #


def test_class_balanced_basic() -> None:
    s = ClassBalancedSampler(_rows())
    b = next(s.iter_batches(6))
    assert len(b) == 6
    assert len(b.label_ids) == 6
    assert set(b.labels) <= {"a", "b", "c"}


def test_class_balanced_rejects_no_labels() -> None:
    rows = [Row(text="x", label=None)]
    with pytest.raises(ValueError):
        ClassBalancedSampler(rows)


def test_class_balanced_fills_and_shuffles() -> None:
    s = ClassBalancedSampler(_rows(), k_way=2, n_shot=1)
    b = next(s.iter_batches(4))
    assert len(b) == 4


# ---------- Raw ---------- #


def test_raw_sampler_yields_batches_of_size() -> None:
    s = RawSampler(_rows())
    batches = list(islice(s.iter_batches(2), 5))
    for b in batches:
        assert len(b) == 2


def test_raw_sampler_label_map_accessible() -> None:
    s = RawSampler(_rows())
    assert set(s.label_map.keys()) == {"a", "b", "c"}

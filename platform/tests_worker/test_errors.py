"""Tests for :mod:`efe_worker.errors`."""

from __future__ import annotations

from efe_worker.errors import (
    CudaOOM,
    SnapshotMissing,
    TrainingDiverged,
    WorkerError,
)


def test_root_has_code_and_retryable_hint() -> None:
    e = WorkerError("oops", detail={"x": 1})
    assert e.code == "worker_error"
    assert e.retryable_hint is False
    assert e.to_dict()["code"] == "worker_error"
    assert e.to_dict()["detail"] == {"x": 1}


def test_cuda_oom_is_retryable() -> None:
    e = CudaOOM("oom")
    assert e.retryable_hint is True
    assert e.code == "cuda_oom"


def test_snapshot_missing_is_data_error() -> None:
    e = SnapshotMissing("nope")
    assert e.code == "snapshot_missing"
    assert e.retryable_hint is False


def test_training_diverged() -> None:
    e = TrainingDiverged("NaN")
    assert e.code == "training_diverged"


def test_repr_contains_code() -> None:
    r = repr(WorkerError("foo"))
    assert "worker_error" in r
    assert "foo" in r


def test_detail_omitted_when_none() -> None:
    e = WorkerError("x")
    assert "detail" not in e.to_dict()

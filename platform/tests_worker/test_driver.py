"""Tests for the training driver.

We exercise the 9-stage pipeline with a pure-Python fake
:class:`TrainingMethod` + :class:`FakeOptimizer` — no torch needed.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest

from efe_worker.config import WorkerSettings
from efe_worker.context import RunContext
from efe_worker.data.samplers.base import BaseSampler, Batch
from efe_worker.data.samplers.raw import RawSampler
from efe_worker.data.snapshot import (
    Row,
    Snapshot,
    build_synthetic_snapshot,
)
from efe_worker.data.tokenizer import FakeTokenizer
from efe_worker.errors import SnapshotMissing, TrainingDiverged
from efe_worker.methods.base import LossOutput, Model
from efe_worker.training.checkpoint import CheckpointManager
from efe_worker.training.driver import (
    FakeOptimizer,
    run_training,
)
from efe_worker.transport.grpc_client import FakeOrchestratorClient
from efe_worker.transport.s3 import FakeBlobStore


class _FakeModel:
    name = "fake"

    def __init__(self) -> None:
        self.training = True
        self.device = None
        self.saved_state = False

    def to(self, device: Any) -> _FakeModel:
        self.device = device
        return self

    def train(self, mode: bool = True) -> _FakeModel:
        self.training = mode
        return self

    def eval(self) -> _FakeModel:
        self.training = False
        return self

    def parameters(self) -> Any:
        return [object()]

    def state_dict(self) -> dict[str, Any]:
        self.saved_state = True
        return {"weights": [1, 2, 3]}

    def load_state_dict(
        self, state: dict[str, Any], strict: bool = True,
    ) -> Any:
        return None


class _FakeMethod:
    name = "fake"

    def __init__(self, hyperparameters: dict[str, Any] | None = None) -> None:
        self.hp = dict(hyperparameters or {})
        self.loss_calls = 0
        self.save_calls = 0

    def build_model(
        self, base_model: str, hyperparameters: dict[str, Any],
    ) -> Model:
        return _FakeModel()

    def build_sampler(
        self, snapshot: Snapshot, tokenizer: FakeTokenizer,
    ) -> BaseSampler:
        return RawSampler(
            snapshot.rows, seed=snapshot.manifest.partitioning.seed,
        )

    def compute_loss(
        self, model: Model, batch: Batch, *, step: int, device: Any,
        tokenizer: FakeTokenizer,
    ) -> LossOutput:
        self.loss_calls += 1
        return LossOutput(
            loss=0.5,
            scalar=0.5,
            metrics={"accuracy": 0.75},
        )

    def eval_metrics(
        self, model: Model, val_rows: list[Row],
        tokenizer: FakeTokenizer, *, device: Any,
    ) -> dict[str, float]:
        return {"loss": 0.3, "accuracy": 0.8}

    def save_artifact(
        self, model: Model, out_dir: Path, *,
        extra: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        self.save_calls += 1
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "model.bin").write_bytes(b"fake")
        meta = {"method": "fake"}
        if extra:
            meta.update(extra)
        return meta


class _DivergingMethod(_FakeMethod):
    def compute_loss(
        self, model: Model, batch: Batch, *, step: int, device: Any,
        tokenizer: FakeTokenizer,
    ) -> LossOutput:
        raise TrainingDiverged("NaN")


# -------------------------------------------------------------------- util


def _build_settings(tmp_path: Path) -> WorkerSettings:
    return WorkerSettings.from_env(overrides={
        "environment": "test",
        "method_name": "classif",
        "identity": {
            "run_id": "r", "tenant_id": "t", "task_id": "r",
            "snapshot_id": "s", "worker_id": "w",
            "snapshot_uri": "file:///snap/s/manifest.json",
        },
        "transport": {"fake_transport": True},
        "training": {
            "max_steps": 4, "batch_size": 4,
            "eval_every_n_steps": 2,
        },
        "checkpoint": {
            "local_dir": str(tmp_path / "ckpts"),
            "remote_uri_prefix": "file:///remote",
            "every_n_steps": 2,
            "keep_local": 2,
            "best_metric": "val/loss",
        },
    })


async def _run(
    tmp_path: Path, method: _FakeMethod,
    *, optimizer: FakeOptimizer | None = None,
) -> tuple[Any, FakeOrchestratorClient, FakeBlobStore]:
    cfg = _build_settings(tmp_path)
    blobs, _uri = build_synthetic_snapshot(snapshot_id="s")
    store = FakeBlobStore(data=blobs)
    transport = FakeOrchestratorClient()
    ctx = RunContext(
        settings=cfg, transport=transport, blob_store=store,
    )
    ckpt = CheckpointManager(
        run_id=ctx.run_id,
        local_root=Path(cfg.checkpoint.local_dir),
        remote_prefix=cfg.checkpoint.remote_uri_prefix,
        blob_store=store,
        keep_local=cfg.checkpoint.keep_local,
        best_metric=cfg.checkpoint.best_metric,
        best_mode=cfg.checkpoint.best_metric_mode,
    )
    outcome = await run_training(
        ctx, method, tokenizer=FakeTokenizer(),
        checkpoint_manager=ckpt,
        optimizer=optimizer or FakeOptimizer(),
    )
    return outcome, transport, store


# -------------------------------------------------------------------- tests


async def test_driver_happy_path(tmp_path: Path) -> None:
    method = _FakeMethod()
    outcome, transport, _store = await _run(tmp_path, method)
    assert outcome.n_steps >= 1
    assert outcome.artifact_uri is not None
    # Should have reported result
    assert len(transport.results) == 1
    assert transport.results[0].artifact_url == outcome.artifact_uri


async def test_driver_checkpoints(tmp_path: Path) -> None:
    method = _FakeMethod()
    _outcome, transport, _store = await _run(tmp_path, method)
    # With max_steps=4 and every_n_steps=2 → 2 checkpoints.
    assert len(transport.checkpoints) == 2


async def test_driver_eval_happens_periodically(tmp_path: Path) -> None:
    method = _FakeMethod()
    outcome, _transport, _store = await _run(tmp_path, method)
    assert "val/loss" in outcome.final_metrics
    assert outcome.best_metric_value == 0.3


async def test_driver_diverging_loss_propagates(tmp_path: Path) -> None:
    method = _DivergingMethod()
    with pytest.raises(TrainingDiverged):
        await _run(tmp_path, method)


async def test_driver_missing_snapshot(tmp_path: Path) -> None:
    cfg = _build_settings(tmp_path)
    ctx = RunContext(
        settings=cfg,
        transport=FakeOrchestratorClient(),
        blob_store=FakeBlobStore(),  # empty
    )
    ckpt = CheckpointManager(
        run_id=ctx.run_id,
        local_root=Path(cfg.checkpoint.local_dir),
        remote_prefix=cfg.checkpoint.remote_uri_prefix,
        blob_store=FakeBlobStore(),
    )
    with pytest.raises(SnapshotMissing):
        await run_training(
            ctx, _FakeMethod(), tokenizer=FakeTokenizer(),
            checkpoint_manager=ckpt, optimizer=FakeOptimizer(),
        )


async def test_driver_empty_snapshot_uri(tmp_path: Path) -> None:
    cfg = WorkerSettings.from_env(overrides={
        "identity": {
            "run_id": "r", "tenant_id": "t", "task_id": "r",
            "snapshot_id": "s", "worker_id": "w",
            "snapshot_uri": "",
        },
        "transport": {"fake_transport": True},
    })
    ctx = RunContext(
        settings=cfg, transport=FakeOrchestratorClient(),
        blob_store=FakeBlobStore(),
    )
    method = _FakeMethod()
    with pytest.raises(SnapshotMissing):
        await run_training(
            ctx, method, tokenizer=FakeTokenizer(),
            optimizer=FakeOptimizer(),
        )


async def test_driver_raises_on_user_deadline(tmp_path: Path) -> None:
    """The driver raises :class:`UserDeadlineExceeded` so __main__ can
    report a distinct error code to M03."""
    from efe_worker.clock import FakeClock
    from efe_worker.errors import UserDeadlineExceeded

    cfg = _build_settings(tmp_path)
    cfg = cfg.model_copy(update={
        "training": cfg.training.model_copy(
            update={"user_deadline_seconds": 1, "max_steps": 100},
        ),
    })
    clock = FakeClock()
    blobs, _uri = build_synthetic_snapshot(snapshot_id="s")
    ctx = RunContext(
        settings=cfg, clock=clock,
        transport=FakeOrchestratorClient(),
        blob_store=FakeBlobStore(data=blobs),
    )
    ckpt = CheckpointManager(
        run_id=ctx.run_id,
        local_root=Path(cfg.checkpoint.local_dir),
        remote_prefix=None, blob_store=None,
    )

    class _SlowMethod(_FakeMethod):
        def compute_loss(self, *a: Any, **k: Any) -> LossOutput:
            clock.advance(2.0)
            return super().compute_loss(*a, **k)

    with pytest.raises(UserDeadlineExceeded):
        await run_training(
            ctx, _SlowMethod(), tokenizer=FakeTokenizer(),
            checkpoint_manager=ckpt, optimizer=FakeOptimizer(),
        )


async def test_driver_respects_stop_event(tmp_path: Path) -> None:
    cfg = _build_settings(tmp_path)
    cfg = cfg.model_copy(update={
        "training": cfg.training.model_copy(update={"max_steps": 100}),
    })
    blobs, _uri = build_synthetic_snapshot(snapshot_id="s")
    ctx = RunContext(
        settings=cfg, transport=FakeOrchestratorClient(),
        blob_store=FakeBlobStore(data=blobs),
    )
    ctx.request_stop()  # immediate stop
    ckpt = CheckpointManager(
        run_id=ctx.run_id,
        local_root=Path(cfg.checkpoint.local_dir),
        remote_prefix=None, blob_store=None,
    )
    outcome = await run_training(
        ctx, _FakeMethod(), tokenizer=FakeTokenizer(),
        checkpoint_manager=ckpt, optimizer=FakeOptimizer(),
    )
    # Driver still completes (eval + upload + report), but did no step.
    assert outcome.n_steps == 0

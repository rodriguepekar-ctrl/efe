"""Tests for :mod:`efe_worker.training.checkpoint`."""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest

from efe_worker.training.checkpoint import (
    CheckpointManager,
    download_ckpt_dir,
)
from efe_worker.transport.s3 import FakeBlobStore


class _Method:
    name = "fake"

    def build_model(self, *a: Any, **k: Any) -> Any:
        raise NotImplementedError

    def build_sampler(self, *a: Any, **k: Any) -> Any:
        raise NotImplementedError

    def compute_loss(self, *a: Any, **k: Any) -> Any:
        raise NotImplementedError

    def eval_metrics(self, *a: Any, **k: Any) -> dict[str, float]:
        return {}

    def save_artifact(
        self, model: Any, out_dir: Path, *,
        extra: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "model.bin").write_bytes(b"x" * 16)
        return {"method": "fake", "extra": extra or {}}


class _Model:
    def to(self, d: Any) -> _Model: return self
    def train(self, m: bool = True) -> _Model: return self
    def eval(self) -> _Model: return self
    def parameters(self) -> Any: return []
    def state_dict(self) -> dict[str, Any]: return {}
    def load_state_dict(self, state: dict[str, Any], strict: bool = True) -> Any:
        return None


async def test_save_creates_local_dir(tmp_path: Path) -> None:
    m = CheckpointManager(
        run_id="r", local_root=tmp_path, remote_prefix=None,
        blob_store=None,
    )
    info = await m.save(
        _Method(), _Model(), step=10, metrics={"val/loss": 0.5},
    )
    assert info.step == 10
    assert info.local_dir.is_dir()
    assert (info.local_dir / "model.bin").is_file()
    assert (info.local_dir / "metrics.json").is_file()


async def test_best_tracking(tmp_path: Path) -> None:
    m = CheckpointManager(
        run_id="r", local_root=tmp_path, remote_prefix=None,
        blob_store=None, best_metric="val/loss", best_mode="min",
    )
    i1 = await m.save(_Method(), _Model(), step=1, metrics={"val/loss": 0.5})
    assert i1.is_best
    i2 = await m.save(_Method(), _Model(), step=2, metrics={"val/loss": 0.6})
    assert not i2.is_best
    i3 = await m.save(_Method(), _Model(), step=3, metrics={"val/loss": 0.4})
    assert i3.is_best


async def test_upload_to_blob_store(tmp_path: Path) -> None:
    s = FakeBlobStore()
    m = CheckpointManager(
        run_id="r",
        local_root=tmp_path,
        remote_prefix="file:///remote",
        blob_store=s,
    )
    info = await m.save(_Method(), _Model(), step=1, metrics={"val/loss": 0.5})
    await m.drain()  # wait for async upload
    assert info.remote_uri is not None
    # All files from local dir should have landed under remote.
    prefix = info.remote_uri
    uris = await s.list_prefix(prefix)
    assert any("model.bin" in u for u in uris)


async def test_prune_respects_keep_local(tmp_path: Path) -> None:
    m = CheckpointManager(
        run_id="r", local_root=tmp_path, remote_prefix=None,
        blob_store=None, keep_local=2,
    )
    for step in (1, 2, 3, 4):
        await m.save(_Method(), _Model(), step=step,
                     metrics={"val/loss": 1.0})
    # Only 2 directories should remain on disk (plus possibly best).
    local_dirs = [
        p for p in tmp_path.iterdir() if p.is_dir() and p.name.startswith("ckpt")
    ]
    assert len(local_dirs) <= 3  # keep_local + best


async def test_upload_final(tmp_path: Path) -> None:
    s = FakeBlobStore()
    m = CheckpointManager(
        run_id="r",
        local_root=tmp_path,
        remote_prefix="file:///remote",
        blob_store=s,
    )
    uri = await m.upload_final(
        _Method(), _Model(), metrics={"val/loss": 0.3},
    )
    assert uri == "file:///remote/r/final"
    uris = await s.list_prefix(uri)
    assert any("model.bin" in u for u in uris)


async def test_upload_final_no_remote(tmp_path: Path) -> None:
    m = CheckpointManager(
        run_id="r", local_root=tmp_path, remote_prefix=None,
        blob_store=None,
    )
    uri = await m.upload_final(_Method(), _Model())
    assert uri.startswith("file://")


async def test_download_ckpt_dir(tmp_path: Path) -> None:
    s = FakeBlobStore()
    await s.put("file:///r/final/a.bin", b"1")
    await s.put("file:///r/final/b.bin", b"2")
    dest = tmp_path / "pulled"
    await download_ckpt_dir(s, "file:///r/final", dest)
    assert (dest / "a.bin").read_bytes() == b"1"
    assert (dest / "b.bin").read_bytes() == b"2"


async def test_checkpoint_rejects_bad_keep_local(tmp_path: Path) -> None:
    with pytest.raises(ValueError):
        CheckpointManager(
            run_id="r", local_root=tmp_path, remote_prefix=None,
            blob_store=None, keep_local=0,
        )


async def test_checkpoint_rejects_bad_mode(tmp_path: Path) -> None:
    with pytest.raises(ValueError):
        CheckpointManager(
            run_id="r", local_root=tmp_path, remote_prefix=None,
            blob_store=None, best_mode="galaxy",
        )

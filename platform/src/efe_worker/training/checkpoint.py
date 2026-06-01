"""Checkpoint manager — local saves + async S3 sync + pruning.

Layout on disk (under :attr:`WorkerSettings.checkpoint.local_dir`):

    ckpt-step-<n>/
        model.pt            (method-specific; see method.save_artifact)
        meta.json           {step, loss, best_metric_value, ...}
        metrics.json        {key → value}

Each checkpoint is mirrored to
``<remote_uri_prefix>/<run_id>/step-<n>/`` after the local save, in
an asyncio background task so the training loop is never blocked.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from efe_worker.errors import StorageError, WorkerError
from efe_worker.methods.base import TrainingMethod
from efe_worker.observability.logging import get_logger
from efe_worker.transport.s3 import BlobStore

_log = get_logger(__name__)


@dataclass(frozen=True, slots=True)
class CheckpointInfo:
    """Metadata returned by :meth:`CheckpointManager.save`."""

    step: int
    local_dir: Path
    remote_uri: str | None
    metrics: dict[str, float]
    is_best: bool
    size_bytes: int = 0


@dataclass
class CheckpointManager:
    """Persist + upload + prune checkpoints for one run."""

    run_id: str
    local_root: Path
    remote_prefix: str | None
    blob_store: BlobStore | None
    keep_local: int = 3
    best_metric: str = "val/loss"
    best_mode: str = "min"

    _best_value: float | None = field(default=None, init=False, repr=False)
    _best_step: int = field(default=-1, init=False, repr=False)
    _history: list[Path] = field(default_factory=list, init=False, repr=False)
    _pending: list[asyncio.Task[Any]] = field(
        default_factory=list, init=False, repr=False,
    )

    def __post_init__(self) -> None:
        self.local_root = Path(self.local_root).expanduser()
        self.local_root.mkdir(parents=True, exist_ok=True)
        if self.keep_local < 1:
            raise ValueError("keep_local must be >= 1")
        if self.best_mode not in ("min", "max"):
            raise ValueError(f"unknown best_mode {self.best_mode!r}")

    # ---- save + upload ------------------------------------------------

    def _local_dir(self, step: int) -> Path:
        return self.local_root / f"ckpt-step-{step:06d}"

    def _remote_uri(self, step: int) -> str | None:
        if self.remote_prefix is None:
            return None
        return f"{self.remote_prefix.rstrip('/')}/{self.run_id}/step-{step:06d}"

    def _dir_size(self, path: Path) -> int:
        total = 0
        for p in path.rglob("*"):
            if p.is_file():
                try:
                    total += p.stat().st_size
                except OSError:
                    pass
        return total

    def _is_best(self, metrics: dict[str, float]) -> bool:
        if self.best_metric not in metrics:
            return False
        v = float(metrics[self.best_metric])
        if self._best_value is None:
            self._best_value = v
            return True
        improved = (
            v < self._best_value if self.best_mode == "min"
            else v > self._best_value
        )
        if improved:
            self._best_value = v
            return True
        return False

    async def save(
        self,
        method: TrainingMethod,
        model: Any,
        *,
        step: int,
        metrics: dict[str, float] | None = None,
    ) -> CheckpointInfo:
        """Write a checkpoint locally + kick off async upload + prune."""
        local = self._local_dir(step)
        local.mkdir(parents=True, exist_ok=True)
        saved = method.save_artifact(
            model, local, extra={"step": int(step)},
        )
        metric_dict = dict(metrics or {})
        (local / "metrics.json").write_text(
            json.dumps(metric_dict, indent=2), encoding="utf-8",
        )
        (local / "step.json").write_text(
            json.dumps({"step": int(step), "saved_meta": saved}, indent=2),
            encoding="utf-8",
        )
        is_best = self._is_best(metric_dict)
        if is_best:
            self._best_step = int(step)
        size = self._dir_size(local)
        remote = self._remote_uri(step)
        if remote and self.blob_store is not None:
            self._pending.append(
                asyncio.create_task(
                    self._upload_dir(local, remote),
                    name=f"upload-ckpt-{step}",
                ),
            )
        # track + prune
        self._history.append(local)
        self._prune()
        return CheckpointInfo(
            step=int(step),
            local_dir=local,
            remote_uri=remote,
            metrics=metric_dict,
            is_best=is_best,
            size_bytes=size,
        )

    async def upload_final(
        self,
        method: TrainingMethod,
        model: Any,
        *,
        metrics: dict[str, float] | None = None,
    ) -> str:
        """Materialise the final artifact and push to the registry.

        The URI returned here is what we report to M03 in
        ``ReportResult.artifact_url``.
        """
        final_local = self.local_root / "final"
        final_local.mkdir(parents=True, exist_ok=True)
        method.save_artifact(
            model, final_local,
            extra={"is_final": True, "metrics": dict(metrics or {})},
        )
        (final_local / "metrics.json").write_text(
            json.dumps(dict(metrics or {}), indent=2), encoding="utf-8",
        )
        remote = None
        if self.remote_prefix:
            remote = (
                f"{self.remote_prefix.rstrip('/')}/{self.run_id}/final"
            )
        if remote and self.blob_store is not None:
            await self._upload_dir(final_local, remote)
        # flush pending ckpt uploads too
        await self.drain()
        return remote or f"file://{final_local}"

    async def drain(self) -> None:
        """Wait for every in-flight upload to settle."""
        if not self._pending:
            return
        pending = list(self._pending)
        self._pending.clear()
        for task in pending:
            try:
                await task
            except Exception as exc:
                _log.warning(
                    "ckpt_upload_failed",
                    error=str(exc),
                    type=type(exc).__name__,
                )

    # ---- upload helpers ----------------------------------------------

    async def _upload_dir(self, local: Path, remote_prefix: str) -> None:
        if self.blob_store is None:
            return
        try:
            for f in sorted(local.rglob("*")):
                if not f.is_file():
                    continue
                rel = f.relative_to(local).as_posix()
                uri = f"{remote_prefix.rstrip('/')}/{rel}"
                data = f.read_bytes()
                await self.blob_store.put(uri, data)
        except Exception as exc:
            raise StorageError(
                f"failed to upload {local} → {remote_prefix}: {exc}",
            ) from exc

    # ---- pruning ------------------------------------------------------

    def _prune(self) -> None:
        """Trim ``self._history`` down to at most ``keep_local`` directories.

        Keeping the ``best`` checkpoint even when it is the oldest is
        tricky: naive ``pop(0) then append(best)`` creates an infinite
        loop when *every* remaining entry is the best. We partition the
        history instead — the best (if any) is pinned in place, and we
        drop the N oldest non-best directories.
        """
        if len(self._history) <= self.keep_local:
            return
        best_dir = (
            self._local_dir(self._best_step)
            if self._best_step >= 0 else None
        )
        keepers: list[Path] = []
        drop: list[Path] = []
        # Walk newest → oldest so the "keep recent" budget is honoured.
        # After the split we restore chronological order.
        for d in reversed(self._history):
            if len(keepers) < self.keep_local or d == best_dir:
                keepers.append(d)
            else:
                drop.append(d)
        self._history = list(reversed(keepers))
        for old in drop:
            try:
                for p in sorted(old.rglob("*"), reverse=True):
                    if p.is_file():
                        p.unlink(missing_ok=True)
                    elif p.is_dir():
                        try:
                            p.rmdir()
                        except OSError:
                            pass
                old.rmdir()
            except OSError:
                pass


# ---------------------------------------------------------------------------
# resume helper — rebuild model state from a prior ckpt
# ---------------------------------------------------------------------------


async def download_ckpt_dir(
    blob_store: BlobStore, remote_prefix: str, local_target: Path,
) -> None:
    """Download every blob under ``remote_prefix`` into ``local_target``."""
    local_target.mkdir(parents=True, exist_ok=True)
    uris = await blob_store.list_prefix(remote_prefix)
    if not uris:
        raise WorkerError(
            f"no blobs under remote prefix {remote_prefix!r}",
        )
    for uri in uris:
        rel = uri[len(remote_prefix):].lstrip("/")
        if not rel:
            continue
        out = local_target / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        data = await blob_store.get(uri)
        out.write_bytes(data)


__all__ = ["CheckpointInfo", "CheckpointManager", "download_ckpt_dir"]

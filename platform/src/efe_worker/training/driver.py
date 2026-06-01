"""Training driver — the 9-stage pipeline.

Stages (per the M04 architecture doc §5):

    1. handshake + first HB
    2. fetch snapshot
    3. build model
    4. build sampler + dataloader
    5. training loop (method-specific ``compute_loss``)
    6. checkpoint every N steps (in-loop)
    7. eval on val set (periodic + final)
    8. upload artifact
    9. report result

The driver is agnostic of the underlying deep-learning framework
thanks to the :class:`TrainingMethod` protocol: it calls
``method.compute_loss`` and delegates the optimiser step to an
injected :class:`Optimizer` facade. Tests pass a pure-Python fake
method + fake optimiser.

Torch integration: when the concrete method uses torch (all 5 built-in
methods), the driver *optionally* builds a :class:`_TorchOptimizer`
that wraps :class:`torch.optim.AdamW` + a linear warmup scheduler.
The wrapper is built lazily — if torch is missing the tests using
fake methods still work.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Protocol

from efe_worker.context import RunContext
from efe_worker.data.samplers.base import BaseSampler
from efe_worker.data.snapshot import Row, Snapshot, fetch_snapshot
from efe_worker.data.tokenizer import FakeTokenizer, Tokenizer
from efe_worker.errors import (
    CudaOOM,
    InvalidHyperparameters,
    SnapshotMissing,
    UserDeadlineExceeded,
    WorkerError,
)
from efe_worker.methods.base import (
    Model,
    TrainingMethod,
)
from efe_worker.observability.logging import get_logger
from efe_worker.training.checkpoint import CheckpointManager, download_ckpt_dir
from efe_worker.training.early_stop import EarlyStopping
from efe_worker.training.eval import evaluate
from efe_worker.transport.grpc_client import (
    CheckpointRequest,
    HeartbeatRequest,
    ResultRequest,
)

_log = get_logger(__name__)


# ---------------------------------------------------------------------------
# Optimiser facade — lets tests inject fakes
# ---------------------------------------------------------------------------


class Optimizer(Protocol):
    def step(self, loss: Any) -> None:  # pragma: no cover
        ...

    def zero_grad(self) -> None:  # pragma: no cover
        ...

    def scheduler_step(self) -> None:  # pragma: no cover
        ...

    @property
    def lr(self) -> float:  # pragma: no cover
        ...


@dataclass
class FakeOptimizer:
    """Pure-Python optimiser for driver tests.

    Records every ``step`` call. The ``loss`` argument is accepted as
    a plain float (or any object exposing ``backward()``; the fake
    just records it).
    """

    steps: int = 0
    zeros: int = 0
    lr_value: float = 1e-4
    losses: list[float] = field(default_factory=list)

    def step(self, loss: Any) -> None:
        self.steps += 1
        try:
            self.losses.append(
                float(loss) if not hasattr(loss, "item") else float(loss.item()),
            )
        except Exception:
            self.losses.append(float("nan"))

    def zero_grad(self) -> None:
        self.zeros += 1

    def scheduler_step(self) -> None:
        return None

    @property
    def lr(self) -> float:
        return self.lr_value


def build_torch_optimizer(
    model: Model,
    *,
    learning_rate: float,
    weight_decay: float,
    warmup_ratio: float,
    max_steps: int,
    grad_clip: float,
) -> Optimizer:
    """Build a torch AdamW + linear warmup scheduler adapter.

    Raises :class:`efe_worker.errors.MissingDependency` if torch is
    not importable.
    """
    from efe_worker.methods.base import ensure_torch
    torch = ensure_torch()

    params = list(model.parameters())
    if hasattr(model, "trainable_parameters"):
        tp = model.trainable_parameters()
        if tp:
            params = tp
    if not params:
        raise InvalidHyperparameters(
            "model has no trainable parameters",
        )
    optim = torch.optim.AdamW(
        params,
        lr=float(learning_rate),
        weight_decay=float(weight_decay),
    )

    warmup_steps = max(1, int(max_steps * warmup_ratio))

    def lr_lambda(step: int) -> float:
        if step < warmup_steps:
            return float(step) / float(warmup_steps)
        progress = (step - warmup_steps) / max(1, max_steps - warmup_steps)
        return max(0.0, 1.0 - progress)

    scheduler = torch.optim.lr_scheduler.LambdaLR(optim, lr_lambda)

    class _TorchOptimizer:
        def step(self, loss: Any) -> None:
            loss.backward()
            if grad_clip > 0:
                torch.nn.utils.clip_grad_norm_(params, grad_clip)
            optim.step()

        def zero_grad(self) -> None:
            optim.zero_grad(set_to_none=True)

        def scheduler_step(self) -> None:
            scheduler.step()

        @property
        def lr(self) -> float:
            return float(optim.param_groups[0]["lr"])

    return _TorchOptimizer()


def _is_cuda_oom(exc: BaseException) -> bool:
    """Best-effort detection of a CUDA out-of-memory exception.

    torch raises :class:`torch.cuda.OutOfMemoryError` in modern
    versions but older / custom builds raise :class:`RuntimeError`
    with ``"out of memory"`` in the message. We check both. Works
    without importing torch at module load time.
    """
    cls_name = type(exc).__name__
    if cls_name in ("OutOfMemoryError", "CudaOOMError"):
        return True
    msg = str(exc).lower()
    return "out of memory" in msg or "cuda oom" in msg


# ---------------------------------------------------------------------------
# Outcome
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class TrainingOutcome:
    n_steps: int
    artifact_uri: str | None
    final_metrics: dict[str, float]
    best_metric_value: float | None
    best_step: int
    early_stopped: bool


# ---------------------------------------------------------------------------
# TrainingDriver
# ---------------------------------------------------------------------------


@dataclass
class TrainingDriver:
    """Orchestrates the 9 stages.

    Callers construct the driver with a :class:`RunContext`, a
    :class:`TrainingMethod`, a :class:`Tokenizer`, and
    (optionally) a pre-built optimiser. When the optimiser is omitted
    the driver tries to build one via :func:`build_torch_optimizer`.
    """

    ctx: RunContext
    method: TrainingMethod
    tokenizer: Tokenizer
    checkpoint_manager: CheckpointManager | None = None
    optimizer: Optimizer | None = None

    # Derived state (not set in __init__)
    snapshot: Snapshot | None = field(default=None, init=False)
    model: Model | None = field(default=None, init=False)
    sampler: BaseSampler | None = field(default=None, init=False)

    # ---- convenience ------------------------------------------------

    @property
    def device(self) -> Any:
        """Return a ``torch.device`` when torch is available.

        When torch is missing (tests with fake methods, CI without the
        heavy dep) we return the literal string ``"cpu"`` — this is
        only ever consumed by fake models that accept any value. The
        real method code paths always go through the torch branch.
        """
        pref = self.ctx.settings.device_preference
        try:
            from efe_worker.methods.base import ensure_torch
            torch = ensure_torch()
        except WorkerError:
            # Fake-method path only; the string is never passed to
            # a real torch tensor constructor.
            return "cpu"
        if pref == "cuda" and torch.cuda.is_available():
            return torch.device("cuda")
        if pref == "auto" and torch.cuda.is_available():
            return torch.device("cuda")
        return torch.device("cpu")

    # ---- stage 1 — handshake ----------------------------------------

    async def stage_handshake(self) -> None:
        """Send the first heartbeat to unblock M03's reaper."""
        self.ctx.set_phase("boot")
        if self.ctx.transport is None:
            return
        req = HeartbeatRequest(
            run_id=self.ctx.run_id,
            worker_id=self.ctx.worker_id,
            beat_seq=0,
            state_hint="boot",
            progress_ratio=0.0,
            step=0,
            extra={},
        )
        try:
            await self.ctx.transport.heartbeat(req)
        except Exception as exc:
            _log.warning(
                "handshake_hb_failed", error=str(exc),
            )

    # ---- stage 2 — fetch snapshot -----------------------------------

    async def stage_fetch_snapshot(self) -> Snapshot:
        self.ctx.set_phase("loading")
        uri = self.ctx.identity.snapshot_uri
        if not uri:
            raise SnapshotMissing(
                "settings.identity.snapshot_uri is empty",
            )
        if self.ctx.blob_store is None:
            raise WorkerError("no blob_store on context")
        snap = await fetch_snapshot(
            uri,
            self.ctx.blob_store,
            verify_shard_sha256=self.ctx.settings.data.verify_shard_sha256,
        )
        self.snapshot = snap
        return snap

    # ---- stage 3 — build model --------------------------------------

    def stage_build_model(self, snapshot: Snapshot) -> Model:
        hp = dict(self.ctx.settings.hyperparameters)
        # Auto-fill n_classes for methods that need it.
        if "n_classes" not in hp:
            hp["n_classes"] = len(snapshot.labels)
        model = self.method.build_model(
            self.ctx.settings.identity.base_model, hp,
        )
        model.to(self.device)
        self.model = model
        return model

    async def stage_maybe_resume(self, model: Model) -> int:
        """Download a prior checkpoint and warm-load the model.

        Returns the step counter at which to resume (0 = no resume).
        We honour :attr:`WorkerSettings.resume_checkpoint_uri` which
        the orchestrator sets when it re-queues a run (e.g. after a
        preemption). The URI is a remote directory — we pull the
        whole prefix into a scratch dir and then call
        :meth:`Model.load_state_dict` on ``model.pt``.

        Graceful degradation: if loading fails (missing file,
        shape mismatch) we log a warning and continue from step 0.
        The run is not aborted because a fresh run is still a valid
        outcome.
        """
        uri = self.ctx.settings.resume_checkpoint_uri
        if not uri:
            return 0
        if self.ctx.blob_store is None:
            _log.warning("resume_skipped_no_blob_store", uri=uri)
            return 0
        try:
            import tempfile
            tmp = Path(tempfile.mkdtemp(prefix="efe-resume-"))
            await download_ckpt_dir(self.ctx.blob_store, uri, tmp)
            # Try both "model.pt" and "student.pt" (distill) layout.
            candidates = ["model.pt", "student.pt"]
            target = None
            for name in candidates:
                p = tmp / name
                if p.is_file():
                    target = p
                    break
            if target is None:
                _log.warning(
                    "resume_skipped_no_state_dict",
                    uri=uri, tried=candidates,
                )
                return 0
            from efe_worker.methods.base import ensure_torch
            torch = ensure_torch()
            state = torch.load(target, map_location="cpu")
            model.load_state_dict(state, strict=False)
            # step.json gives us the step counter to resume from.
            step = 0
            step_file = tmp / "step.json"
            if step_file.is_file():
                try:
                    step = int(
                        json.loads(step_file.read_text())["step"],
                    )
                except (json.JSONDecodeError, KeyError, ValueError):
                    step = 0
            _log.info("resume_loaded", uri=uri, step=step)
            return step
        except Exception as exc:
            _log.warning(
                "resume_failed", uri=uri, error=str(exc),
                type=type(exc).__name__,
            )
            return 0

    # ---- stage 4 — build sampler + split ----------------------------

    def stage_build_sampler(
        self, snapshot: Snapshot,
    ) -> tuple[BaseSampler, list[Row], list[Row]]:
        train_rows, val_rows, _holdout = snapshot.partition()
        train_snap = Snapshot(manifest=snapshot.manifest, rows=train_rows)
        sampler = self.method.build_sampler(train_snap, self.tokenizer)
        self.sampler = sampler
        return sampler, val_rows, _holdout

    # ---- stage 5 — training loop ------------------------------------

    async def stage_training_loop(
        self,
        sampler: BaseSampler,
        val_rows: list[Row],
        model: Model,
        *,
        start_step: int = 0,
    ) -> tuple[int, dict[str, float], bool, int]:
        """Run the main loop. Returns (n_steps, final_metrics, early,
        best_step).

        ``start_step`` is the step counter at which to resume — this
        is 0 on a fresh run and ``meta['step']`` when
        :meth:`stage_maybe_resume` restored a prior checkpoint.
        """
        self.ctx.set_phase("training")
        self.ctx.mark_started()
        if self.optimizer is None:
            cfg = self.ctx.settings.training
            self.optimizer = build_torch_optimizer(
                model,
                learning_rate=cfg.learning_rate,
                weight_decay=cfg.weight_decay,
                warmup_ratio=cfg.warmup_ratio,
                max_steps=cfg.max_steps,
                grad_clip=cfg.grad_clip,
            )
        optimizer = self.optimizer
        batch_iter = sampler.iter_batches(
            self.ctx.settings.training.batch_size,
        )
        early = EarlyStopping(
            metric=self.ctx.settings.checkpoint.best_metric,
            patience=self.ctx.settings.training.early_stopping_patience,
            mode=self.ctx.settings.checkpoint.best_metric_mode,
        )
        final_metrics: dict[str, float] = {}
        early_stopped = False
        model.train()
        step = int(start_step)
        if step > 0:
            self.ctx.step = step
        max_steps = self.ctx.settings.training.max_steps
        eval_every = self.ctx.settings.training.eval_every_n_steps
        ckpt_every = self.ctx.settings.checkpoint.every_n_steps
        for batch in batch_iter:
            if self.ctx.should_stop() or step >= max_steps:
                break
            if self.ctx.user_deadline_reached():
                _log.info("user_deadline_reached", step=step)
                # Surface as a distinct error so M03 can classify it
                # as TIMEOUT and decide retry vs budget exhaustion.
                raise UserDeadlineExceeded(
                    f"user deadline reached at step {step}",
                    detail={"step": step},
                )
            step += 1
            # Forward + backward. CUDA OOM is the most common
            # non-fatal failure on GPU pools; we classify it as a
            # dedicated error so M03 can retry on a bigger pool.
            try:
                out = self.method.compute_loss(
                    model, batch, step=step, device=self.device,
                    tokenizer=self.tokenizer,
                )
            except Exception as exc:
                if _is_cuda_oom(exc):
                    raise CudaOOM(
                        f"CUDA OOM at step {step}: {exc}",
                        detail={"step": step,
                                "original": type(exc).__name__},
                    ) from exc
                raise
            optimizer.zero_grad()
            try:
                optimizer.step(out.loss)
            except Exception as exc:
                if _is_cuda_oom(exc):
                    raise CudaOOM(
                        f"CUDA OOM during optim step {step}: {exc}",
                        detail={"step": step},
                    ) from exc
                raise
            optimizer.scheduler_step()
            # Update live context
            self.ctx.record(
                step=step, loss=out.scalar, lr=optimizer.lr,
                **out.metrics,
            )
            self.ctx.progress_ratio = step / float(max_steps)
            # Periodic eval
            if val_rows and step % eval_every == 0:
                val_metrics = evaluate(
                    self.method, model, val_rows, self.tokenizer,
                    device=self.device,
                )
                final_metrics.update(val_metrics)
                # Keys may contain slashes (``val/loss``) which are not
                # valid Python identifiers, so update the metrics map
                # directly rather than using **kwargs.
                for k, v in val_metrics.items():
                    try:
                        self.ctx.live_metrics[k] = float(v)
                    except (TypeError, ValueError):
                        continue
                if early.step(val_metrics, at_step=step):
                    _log.info(
                        "early_stopped",
                        step=step,
                        best=early.best,
                        best_step=early.best_step,
                    )
                    early_stopped = True
                    break
            # Periodic checkpoint
            if (
                self.checkpoint_manager is not None
                and step % ckpt_every == 0
            ):
                info = await self.checkpoint_manager.save(
                    self.method, model,
                    step=step, metrics=dict(self.ctx.live_metrics),
                )
                if self.ctx.transport is not None:
                    try:
                        await self.ctx.transport.report_checkpoint(
                            CheckpointRequest(
                                run_id=self.ctx.run_id,
                                worker_id=self.ctx.worker_id,
                                step=step,
                                blob_uri=info.remote_uri or str(info.local_dir),
                                metrics=info.metrics,
                                size_bytes=info.size_bytes,
                                is_best=info.is_best,
                            ),
                        )
                    except Exception as exc:
                        _log.warning(
                            "report_ckpt_failed",
                            step=step, error=str(exc),
                        )
        return step, final_metrics, early_stopped, early.best_step

    # ---- stage 7 — final eval ---------------------------------------

    def stage_eval(
        self, model: Model, val_rows: list[Row],
    ) -> dict[str, float]:
        self.ctx.set_phase("eval")
        if not val_rows:
            return {}
        return evaluate(
            self.method, model, val_rows, self.tokenizer, device=self.device,
        )

    # ---- stage 8 — upload artifact ----------------------------------

    async def stage_upload(
        self, model: Model, metrics: dict[str, float],
    ) -> str:
        self.ctx.set_phase("upload")
        if self.checkpoint_manager is not None:
            return await self.checkpoint_manager.upload_final(
                self.method, model, metrics=metrics,
            )
        # No checkpoint manager: serialize to a tmp dir and return file://
        tmp = Path("/tmp") / f"efe-artifact-{self.ctx.run_id}"
        tmp.mkdir(parents=True, exist_ok=True)
        self.method.save_artifact(
            model, tmp, extra={"metrics": dict(metrics)},
        )
        return f"file://{tmp}"

    # ---- stage 9 — report result ------------------------------------

    async def stage_report(
        self, artifact_uri: str, metrics: dict[str, float],
    ) -> None:
        self.ctx.set_phase("done")
        if self.ctx.transport is None:
            return
        await self.ctx.transport.report_result(
            ResultRequest(
                run_id=self.ctx.run_id,
                worker_id=self.ctx.worker_id,
                artifact_url=artifact_uri,
                metrics=dict(metrics),
            ),
        )

    # ---- top-level ---------------------------------------------------

    async def run(self) -> TrainingOutcome:
        await self.stage_handshake()
        snapshot = await self.stage_fetch_snapshot()
        model = self.stage_build_model(snapshot)
        start_step = await self.stage_maybe_resume(model)
        sampler, val_rows, _ = self.stage_build_sampler(snapshot)
        n_steps, final_metrics, early_stopped, best_step = (
            await self.stage_training_loop(
                sampler, val_rows, model, start_step=start_step,
            )
        )
        eval_final = self.stage_eval(model, val_rows)
        final_metrics.update(eval_final)
        artifact_uri = await self.stage_upload(model, final_metrics)
        await self.stage_report(artifact_uri, final_metrics)
        best_value = None
        if self.ctx.settings.checkpoint.best_metric in final_metrics:
            best_value = final_metrics[
                self.ctx.settings.checkpoint.best_metric
            ]
        return TrainingOutcome(
            n_steps=n_steps,
            artifact_uri=artifact_uri,
            final_metrics=final_metrics,
            best_metric_value=best_value,
            best_step=best_step,
            early_stopped=early_stopped,
        )


# ---------------------------------------------------------------------------
# Convenience top-level coroutine
# ---------------------------------------------------------------------------


async def run_training(
    ctx: RunContext,
    method: TrainingMethod,
    tokenizer: Tokenizer | None = None,
    *,
    checkpoint_manager: CheckpointManager | None = None,
    optimizer: Optimizer | None = None,
) -> TrainingOutcome:
    """One-line helper: construct a :class:`TrainingDriver` and run it."""
    tk = tokenizer or FakeTokenizer(
        max_seq_len=ctx.settings.data.max_seq_len,
    )
    driver = TrainingDriver(
        ctx=ctx,
        method=method,
        tokenizer=tk,
        checkpoint_manager=checkpoint_manager,
        optimizer=optimizer,
    )
    return await driver.run()


__all__ = [
    "FakeOptimizer",
    "Optimizer",
    "TrainingDriver",
    "TrainingOutcome",
    "build_torch_optimizer",
    "run_training",
]

"""CLI entry point: ``python -m efe_worker``.

Subcommands:

    run        — execute a single training run from ENV (production).
    config     — dump resolved config as JSON.
    list-methods — print the registered methods.
    describe-method <name> — show hyperparameter defaults.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import signal
import sys
from pathlib import Path
from typing import Any

from efe_worker.config import WorkerSettings
from efe_worker.context import RunContext
from efe_worker.errors import SigtermGraceful, WorkerError
from efe_worker.methods import METHOD_REGISTRY, resolve_method
from efe_worker.observability.logging import get_logger, init_logging
from efe_worker.training.checkpoint import CheckpointManager
from efe_worker.training.driver import run_training
from efe_worker.transport.grpc_client import (
    ErrorRequest,
    FakeOrchestratorClient,
    HttpOrchestratorClient,
    OrchestratorClient,
)
from efe_worker.transport.heartbeat import heartbeat_loop
from efe_worker.transport.s3 import FakeBlobStore, LocalBlobStore

_log = get_logger("efe_worker.main")


def _parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="efe_worker",
        description="EFE Training Worker (M04) CLI.",
    )
    sub = p.add_subparsers(dest="command", required=True)
    sub.add_parser("run", help="Run a single training run.")
    sub.add_parser("config", help="Print resolved config.")
    sub.add_parser("list-methods", help="List registered methods.")
    d = sub.add_parser("describe-method",
                        help="Describe a method's defaults.")
    d.add_argument("name")
    return p.parse_args(argv)


def _build_transport(cfg: WorkerSettings) -> OrchestratorClient:
    if cfg.transport.fake_transport:
        return FakeOrchestratorClient()
    return HttpOrchestratorClient(
        cfg.transport.callback_url,
        timeout_s=cfg.transport.rpc_timeout_s,
        max_retries=cfg.transport.rpc_max_retries,
    )


def _build_blob_store(cfg: WorkerSettings) -> Any:
    if cfg.transport.fake_transport:
        return FakeBlobStore()
    return LocalBlobStore(cfg.data.cache_dir)


async def _cmd_run() -> int:
    init_logging(
        level="INFO", json=True,
    )
    cfg = WorkerSettings.from_env()
    transport = _build_transport(cfg)
    blob_store = _build_blob_store(cfg)
    ctx = RunContext(
        settings=cfg, transport=transport, blob_store=blob_store,
    )
    # SIGTERM handler: request graceful stop.
    loop = asyncio.get_running_loop()

    def _on_sigterm() -> None:
        _log.warning("sigterm_received")
        ctx.request_stop()

    try:
        loop.add_signal_handler(signal.SIGTERM, _on_sigterm)
    except (NotImplementedError, RuntimeError):
        # Windows or restricted environments; skip.
        pass

    method = resolve_method(cfg.method_name, cfg.hyperparameters)
    ckpt = CheckpointManager(
        run_id=ctx.run_id,
        local_root=Path(cfg.checkpoint.local_dir),
        remote_prefix=cfg.checkpoint.remote_uri_prefix,
        blob_store=blob_store,
        keep_local=cfg.checkpoint.keep_local,
        best_metric=cfg.checkpoint.best_metric,
        best_mode=cfg.checkpoint.best_metric_mode,
    )
    hb_task = asyncio.create_task(
        heartbeat_loop(transport, ctx), name="hb",
    )
    try:
        outcome = await run_training(
            ctx, method, checkpoint_manager=ckpt,
        )
        _log.info(
            "run_succeeded",
            n_steps=outcome.n_steps,
            artifact=outcome.artifact_uri,
        )
        return 0
    except WorkerError as exc:
        _log.error(
            "run_failed",
            code=exc.code,
            message=exc.message,
        )
        try:
            await transport.report_error(ErrorRequest(
                run_id=ctx.run_id,
                worker_id=ctx.worker_id,
                error_code=exc.code,
                error_message=exc.message,
            ))
        except Exception as exc2:
            _log.error("report_error_failed", error=str(exc2))
        return 1
    except BaseException as exc:
        # Last-resort capture. ``BaseException`` catches e.g.
        # KeyboardInterrupt and asyncio.CancelledError too; we
        # must never leave the orchestrator wondering.
        code = _classify_exception(exc)
        _log.error(
            "run_crashed",
            error=str(exc),
            type=type(exc).__name__,
            code=code,
        )
        try:
            await transport.report_error(ErrorRequest(
                run_id=ctx.run_id,
                worker_id=ctx.worker_id,
                error_code=code,
                error_message=str(exc)[:1000],
            ))
        except Exception:
            pass
        return 2
    finally:
        ctx.request_stop()
        # Flush any pending checkpoint uploads so the last-observed
        # state is actually on S3 before the pod dies. Best-effort:
        # we log on error but don't surface it (the run result has
        # already been reported).
        try:
            await ckpt.drain()
        except Exception as exc:
            _log.warning("ckpt_drain_failed", error=str(exc))
        try:
            await asyncio.wait_for(hb_task, timeout=5.0)
        except (asyncio.TimeoutError, asyncio.CancelledError):
            hb_task.cancel()
        await transport.close()


def _classify_exception(exc: BaseException) -> str:
    """Map an arbitrary exception into a canonical ``error_code``.

    Mirrors the taxonomy in :mod:`efe_worker.errors`. Used by the
    last-resort handler in :func:`_cmd_run` so even crashes land with
    a stable code on M03's side.
    """
    # Torch-specific (lazy import; only match if torch is loaded).
    cls_name = type(exc).__name__
    if cls_name in ("OutOfMemoryError", "CudaOOMError"):
        return "cuda_oom"
    if cls_name == "MemoryError":
        return "worker_oom"
    if isinstance(exc, KeyboardInterrupt):
        return "sigterm_graceful"
    # Fallback.
    return "training_error"


async def _cmd_config() -> int:
    cfg = WorkerSettings.from_env()
    print(json.dumps(cfg.model_dump(), indent=2, default=str))
    return 0


def _cmd_list_methods() -> int:
    print("\n".join(sorted(METHOD_REGISTRY.keys())))
    return 0


def _cmd_describe_method(name: str) -> int:
    if name not in METHOD_REGISTRY:
        print(f"unknown method: {name!r}", file=sys.stderr)
        return 1
    factory = METHOD_REGISTRY[name]
    try:
        inst = factory({})
    except WorkerError as exc:
        print(f"method requires hyperparameters: {exc.message}",
              file=sys.stderr)
        return 0
    payload = {
        "name": getattr(inst, "name", name),
    }
    for attr in (
        "temperature", "normalize", "margin", "strategy",
        "dropout", "label_smoothing", "pool", "r", "alpha",
        "lora_dropout", "target_modules", "teacher_model",
    ):
        if hasattr(inst, attr):
            payload[attr] = getattr(inst, attr)
    print(json.dumps(payload, indent=2, default=str))
    return 0


def main(argv: list[str] | None = None) -> int:
    ns = _parse_args(argv if argv is not None else sys.argv[1:])
    if ns.command == "run":
        return asyncio.run(_cmd_run())
    if ns.command == "config":
        return asyncio.run(_cmd_config())
    if ns.command == "list-methods":
        return _cmd_list_methods()
    if ns.command == "describe-method":
        return _cmd_describe_method(ns.name)
    raise AssertionError(
        f"unknown command {ns.command!r}",
    )  # pragma: no cover


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())


# Silence unused-import warning from static checkers
_ = SigtermGraceful

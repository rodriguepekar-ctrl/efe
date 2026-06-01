"""Protocol + helpers for training methods.

The :class:`TrainingMethod` protocol is the *single* contract the
driver knows about. Every concrete method (A-E) implements it.

Why a Protocol rather than an ABC?

* We want method implementations to be independent modules that can
  optionally import torch — an ABC would force a common base import,
  which is annoying for modules that are themselves torch-only.
* Tests register fake methods without inheriting from anything.

Two helpers sit alongside:

* :class:`Model` — shadow of :class:`torch.nn.Module` exposing only
  the methods the driver actually needs (``to``, ``train``, ``eval``,
  ``parameters``, ``state_dict``). Lets us write typed driver code
  without importing torch.
* :func:`ensure_torch` — lazy-import helper that raises
  :class:`MissingDependency` if torch is absent.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Protocol

from efe_worker.data.samplers.base import BaseSampler, Batch
from efe_worker.data.snapshot import Snapshot
from efe_worker.data.tokenizer import Tokenizer
from efe_worker.errors import MissingDependency


@dataclass(frozen=True, slots=True)
class LossOutput:
    """What :meth:`TrainingMethod.compute_loss` returns."""

    loss: Any  # torch.Tensor at runtime; Any to stay torch-free here
    scalar: float
    metrics: dict[str, float] = field(default_factory=dict)


class Model(Protocol):
    """Narrow shadow of :class:`torch.nn.Module`.

    The driver only calls a handful of methods; this protocol makes
    the contract explicit and lets us mock ``Model`` in tests without
    importing torch.
    """

    def to(self, device: Any) -> Model:  # pragma: no cover
        ...

    def train(self, mode: bool = True) -> Model:  # pragma: no cover
        ...

    def eval(self) -> Model:  # pragma: no cover
        ...

    def parameters(self) -> Iterable[Any]:  # pragma: no cover
        ...

    def state_dict(self) -> dict[str, Any]:  # pragma: no cover
        ...

    def load_state_dict(
        self, state: dict[str, Any], strict: bool = True,
    ) -> Any:  # pragma: no cover
        ...


class TrainingMethod(Protocol):
    """Every method implements this exact surface."""

    name: str
    """Canonical name used by the registry / env var."""

    def build_model(
        self, base_model: str, hyperparameters: dict[str, Any],
    ) -> Model:  # pragma: no cover
        ...

    def build_sampler(
        self, snapshot: Snapshot, tokenizer: Tokenizer,
    ) -> BaseSampler:  # pragma: no cover
        ...

    def compute_loss(
        self, model: Model, batch: Batch, *, step: int, device: Any,
        tokenizer: Tokenizer,
    ) -> LossOutput:  # pragma: no cover
        ...

    def eval_metrics(
        self, model: Model, val_rows: list[Any], tokenizer: Tokenizer,
        *, device: Any,
    ) -> dict[str, float]:  # pragma: no cover
        ...

    def save_artifact(
        self, model: Model, out_dir: Path, *, extra: dict[str, Any] | None = None,
    ) -> dict[str, Any]:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# lazy torch loader
# ---------------------------------------------------------------------------


def ensure_torch() -> Any:
    """Return the ``torch`` module or raise :class:`MissingDependency`."""
    try:
        import torch
    except ImportError as exc:  # pragma: no cover — depends on env
        raise MissingDependency(
            "torch is required for training methods; "
            "install with: pip install 'torch>=2.3'",
        ) from exc
    return torch


def ensure_transformers() -> Any:
    try:
        import transformers
    except ImportError as exc:  # pragma: no cover — depends on env
        raise MissingDependency(
            "transformers is required for this method; "
            "install with: pip install 'transformers>=4.41'",
        ) from exc
    return transformers


def ensure_peft() -> Any:
    try:
        import peft
    except ImportError as exc:
        raise MissingDependency(
            "peft is required for LoRA; "
            "install with: pip install 'peft>=0.11'",
        ) from exc
    return peft


__all__ = [
    "LossOutput",
    "Model",
    "TrainingMethod",
    "ensure_peft",
    "ensure_torch",
    "ensure_transformers",
]

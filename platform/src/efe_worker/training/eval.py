"""Evaluation harness — method-agnostic wrapper."""

from __future__ import annotations

from typing import Any

from efe_worker.data.snapshot import Row
from efe_worker.data.tokenizer import Tokenizer
from efe_worker.methods.base import Model, TrainingMethod


def evaluate(
    method: TrainingMethod,
    model: Model,
    val_rows: list[Row],
    tokenizer: Tokenizer,
    *,
    device: Any,
) -> dict[str, float]:
    """Switch the model to eval mode, collect metrics, switch back."""
    if not val_rows:
        return {}
    model.eval()
    try:
        metrics = method.eval_metrics(
            model, val_rows, tokenizer, device=device,
        )
    finally:
        model.train()
    # Normalise keys to "val/<m>" to match the early-stop + metrics schema.
    out: dict[str, float] = {}
    for k, v in metrics.items():
        key = k if k.startswith("val/") else f"val/{k}"
        out[key] = float(v)
    return out


__all__ = ["evaluate"]

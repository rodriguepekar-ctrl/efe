"""Five fine-tuning methods.

Each method is a class that implements :class:`TrainingMethod`. They
share the outer driver but control:

* the model architecture (``build_model``),
* the sampler used to batch rows (``build_sampler``),
* the loss function (``compute_loss``),
* the evaluation metrics (``eval_metrics``),
* the artifact layout (``save_artifact``).

Lookup via :func:`METHOD_REGISTRY` returns the class by canonical
name (``infonce`` | ``triplet`` | ``classif`` | ``lora`` | ``distill``).
"""

from __future__ import annotations

from efe_worker.methods.base import (
    LossOutput,
    Model,
    TrainingMethod,
    ensure_torch,
)
from efe_worker.methods.registry import (
    METHOD_REGISTRY,
    register_method,
    resolve_method,
)

__all__ = [
    "METHOD_REGISTRY",
    "LossOutput",
    "Model",
    "TrainingMethod",
    "ensure_torch",
    "register_method",
    "resolve_method",
]

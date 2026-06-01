"""Training layer — outer driver, checkpoint manager, eval harness.

Public surface:

    run_training(ctx, method)  — the 9-stage pipeline (async)
    TrainingDriver             — class form, used when tests want
                                 fine-grained control over stages
    CheckpointManager
    EarlyStopping
    evaluate
"""

from __future__ import annotations

from efe_worker.training.checkpoint import CheckpointInfo, CheckpointManager
from efe_worker.training.driver import (
    TrainingDriver,
    TrainingOutcome,
    run_training,
)
from efe_worker.training.early_stop import EarlyStopping
from efe_worker.training.eval import evaluate

__all__ = [
    "CheckpointInfo",
    "CheckpointManager",
    "EarlyStopping",
    "TrainingDriver",
    "TrainingOutcome",
    "evaluate",
    "run_training",
]

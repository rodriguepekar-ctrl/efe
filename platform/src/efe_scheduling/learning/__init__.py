"""Continuous-learning decisions and orchestration.

Six pure decision engines — each exposes ``decide(state, inputs, cfg) →
Decision`` — plus three executor jobs (training, validation, canary) and
one glue pipeline (:class:`ContinuousLearningPipeline`).
"""

from __future__ import annotations

from efe_scheduling.learning.baseline_refresh import (
    BaselineRefreshDecision,
    BaselineRefreshInputs,
    BaselineRefreshOutcome,
    decide_baseline_refresh,
)
from efe_scheduling.learning.canary import (
    CanaryReport,
    CanaryTrafficStage,
)
from efe_scheduling.learning.curator import (
    CuratorInputs,
    CuratorOutcome,
    DatasetManifest,
    build_dataset_manifest,
)
from efe_scheduling.learning.pipeline import (
    ContinuousLearningPipeline,
    LearningConfig,
)
from efe_scheduling.learning.promotion import (
    PromotionDecision,
    PromotionInputs,
    decide_promotion,
)
from efe_scheduling.learning.retrain import (
    RetrainDecision,
    RetrainInputs,
    RetrainOutcome,
    decide_retrain,
)
from efe_scheduling.learning.rollback import (
    RollbackDecision,
    RollbackInputs,
    decide_rollback,
)
from efe_scheduling.learning.validation import (
    ValidationReport,
    ValidationThresholds,
    validate_report,
)

__all__ = [
    "BaselineRefreshDecision",
    "BaselineRefreshInputs",
    "BaselineRefreshOutcome",
    "CanaryReport",
    "CanaryTrafficStage",
    "ContinuousLearningPipeline",
    "CuratorInputs",
    "CuratorOutcome",
    "DatasetManifest",
    "LearningConfig",
    "PromotionDecision",
    "PromotionInputs",
    "RetrainDecision",
    "RetrainInputs",
    "RetrainOutcome",
    "RollbackDecision",
    "RollbackInputs",
    "ValidationReport",
    "ValidationThresholds",
    "build_dataset_manifest",
    "decide_baseline_refresh",
    "decide_promotion",
    "decide_retrain",
    "decide_rollback",
    "validate_report",
]

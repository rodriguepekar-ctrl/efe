"""Drift-detection plane — L4."""

from __future__ import annotations

from efe_drift.detection.base import (
    DetectionContext,
    DriftDetector,
    DriftMethod,
    DriftResult,
    MultiScaleWindow,
)
from efe_drift.detection.chi_squared import (
    ChiSquaredDetector,
    chi_squared_goodness_of_fit,
    g_test_statistic,
)
from efe_drift.detection.classifier import (
    ClassifierDriftDetector,
    classifier_drift_score,
)
from efe_drift.detection.evaluator import (
    DriftEvaluator,
    EvaluationSweep,
    InMemorySnapshotSource,
    SnapshotSource,
)
from efe_drift.detection.ks import KsDetector
from efe_drift.detection.mmd import (
    MmdDetector,
    centroid_distance,
    mmd_permutation_p_value,
    mmd_squared_rbf,
)
from efe_drift.detection.psi import (
    JsDivergenceDetector,
    PsiDetector,
    compute_psi,
    js_divergence,
    per_bin_contributions,
)
from efe_drift.detection.registry import (
    DetectorRegistry,
    build_default_registry,
)
from efe_drift.detection.sequential import ScaleWindow, compute_scale_windows
from efe_drift.detection.severity import (
    DEFAULTS,
    Orientation,
    Severity,
    SeverityThresholds,
)
from efe_drift.detection.stats import (
    chi2_cdf,
    kolmogorov_p_value,
    ks_two_sample,
    normal_cdf,
    normal_ppf,
    rank_auc,
    student_t_cdf,
)
from efe_drift.detection.wasserstein import (
    WassersteinDetector,
    wasserstein_1d,
    wasserstein_1d_sorted,
)

__all__ = [
    "DEFAULTS",
    "ChiSquaredDetector",
    "ClassifierDriftDetector",
    "DetectionContext",
    "DetectorRegistry",
    "DriftDetector",
    "DriftEvaluator",
    "DriftMethod",
    "DriftResult",
    "EvaluationSweep",
    "InMemorySnapshotSource",
    "JsDivergenceDetector",
    "KsDetector",
    "MmdDetector",
    "MultiScaleWindow",
    "Orientation",
    "PsiDetector",
    "ScaleWindow",
    "Severity",
    "SeverityThresholds",
    "SnapshotSource",
    "WassersteinDetector",
    "build_default_registry",
    "centroid_distance",
    "chi2_cdf",
    "chi_squared_goodness_of_fit",
    "classifier_drift_score",
    "compute_psi",
    "compute_scale_windows",
    "g_test_statistic",
    "js_divergence",
    "kolmogorov_p_value",
    "ks_two_sample",
    "mmd_permutation_p_value",
    "mmd_squared_rbf",
    "normal_cdf",
    "normal_ppf",
    "per_bin_contributions",
    "rank_auc",
    "student_t_cdf",
    "wasserstein_1d",
    "wasserstein_1d_sorted",
]

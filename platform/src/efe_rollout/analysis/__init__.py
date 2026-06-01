"""Statistical analysis pipeline."""

from __future__ import annotations

from efe_rollout.analysis.analyzer import (
    AnalysisInputs,
    AnalysisKind,
    ExperimentAnalyzer,
)
from efe_rollout.analysis.bayesian import (
    BayesianPosterior,
    bayesian_beta_binomial,
    bayesian_decision,
)
from efe_rollout.analysis.cluster import (
    ClusterRobustResult,
    cluster_robust_se,
)
from efe_rollout.analysis.cuped import CupedResult, apply_cuped
from efe_rollout.analysis.hte import SegmentTestResult, subgroup_analysis
from efe_rollout.analysis.meta import (
    MetaAnalysisResult,
    StudySummary,
    meta_analyze,
)
from efe_rollout.analysis.power import (
    PowerCalcResult,
    detectable_effect_proportions,
    sample_size_means,
    sample_size_proportions,
)
from efe_rollout.analysis.sequential import AlphaSpending
from efe_rollout.analysis.srm import SrmReport, detect_srm
from efe_rollout.analysis.stats import (
    MeanTestResult,
    PropTestResult,
    bh_adjusted_p,
    bh_correction,
    chi2_cdf,
    newcombe_ci,
    normal_cdf,
    normal_ppf,
    student_t_cdf,
    wald_prop_test,
    welch_t_test,
)

__all__ = [
    "AlphaSpending",
    "AnalysisInputs",
    "AnalysisKind",
    "BayesianPosterior",
    "ClusterRobustResult",
    "CupedResult",
    "ExperimentAnalyzer",
    "MeanTestResult",
    "MetaAnalysisResult",
    "PowerCalcResult",
    "PropTestResult",
    "SegmentTestResult",
    "SrmReport",
    "StudySummary",
    "apply_cuped",
    "bayesian_beta_binomial",
    "bayesian_decision",
    "bh_adjusted_p",
    "bh_correction",
    "chi2_cdf",
    "cluster_robust_se",
    "detect_srm",
    "detectable_effect_proportions",
    "meta_analyze",
    "newcombe_ci",
    "normal_cdf",
    "normal_ppf",
    "sample_size_means",
    "sample_size_proportions",
    "student_t_cdf",
    "subgroup_analysis",
    "wald_prop_test",
    "welch_t_test",
]

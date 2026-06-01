"""DerSimonian-Laird meta-analysis."""

from __future__ import annotations

from efe_rollout.analysis.meta import StudySummary, meta_analyze


def test_single_study_pools_to_itself():
    studies = [StudySummary(study_id="s1", estimate=0.1, variance=0.04)]
    result = meta_analyze(studies)
    assert result.k == 1
    assert result.pooled_estimate == 0.1
    assert result.tau_squared == 0.0
    assert result.i_squared == 0.0


def test_homogeneous_studies_have_low_tau_squared():
    studies = [
        StudySummary(study_id=f"s{i}", estimate=0.1, variance=0.01)
        for i in range(5)
    ]
    result = meta_analyze(studies)
    assert abs(result.pooled_estimate - 0.1) < 1e-6
    assert result.tau_squared == 0.0
    assert result.i_squared == 0.0


def test_heterogeneous_studies_have_positive_tau_squared():
    studies = [
        StudySummary(study_id="s1", estimate=0.1, variance=0.005),
        StudySummary(study_id="s2", estimate=0.5, variance=0.005),
        StudySummary(study_id="s3", estimate=-0.2, variance=0.005),
        StudySummary(study_id="s4", estimate=0.3, variance=0.005),
    ]
    result = meta_analyze(studies)
    assert result.tau_squared > 0
    assert result.i_squared > 0.5

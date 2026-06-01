"""Heterogeneous Treatment Effects (HTE) + segment analysis.

The "Average Treatment Effect" (ATE) is what an A/B directly reports,
but it can *mask* very different sub-group effects — that's the
Simpson-paradox-lite scenario every experimenter hits eventually.

:func:`subgroup_analysis` splits outcomes by a segment key and runs
the same test per sub-group, returning a dict of
:class:`~efe_rollout.analysis.stats.MeanTestResult` values. An optional
BH correction is applied on the set of p-values.

Typical segment keys: ``"country"``, ``"device"``, ``"tenant_tier"``,
``"first_week_user"``. Values are usually strings or bools.
"""

from __future__ import annotations

from collections import defaultdict
from collections.abc import Iterable
from dataclasses import dataclass

from efe_rollout.analysis.stats import (
    MeanTestResult,
    bh_adjusted_p,
    welch_t_test,
)
from efe_rollout.records import Outcome


@dataclass(frozen=True, slots=True)
class SegmentTestResult:
    result: MeanTestResult
    q_value: float                 # BH-adjusted p-value
    significant: bool              # q_value ≤ alpha after BH
    n_control: int
    n_treatment: int


def subgroup_analysis(
    outcomes: Iterable[Outcome],
    *,
    segment_key: str,
    control_variant_id: str,
    treatment_variant_id: str,
    alpha: float = 0.05,
    apply_bh: bool = True,
) -> dict[str, SegmentTestResult]:
    """Per-segment Welch t-test of ``treatment - control``."""
    buckets_control: dict[str, list[float]] = defaultdict(list)
    buckets_treatment: dict[str, list[float]] = defaultdict(list)
    for o in outcomes:
        seg_val = str(o.covariate.get(segment_key, "__unknown__"))
        if o.variant_id == control_variant_id:
            buckets_control[seg_val].append(o.metric_value)
        elif o.variant_id == treatment_variant_id:
            buckets_treatment[seg_val].append(o.metric_value)

    segments = sorted(set(buckets_control) | set(buckets_treatment))
    per_segment_results: dict[str, MeanTestResult] = {}
    per_segment_n: dict[str, tuple[int, int]] = {}
    for seg in segments:
        ctrl = buckets_control.get(seg, [])
        trt = buckets_treatment.get(seg, [])
        result = welch_t_test(ctrl, trt, alpha=alpha)
        per_segment_results[seg] = result
        per_segment_n[seg] = (len(ctrl), len(trt))

    # BH adjustment on the vector of p-values.
    if apply_bh and segments:
        pvals = [per_segment_results[s].p_value for s in segments]
        # non-NaN substitution — NaN gets q=1
        safe = [p if p == p else 1.0 for p in pvals]  # NaN!=NaN
        q = bh_adjusted_p(safe)
    else:
        q = [
            per_segment_results[s].p_value
            for s in segments
        ]

    out: dict[str, SegmentTestResult] = {}
    for i, seg in enumerate(segments):
        res = per_segment_results[seg]
        n_c, n_t = per_segment_n[seg]
        q_val = q[i] if i < len(q) else 1.0
        out[seg] = SegmentTestResult(
            result=res,
            q_value=q_val,
            significant=bool(q_val <= alpha and q_val == q_val),
            n_control=n_c,
            n_treatment=n_t,
        )
    return out


__all__ = ["SegmentTestResult", "subgroup_analysis"]

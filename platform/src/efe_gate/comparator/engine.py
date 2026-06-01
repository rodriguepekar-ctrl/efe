"""Metric-level comparator.

The :class:`ComparisonEngine` is a **pure** function: given two
:class:`MetricsBundle` (champion + candidate) it returns a list of
:class:`MetricComparison` entries, one per metric. Each entry carries
the observed delta, a CI if applicable, the test name used, and a
verdict (``pass`` / ``fail`` / ``ambiguous`` / ``better``).

The engine does **not** decide promotion — that's the next stage
(:mod:`efe_gate.decision`). It only surfaces the statistical truth
per metric. This separation matters because the same comparison
surface is consumed by:

* the decision engine (live promotion),
* the review UI (human looks at per-metric deltas),
* the rollback daemon (post-promote drift detection).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any

import numpy as np

from efe_gate.errors import StatisticalTestError
from efe_gate.metrics.bundle import (
    MetricBundle,
    MetricRole,
    MetricsBundle,
    SampleScores,
)
from efe_gate.metrics.catalog import TestName, resolve_metric
from efe_gate.metrics.stats import (
    bootstrap_delta_ci,
    mcnemar_test,
    tost_equivalence,
    welch_test,
)


class MetricComparisonVerdict(str, Enum):
    PASS = "pass"
    FAIL = "fail"
    AMBIGUOUS = "ambiguous"
    BETTER = "better"
    BUDGET_EXCEEDED = "budget_exceeded"
    UNCOMPARABLE = "uncomparable"   # missing from one side


@dataclass(frozen=True, slots=True)
class MetricComparison:
    metric_name: str
    role: MetricRole
    verdict: MetricComparisonVerdict
    champion_value: float | None
    candidate_value: float
    delta: float | None
    ci_lower: float | None
    ci_upper: float | None
    p_value: float | None
    test: str
    sample_size: int
    higher_is_better: bool
    reason: str = ""
    detail: dict[str, Any] = field(default_factory=dict)

    def as_dict(self) -> dict[str, Any]:
        return {
            "metric": self.metric_name,
            "role": self.role.value,
            "verdict": self.verdict.value,
            "champion": self.champion_value,
            "candidate": self.candidate_value,
            "delta": self.delta,
            "ci_lower": self.ci_lower,
            "ci_upper": self.ci_upper,
            "p_value": self.p_value,
            "test": self.test,
            "sample_size": self.sample_size,
            "higher_is_better": self.higher_is_better,
            "reason": self.reason,
            "detail": dict(self.detail),
        }


@dataclass
class ComparisonEngine:
    """Runs all per-metric tests between two MetricsBundles."""

    delta_margin_default: float = 0.01
    bootstrap_resamples: int = 10_000
    conf: float = 0.95
    seed: int = 1337

    # ------------------------------------------------------------------ API

    def compare(
        self,
        champion: MetricsBundle,
        candidate: MetricsBundle,
        *,
        margin_overrides: dict[str, float] | None = None,
        budget_caps: dict[str, float] | None = None,
    ) -> list[MetricComparison]:
        """Run comparisons on every metric present in ``candidate``.

        ``margin_overrides`` per-metric δ overrides.
        ``budget_caps`` per-metric hard caps used for BUDGET role metrics.
        """
        out: list[MetricComparison] = []
        overrides = margin_overrides or {}
        caps = budget_caps or {}
        for name, cand_bundle in candidate.metrics.items():
            champ_bundle = champion.metrics.get(name)
            delta_margin = float(
                overrides.get(name, self.delta_margin_default),
            )
            cap = caps.get(name)
            try:
                if cand_bundle.role is MetricRole.BUDGET:
                    out.append(self._budget_cmp(
                        cand_bundle, champ_bundle, cap=cap,
                        champion_raw=champion.raw.get(name),
                        candidate_raw=candidate.raw.get(name),
                    ))
                else:
                    out.append(self._metric_cmp(
                        name=name,
                        champ_bundle=champ_bundle,
                        cand_bundle=cand_bundle,
                        champion_raw=champion.raw.get(name),
                        candidate_raw=candidate.raw.get(name),
                        delta_margin=delta_margin,
                    ))
            except StatisticalTestError as exc:
                out.append(MetricComparison(
                    metric_name=name,
                    role=cand_bundle.role,
                    verdict=MetricComparisonVerdict.UNCOMPARABLE,
                    champion_value=(
                        champ_bundle.value if champ_bundle else None
                    ),
                    candidate_value=cand_bundle.value,
                    delta=None, ci_lower=None, ci_upper=None,
                    p_value=None, test="error",
                    sample_size=cand_bundle.sample_size,
                    higher_is_better=cand_bundle.higher_is_better,
                    reason=str(exc),
                ))
        return out

    # --------------------------------------------------------- branches

    def _budget_cmp(
        self,
        cand: MetricBundle,
        champ: MetricBundle | None,
        *,
        cap: float | None,
        champion_raw: SampleScores | None,
        candidate_raw: SampleScores | None,
    ) -> MetricComparison:
        """Budgets are compared against a hard cap; champion is reference.

        If a cap is provided, breach → BUDGET_EXCEEDED. Otherwise the
        budget is enforced as "candidate no worse than champion" via
        a Welch t-test on the raw samples when available, else falls
        back to aggregate value comparison.
        """
        if cap is not None and cand.value > cap:
            return MetricComparison(
                metric_name=cand.name, role=cand.role,
                verdict=MetricComparisonVerdict.BUDGET_EXCEEDED,
                champion_value=champ.value if champ else None,
                candidate_value=cand.value,
                delta=(
                    cand.value - champ.value
                    if champ is not None else None
                ),
                ci_lower=None, ci_upper=None, p_value=None,
                test="hard_cap",
                sample_size=cand.sample_size,
                higher_is_better=cand.higher_is_better,
                reason=f"exceeds hard cap {cap}",
                detail={"cap": cap},
            )
        # No breach → compare distributions (needed for budget drift).
        if (
            champion_raw is not None and candidate_raw is not None
            and champion_raw.n >= 2 and candidate_raw.n >= 2
        ):
            res = welch_test(
                champion_raw.values, candidate_raw.values,
                alternative="greater",
            )
            if res.p_value < 0.05:
                verdict = MetricComparisonVerdict.FAIL
                reason = "Welch t-test: candidate significantly slower"
            else:
                verdict = MetricComparisonVerdict.PASS
                reason = "within budget"
            return MetricComparison(
                metric_name=cand.name, role=cand.role,
                verdict=verdict,
                champion_value=res.champion_mean,
                candidate_value=res.candidate_mean,
                delta=res.candidate_mean - res.champion_mean,
                ci_lower=None, ci_upper=None, p_value=res.p_value,
                test="welch_t", sample_size=cand.sample_size,
                higher_is_better=cand.higher_is_better,
                reason=reason,
            )
        # No raw samples available — compare aggregated values.
        if champ is None:
            return MetricComparison(
                metric_name=cand.name, role=cand.role,
                verdict=MetricComparisonVerdict.PASS,
                champion_value=None, candidate_value=cand.value,
                delta=None, ci_lower=None, ci_upper=None,
                p_value=None, test="none",
                sample_size=cand.sample_size,
                higher_is_better=cand.higher_is_better,
                reason="no champion baseline; accept if within cap",
            )
        delta = cand.value - champ.value
        # lower-is-better: positive delta is bad
        is_worse = (
            delta > 0 if not cand.higher_is_better else delta < 0
        )
        return MetricComparison(
            metric_name=cand.name, role=cand.role,
            verdict=(
                MetricComparisonVerdict.FAIL
                if is_worse and abs(delta) > 0.05 * abs(champ.value or 1)
                else MetricComparisonVerdict.PASS
            ),
            champion_value=champ.value,
            candidate_value=cand.value,
            delta=delta, ci_lower=None, ci_upper=None,
            p_value=None, test="aggregate",
            sample_size=cand.sample_size,
            higher_is_better=cand.higher_is_better,
            reason="aggregate comparison (no raw samples)",
        )

    def _metric_cmp(
        self,
        *,
        name: str,
        champ_bundle: MetricBundle | None,
        cand_bundle: MetricBundle,
        champion_raw: SampleScores | None,
        candidate_raw: SampleScores | None,
        delta_margin: float,
    ) -> MetricComparison:
        if champ_bundle is None:
            # No champion reference — accept as PASS (first-ever run).
            return MetricComparison(
                metric_name=name, role=cand_bundle.role,
                verdict=MetricComparisonVerdict.PASS,
                champion_value=None, candidate_value=cand_bundle.value,
                delta=None, ci_lower=None, ci_upper=None,
                p_value=None, test="bootstrap_none",
                sample_size=cand_bundle.sample_size,
                higher_is_better=cand_bundle.higher_is_better,
                reason="no prior champion — first run",
            )

        md = resolve_metric(name)
        test: TestName = md.default_test

        # Route per test kind. We prefer TOST-via-bootstrap for
        # non-budget comparisons because it gives a direct equivalence
        # verdict against δ; McNemar is used for classification-style
        # binary correctness to produce a p-value, supplemented by a
        # TOST on the raw 0/1 values.
        if test == "mcnemar":
            return self._cmp_mcnemar(
                name, cand_bundle, champ_bundle,
                candidate_raw, champion_raw, delta_margin,
            )
        if test == "welch_t":
            return self._cmp_welch(
                name, cand_bundle, champ_bundle,
                candidate_raw, champion_raw,
            )
        if test in ("bootstrap", "paired_t"):
            return self._cmp_tost(
                name, cand_bundle, champ_bundle,
                candidate_raw, champion_raw, delta_margin,
            )
        raise StatisticalTestError(f"unsupported test {test!r}")

    # --------------------------------------------------- per-test branches

    def _cmp_tost(
        self, name: str,
        cand: MetricBundle, champ: MetricBundle,
        cand_raw: SampleScores | None, champ_raw: SampleScores | None,
        delta_margin: float,
    ) -> MetricComparison:
        if cand_raw is None or champ_raw is None:
            # Only aggregates — fall back to rough comparison using
            # the bundle's CI, if any.
            delta = cand.value - champ.value
            verdict = self._verdict_from_aggregate(
                delta, delta_margin, cand.higher_is_better,
            )
            return MetricComparison(
                metric_name=name, role=cand.role, verdict=verdict,
                champion_value=champ.value, candidate_value=cand.value,
                delta=delta, ci_lower=None, ci_upper=None,
                p_value=None, test="aggregate",
                sample_size=cand.sample_size,
                higher_is_better=cand.higher_is_better,
                reason="no raw samples; compared aggregates",
            )
        # Paired bootstrap + TOST.
        res = tost_equivalence(
            champion=champ_raw.values, candidate=cand_raw.values,
            delta_margin=delta_margin,
            conf=self.conf, n_resamples=self.bootstrap_resamples,
            seed=self.seed, higher_is_better=cand.higher_is_better,
        )
        verdict_map = {
            "pass": MetricComparisonVerdict.PASS,
            "fail": MetricComparisonVerdict.FAIL,
            "ambiguous": MetricComparisonVerdict.AMBIGUOUS,
            "better": MetricComparisonVerdict.BETTER,
        }
        return MetricComparison(
            metric_name=name, role=cand.role,
            verdict=verdict_map[res.verdict.value],
            champion_value=champ.value, candidate_value=cand.value,
            delta=res.observed_delta,
            ci_lower=res.ci_lower, ci_upper=res.ci_upper,
            p_value=None, test="tost_bootstrap",
            sample_size=res.n_samples,
            higher_is_better=cand.higher_is_better,
            reason=f"TOST δ={delta_margin:.4f}",
        )

    def _cmp_mcnemar(
        self, name: str,
        cand: MetricBundle, champ: MetricBundle,
        cand_raw: SampleScores | None, champ_raw: SampleScores | None,
        delta_margin: float,
    ) -> MetricComparison:
        if cand_raw is None or champ_raw is None:
            return self._cmp_tost(
                name, cand, champ, cand_raw, champ_raw, delta_margin,
            )
        mres = mcnemar_test(
            champion_correct=champ_raw.values,
            candidate_correct=cand_raw.values,
        )
        # Also run TOST on the 0/1 scores for equivalence band.
        tost = tost_equivalence(
            champion=champ_raw.values.astype(np.float64),
            candidate=cand_raw.values.astype(np.float64),
            delta_margin=delta_margin,
            conf=self.conf, n_resamples=self.bootstrap_resamples,
            seed=self.seed, higher_is_better=cand.higher_is_better,
        )
        # Combine: TOST verdict is the primary; p-value from McNemar
        # is reported for transparency.
        verdict_map = {
            "pass": MetricComparisonVerdict.PASS,
            "fail": MetricComparisonVerdict.FAIL,
            "ambiguous": MetricComparisonVerdict.AMBIGUOUS,
            "better": MetricComparisonVerdict.BETTER,
        }
        return MetricComparison(
            metric_name=name, role=cand.role,
            verdict=verdict_map[tost.verdict.value],
            champion_value=champ.value, candidate_value=cand.value,
            delta=tost.observed_delta,
            ci_lower=tost.ci_lower, ci_upper=tost.ci_upper,
            p_value=mres.p_value,
            test=f"mcnemar+tost(δ={delta_margin:.4f})",
            sample_size=tost.n_samples,
            higher_is_better=cand.higher_is_better,
            reason="McNemar paired binary + bootstrap TOST",
            detail={"b": mres.b, "c": mres.c, "mcnemar_test": mres.test},
        )

    def _cmp_welch(
        self, name: str,
        cand: MetricBundle, champ: MetricBundle,
        cand_raw: SampleScores | None, champ_raw: SampleScores | None,
    ) -> MetricComparison:
        if (
            cand_raw is None or champ_raw is None
            or cand_raw.n < 2 or champ_raw.n < 2
        ):
            # Aggregate fallback.
            delta = cand.value - champ.value
            worse = (
                delta > 0 if not cand.higher_is_better else delta < 0
            )
            return MetricComparison(
                metric_name=name, role=cand.role,
                verdict=(
                    MetricComparisonVerdict.FAIL if worse
                    else MetricComparisonVerdict.PASS
                ),
                champion_value=champ.value, candidate_value=cand.value,
                delta=delta, ci_lower=None, ci_upper=None,
                p_value=None, test="aggregate_welch_fallback",
                sample_size=cand.sample_size,
                higher_is_better=cand.higher_is_better,
                reason="insufficient raw samples for Welch t",
            )
        alt = "greater" if not cand.higher_is_better else "less"
        res = welch_test(
            champion=champ_raw.values, candidate=cand_raw.values,
            alternative=alt,  # type: ignore[arg-type]
        )
        verdict = (
            MetricComparisonVerdict.FAIL if res.p_value < 0.05
            else MetricComparisonVerdict.PASS
        )
        return MetricComparison(
            metric_name=name, role=cand.role, verdict=verdict,
            champion_value=res.champion_mean,
            candidate_value=res.candidate_mean,
            delta=res.candidate_mean - res.champion_mean,
            ci_lower=None, ci_upper=None,
            p_value=res.p_value,
            test="welch_t", sample_size=cand.sample_size,
            higher_is_better=cand.higher_is_better,
            reason=f"Welch t (alt={alt})",
        )

    # --------------------------------------------------------- helpers

    @staticmethod
    def _verdict_from_aggregate(
        delta: float, delta_margin: float, higher_is_better: bool,
    ) -> MetricComparisonVerdict:
        # With only aggregates (no CI), we make a conservative call:
        # within ±δ = PASS; outside on the worse side = FAIL; on the
        # better side = BETTER. AMBIGUOUS is impossible without CI.
        d = delta if higher_is_better else -delta
        if abs(d) <= delta_margin:
            return MetricComparisonVerdict.PASS
        if d > 0:
            return MetricComparisonVerdict.BETTER
        return MetricComparisonVerdict.FAIL


def paired_bootstrap_fallback(
    champion: SampleScores, candidate: SampleScores,
    *, conf: float = 0.95, n_resamples: int = 10_000, seed: int = 1337,
) -> tuple[float, float, float]:
    """Convenience: (mean_delta, ci_lower, ci_upper)."""
    res = bootstrap_delta_ci(
        champion.values, candidate.values,
        n_resamples=n_resamples, conf=conf, seed=seed,
    )
    return res.mean, res.ci_lower, res.ci_upper


__all__ = [
    "ComparisonEngine",
    "MetricComparison",
    "MetricComparisonVerdict",
    "paired_bootstrap_fallback",
]

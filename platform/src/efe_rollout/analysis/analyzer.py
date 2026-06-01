"""End-to-end analysis pipeline.

Given a spec, a list of exposures, and a list of outcomes, the
:class:`ExperimentAnalyzer` :

1. Aligns exposures ↔ outcomes (ITT or PP).
2. Optionally applies CUPED variance reduction using a caller-declared
   covariate key.
3. Runs either a Welch t-test (continuous metric) or a Wald proportion
   test (binary metric auto-detected by value range / metric-shape).
4. Runs a Bayesian beta-binomial posterior (binary metrics only).
5. Optionally augments with cluster-robust SE when every exposure is
   tied to a user-level cluster.
6. Maps the outputs onto a ``decision`` recommendation.

Returns an immutable :class:`AnalysisResult` which the service layer
persists to the ``analyses`` table and publishes as a NATS
``efe.rollout.analyzed`` event.
"""

from __future__ import annotations

import math
from collections.abc import Iterable
from dataclasses import dataclass
from typing import Literal

from efe_rollout.analysis.bayesian import (
    BayesianPosterior,
    bayesian_beta_binomial,
    bayesian_decision,
)
from efe_rollout.analysis.cluster import cluster_robust_se
from efe_rollout.analysis.cuped import apply_cuped
from efe_rollout.analysis.stats import (
    MeanTestResult,
    PropTestResult,
    wald_prop_test,
    welch_t_test,
)
from efe_rollout.clock import Clock, SystemClock
from efe_rollout.errors import InsufficientData
from efe_rollout.experiment import ExperimentSpec, Variant
from efe_rollout.ids import new_uuid
from efe_rollout.records import AnalysisResult, Exposure, Outcome

AnalysisKind = Literal["interim", "final", "post_hoc", "segment", "cuped"]


@dataclass(frozen=True, slots=True)
class AnalysisInputs:
    spec: ExperimentSpec
    exposures: tuple[Exposure, ...]
    outcomes: tuple[Outcome, ...]
    primary_metric: str
    kind: AnalysisKind = "final"
    apply_cuped_covariate: str | None = None
    apply_cluster_robust: bool = False
    bayesian_ship_threshold: float = 0.95
    bayesian_rollback_threshold: float = 0.05
    bayesian_n_samples: int = 20_000
    bayesian_seed: int = 20260420
    alpha: float = 0.05


class ExperimentAnalyzer:
    """Composes all the primitives above into one analysis run."""

    __slots__ = ("_clock",)

    def __init__(self, *, clock: Clock | None = None) -> None:
        self._clock = clock or SystemClock()

    def analyze(self, inputs: AnalysisInputs) -> AnalysisResult:
        control, treatment = self._resolve_two_arms(inputs.spec)
        ctrl_out, trt_out = self._split(
            inputs.outcomes,
            inputs.primary_metric,
            control.variant_id,
            treatment.variant_id,
        )
        if len(ctrl_out) < 2 or len(trt_out) < 2:
            raise InsufficientData(
                f"analysis requires ≥2 outcomes per arm, got "
                f"control={len(ctrl_out)}, treatment={len(trt_out)}",
            )
        is_binary = _looks_binary(ctrl_out, trt_out)

        methods: dict[str, object] = {
            "primary_metric": inputs.primary_metric,
            "is_binary": is_binary,
            "kind": inputs.kind,
        }

        ci_95: tuple[float, float] | None = None
        estimate = 0.0
        p_value: float | None = None
        posterior: BayesianPosterior | None = None

        if is_binary:
            s_a = sum(1 for v in ctrl_out if v > 0.5)
            s_b = sum(1 for v in trt_out if v > 0.5)
            prop_result: PropTestResult = wald_prop_test(
                s_a, len(ctrl_out), s_b, len(trt_out),
                alpha=inputs.alpha,
            )
            estimate = prop_result.estimate
            ci_95 = prop_result.ci_95
            p_value = prop_result.p_value
            methods["method"] = "wald_prop_test"
            posterior = bayesian_beta_binomial(
                s_a=s_a, n_a=len(ctrl_out),
                s_b=s_b, n_b=len(trt_out),
                n_samples=inputs.bayesian_n_samples,
                seed=inputs.bayesian_seed,
            )
        else:
            ctrl_vals = list(ctrl_out)
            trt_vals = list(trt_out)
            if inputs.apply_cuped_covariate:
                ctrl_vals = self._with_cuped(
                    ctrl_out, inputs.outcomes,
                    inputs.apply_cuped_covariate, control.variant_id,
                    inputs.primary_metric,
                )
                trt_vals = self._with_cuped(
                    trt_out, inputs.outcomes,
                    inputs.apply_cuped_covariate, treatment.variant_id,
                    inputs.primary_metric,
                )
                methods["cuped_applied"] = True
                methods["cuped_covariate"] = inputs.apply_cuped_covariate
            mean_result: MeanTestResult = welch_t_test(
                ctrl_vals, trt_vals, alpha=inputs.alpha,
            )
            estimate = mean_result.estimate
            ci_95 = mean_result.ci_95
            p_value = mean_result.p_value
            methods["method"] = "welch_t_test"

            if inputs.apply_cluster_robust:
                y = list(ctrl_vals) + list(trt_vals)
                d = [0] * len(ctrl_vals) + [1] * len(trt_vals)
                # Cluster on user_id. The values in ``ctrl_vals`` /
                # ``trt_vals`` are in the same order as the outcomes
                # were iterated by :meth:`_split` (and possibly by
                # :meth:`_with_cuped`, which preserves order). So we
                # extract user ids in one O(n) pass each, matching
                # the insertion order of the split lists.
                cluster_keys = (
                    self._user_ids_for_arm(
                        inputs.outcomes,
                        control.variant_id,
                        inputs.primary_metric,
                        n=len(ctrl_vals),
                    )
                    + self._user_ids_for_arm(
                        inputs.outcomes,
                        treatment.variant_id,
                        inputs.primary_metric,
                        n=len(trt_vals),
                    )
                )
                cr = cluster_robust_se(y=y, d=d, clusters=cluster_keys)
                methods["cluster_robust_se"] = cr.se
                methods["cluster_robust_beta"] = cr.beta
                methods["cluster_robust_t"] = cr.t_stat
                methods["n_clusters"] = cr.n_clusters

        decision = self._decide(
            p_value=p_value,
            alpha=inputs.alpha,
            estimate=estimate,
            posterior=posterior,
            ship_threshold=inputs.bayesian_ship_threshold,
            rollback_threshold=inputs.bayesian_rollback_threshold,
        )

        return AnalysisResult(
            analysis_id=new_uuid(),
            experiment_id=self._exp_id(inputs.exposures),
            kind=inputs.kind,
            computed_at=self._clock.now(),
            n_per_variant={
                control.name: len(ctrl_out),
                treatment.name: len(trt_out),
            },
            primary_metric=inputs.primary_metric,
            estimate=estimate,
            ci_95=ci_95,
            p_value=p_value,
            posterior_prob_better=(
                posterior.prob_b_gt_a if posterior else None
            ),
            expected_lift_pct=(
                100.0 * posterior.expected_lift if posterior else None
            ),
            credible_interval_95=(
                posterior.credible_interval_95 if posterior else None
            ),
            decision=decision,
            methods=methods,
            segments={},
        )

    # ------------------------------------------------------------------
    # helpers
    # ------------------------------------------------------------------

    def _resolve_two_arms(
        self, spec: ExperimentSpec,
    ) -> tuple[Variant, Variant]:
        if len(spec.variants) < 2:
            raise InsufficientData(
                f"analysis requires ≥2 variants, got {len(spec.variants)}",
            )
        control = spec.control()
        if control is None:
            # fall back to alphabetical-first variant as control
            control = spec.variants_sorted()[0]
        treatment = next(
            (v for v in spec.variants_sorted() if v is not control),
            None,
        )
        assert treatment is not None
        return control, treatment

    def _split(
        self,
        outcomes: Iterable[Outcome],
        metric: str,
        control_vid: str | None,
        treatment_vid: str | None,
    ) -> tuple[list[float], list[float]]:
        control_vals: list[float] = []
        treatment_vals: list[float] = []
        for o in outcomes:
            if o.metric_name != metric:
                continue
            if o.variant_id == control_vid:
                control_vals.append(o.metric_value)
            elif o.variant_id == treatment_vid:
                treatment_vals.append(o.metric_value)
        return control_vals, treatment_vals

    def _with_cuped(
        self,
        y_values: list[float],
        outcomes: Iterable[Outcome],
        covariate_key: str,
        variant_id: str | None,
        metric: str,
    ) -> list[float]:
        pairs: list[tuple[float, float]] = []
        for o in outcomes:
            if o.variant_id != variant_id or o.metric_name != metric:
                continue
            cov = o.covariate.get(covariate_key)
            if isinstance(cov, (int, float)) and not math.isnan(float(cov)):
                pairs.append((o.metric_value, float(cov)))
        if len(pairs) < 2:
            return y_values
        y = [p[0] for p in pairs]
        x = [p[1] for p in pairs]
        adj = apply_cuped(y, x)
        return list(adj.y_adj)

    def _exp_id(self, exposures: Iterable[Exposure]) -> str:
        for e in exposures:
            return e.experiment_id
        return "unknown"

    def _user_ids_for_arm(
        self,
        outcomes: Iterable[Outcome],
        variant_id: str | None,
        metric: str,
        *,
        n: int,
    ) -> list[str]:
        """One-pass, order-preserving collection of user ids for an arm.

        Replaces the earlier O(n²) ``_user_for_value`` helper. Callers
        request exactly ``n`` ids — if fewer outcomes match the arm
        (which should never happen given :meth:`_split` produced ``n``
        values from the same iterable in the same order) we pad with a
        synthetic marker so :func:`cluster_robust_se` doesn't choke.
        """
        collected: list[str] = []
        for o in outcomes:
            if o.variant_id == variant_id and o.metric_name == metric:
                collected.append(o.user_id)
                if len(collected) == n:
                    return collected
        # Pad defensively — should be unreachable.
        while len(collected) < n:
            collected.append(f"__missing__:{len(collected)}")
        return collected

    def _decide(
        self,
        *,
        p_value: float | None,
        alpha: float,
        estimate: float,
        posterior: BayesianPosterior | None,
        ship_threshold: float,
        rollback_threshold: float,
    ) -> Literal["ship", "rollback", "inconclusive", "continue"] | None:
        if posterior is not None:
            return bayesian_decision(  # type: ignore[return-value]
                posterior,
                ship_threshold=ship_threshold,
                rollback_threshold=rollback_threshold,
            )
        if p_value is None or math.isnan(p_value):
            return "inconclusive"
        if p_value <= alpha and estimate > 0:
            return "ship"
        if p_value <= alpha and estimate < 0:
            return "rollback"
        return "inconclusive"


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _looks_binary(
    ctrl: list[float], trt: list[float],
) -> bool:
    """Heuristic: if every observation is in {0, 1}, treat as binary."""
    return all(v in (0.0, 1.0) for v in ctrl + trt)


__all__ = ["AnalysisInputs", "AnalysisKind", "ExperimentAnalyzer"]

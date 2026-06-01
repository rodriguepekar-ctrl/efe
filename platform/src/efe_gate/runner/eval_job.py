"""Eval-job runner.

The runner is the stage that actually *runs inference* for the
candidate (and, when necessary, re-runs the champion). It is
intentionally decoupled from the rest of the gate so tests can
inject a :class:`FakeScorer` that returns pre-baked per-sample
scores.

In production the runner is a K8s Job that:

    1. downloads candidate (+ optionally champion) artifacts,
    2. loads them (reusing :mod:`efe_worker.methods`),
    3. runs inference on the locked golden set,
    4. computes per-metric per-sample scores,
    5. persists them as :class:`MetricsBundle`.

For the MVP we ship the *scoring pipeline* interface and a fake
scorer; the actual model-loading path is a phase-2 integration
because it requires torch + the concrete method registry.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol

import numpy as np

from efe_gate.errors import InsufficientSample, RunnerError
from efe_gate.metrics.bundle import (
    MetricBundle,
    MetricRole,
    MetricsBundle,
    SampleScores,
)
from efe_gate.metrics.catalog import resolve_metric
from efe_gate.metrics.stats.bootstrap import bootstrap_ci


@dataclass(frozen=True, slots=True)
class EvalInputs:
    """Input bundle passed to the runner."""

    tenant_id: str
    task_id: str
    task_type: str  # "classif" | "retrieval" | "distill"
    candidate_uri: str
    champion_uri: str | None
    golden_snapshot_uri: str
    metric_names: tuple[str, ...]


class EvalScorer(Protocol):
    """Produces per-sample scores for a (model, metric) pair."""

    async def score(
        self, *, artifact_uri: str, metric: str,
        golden_snapshot_uri: str,
    ) -> np.ndarray:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# FakeScorer — tests
# ---------------------------------------------------------------------------


@dataclass
class FakeScorer:
    """In-memory scorer.

    Callers pre-populate ``scores[(artifact_uri, metric)] = np.array(...)``;
    absent keys yield an empty array which the runner treats as an
    error.
    """

    scores: dict[tuple[str, str], np.ndarray] = field(default_factory=dict)
    groups: dict[tuple[str, str], np.ndarray] = field(default_factory=dict)

    async def score(
        self, *, artifact_uri: str, metric: str,
        golden_snapshot_uri: str,
    ) -> np.ndarray:
        _ = golden_snapshot_uri
        arr = self.scores.get((artifact_uri, metric))
        if arr is None:
            raise RunnerError(
                f"fake scorer has no data for ({artifact_uri!r}, "
                f"{metric!r})",
            )
        return arr


# ---------------------------------------------------------------------------
# run_evaluation
# ---------------------------------------------------------------------------


async def run_evaluation(
    inputs: EvalInputs,
    *,
    scorer: EvalScorer,
    groups_fetcher: (
        dict[str, np.ndarray] | None
    ) = None,
    bootstrap_resamples: int = 10_000,
    conf: float = 0.95,
    seed: int = 1337,
) -> tuple[MetricsBundle, MetricsBundle | None]:
    """Compute a :class:`MetricsBundle` for candidate and champion.

    Returns ``(candidate_bundle, champion_bundle_or_None)``.
    ``champion_bundle`` is only computed when ``inputs.champion_uri``
    is provided.

    ``groups_fetcher`` maps metric_name → per-sample group array (for
    fairness splits). When not provided, no by-group splits are
    computed.
    """
    cand = await _score_all(
        inputs, inputs.candidate_uri, scorer=scorer,
        groups_fetcher=groups_fetcher,
        bootstrap_resamples=bootstrap_resamples, conf=conf, seed=seed,
    )
    champ: MetricsBundle | None = None
    if inputs.champion_uri:
        champ = await _score_all(
            inputs, inputs.champion_uri, scorer=scorer,
            groups_fetcher=groups_fetcher,
            bootstrap_resamples=bootstrap_resamples, conf=conf, seed=seed,
        )
    return cand, champ


async def _score_all(
    inputs: EvalInputs, artifact_uri: str,
    *,
    scorer: EvalScorer,
    groups_fetcher: dict[str, np.ndarray] | None,
    bootstrap_resamples: int,
    conf: float,
    seed: int,
) -> MetricsBundle:
    bundle = MetricsBundle()
    for metric in inputs.metric_names:
        md = resolve_metric(metric)
        scores = await scorer.score(
            artifact_uri=artifact_uri, metric=metric,
            golden_snapshot_uri=inputs.golden_snapshot_uri,
        )
        if scores.size < 1:
            raise InsufficientSample(
                f"scorer returned empty array for {metric}",
            )
        # Raw sample scores (needed downstream for McNemar / bootstrap).
        groups = None
        if groups_fetcher and metric in groups_fetcher:
            groups = groups_fetcher[metric]
            if groups.shape[0] != scores.shape[0]:
                raise RunnerError(
                    f"groups length mismatch for {metric}",
                )
        sample = SampleScores(
            metric_name=metric, values=scores, groups=groups,
        )
        bundle.add_raw(sample)
        # Aggregate value + bootstrap CI.
        res = bootstrap_ci(
            scores, n_resamples=bootstrap_resamples,
            conf=conf, seed=seed,
        )
        bundle.add(MetricBundle(
            name=metric, role=md.role,
            value=res.mean,
            ci_lower=res.ci_lower,
            ci_upper=res.ci_upper,
            sample_size=res.n_samples,
            higher_is_better=md.higher_is_better,
        ))
        # Per-group splits for fairness.
        if groups is not None:
            for g in np.unique(groups):
                mask = groups == g
                sub = scores[mask]
                if sub.size < 1:
                    continue
                sub_res = bootstrap_ci(
                    sub,
                    n_resamples=min(bootstrap_resamples, 2_000),
                    conf=conf, seed=seed,
                )
                bundle.add_group(str(g), MetricBundle(
                    name=metric, role=md.role,
                    value=sub_res.mean,
                    ci_lower=sub_res.ci_lower,
                    ci_upper=sub_res.ci_upper,
                    sample_size=sub_res.n_samples,
                    higher_is_better=md.higher_is_better,
                ))
    return bundle


def _derive_role(_name: str) -> MetricRole:
    return MetricRole.PRIMARY  # unused — kept to silence import


_ = _derive_role


__all__ = [
    "EvalInputs",
    "EvalScorer",
    "FakeScorer",
    "run_evaluation",
]


# typing-silencer for unused Any at module scope
_ANY_GUARD: Any = None
_ = _ANY_GUARD

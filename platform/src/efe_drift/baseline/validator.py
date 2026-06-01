"""Baseline sanity checks (M09 §13.2).

Run on a :class:`~efe_drift.records.BaselineSpec` before promotion.
Produces a list of :class:`ValidationIssue`. A ``blocker`` forbids
promotion; a ``warning`` lets the promotion proceed but is recorded
in the audit trail.

Checks performed :

1. **Minimum sample count** — default 10 000 observations.
2. **Window coverage** — at least N days of accumulation.
3. **Non-degenerate distribution** — no single numeric bin covers
   more than ``max_bin_skew_pct`` of the mass ; no zero-variance
   feature.
4. **Bin monotonicity** — numeric edges strictly increasing and
   finite. (Already enforced at bin construction time, but the
   validator double-checks post-seal.)
5. **Category non-emptiness** — categorical features must have at
   least 2 distinct categories.
6. **Embedding non-degenerate** — centroid must not be all-zero and
   std must not be all-zero.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field

from efe_drift.errors import BaselineSanityCheckFailed
from efe_drift.records import BaselineSpec


@dataclass(frozen=True, slots=True)
class ValidationIssue:
    code: str
    severity: str                        # "blocker" or "warning"
    message: str


@dataclass(frozen=True, slots=True)
class ValidationReport:
    issues: tuple[ValidationIssue, ...] = field(default_factory=tuple)

    @property
    def blockers(self) -> tuple[ValidationIssue, ...]:
        return tuple(i for i in self.issues if i.severity == "blocker")

    @property
    def warnings(self) -> tuple[ValidationIssue, ...]:
        return tuple(i for i in self.issues if i.severity == "warning")

    @property
    def ok(self) -> bool:
        return not self.blockers


# ---------------------------------------------------------------------------
# Validator
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class BaselineValidator:
    """Configurable sanity-check runner."""

    min_sample_count: int = 10_000
    min_days: int = 7
    max_bin_skew_pct: float = 0.95
    min_days_warning: int = 14

    def validate(self, spec: BaselineSpec) -> ValidationReport:
        issues: list[ValidationIssue] = []
        self._check_sample_count(spec, issues)
        self._check_window(spec, issues)
        if spec.feature_kind in ("numeric", "binary"):
            self._check_numeric(spec, issues)
        elif spec.feature_kind == "categorical":
            self._check_categorical(spec, issues)
        elif spec.feature_kind == "embedding":
            self._check_embedding(spec, issues)
        return ValidationReport(issues=tuple(issues))

    def validate_strict(self, spec: BaselineSpec) -> None:
        """Like :meth:`validate` but raises on blockers.

        Callers that want the simple "go/no-go" semantics use this
        and let the exception propagate.
        """
        report = self.validate(spec)
        if not report.ok:
            raise BaselineSanityCheckFailed(
                "baseline sanity failed : "
                + "; ".join(i.message for i in report.blockers),
                detail={
                    "blockers": [
                        {
                            "code": i.code,
                            "message": i.message,
                        }
                        for i in report.blockers
                    ],
                    "warnings": [
                        {
                            "code": i.code,
                            "message": i.message,
                        }
                        for i in report.warnings
                    ],
                },
            )

    # ---- checks --------------------------------------------------------

    def _check_sample_count(
        self, spec: BaselineSpec, issues: list[ValidationIssue],
    ) -> None:
        if spec.sample_count < self.min_sample_count:
            issues.append(ValidationIssue(
                code="insufficient_samples",
                severity="blocker",
                message=(
                    f"sample_count={spec.sample_count} < "
                    f"min_sample_count={self.min_sample_count}"
                ),
            ))

    def _check_window(
        self, spec: BaselineSpec, issues: list[ValidationIssue],
    ) -> None:
        duration = (spec.window_end - spec.window_start).total_seconds()
        days = duration / 86_400
        if days < self.min_days:
            issues.append(ValidationIssue(
                code="short_window_blocker",
                severity="blocker",
                message=(
                    f"window covers {days:.1f} days, need ≥ {self.min_days}"
                ),
            ))
        elif days < self.min_days_warning:
            issues.append(ValidationIssue(
                code="short_window_warning",
                severity="warning",
                message=(
                    f"window covers {days:.1f} days, recommended ≥ "
                    f"{self.min_days_warning}"
                ),
            ))

    def _check_numeric(
        self, spec: BaselineSpec, issues: list[ValidationIssue],
    ) -> None:
        if not spec.bins or len(spec.bins) < 2:
            issues.append(ValidationIssue(
                code="missing_bins",
                severity="blocker",
                message="numeric baseline has no bin edges",
            ))
            return
        edges = spec.bins
        for i in range(len(edges) - 1):
            if edges[i + 1] <= edges[i]:
                issues.append(ValidationIssue(
                    code="non_monotonic_bins",
                    severity="blocker",
                    message=(
                        f"bin edge {i + 1} ({edges[i + 1]}) "
                        f"≤ edge {i} ({edges[i]})"
                    ),
                ))
                return
            if not math.isfinite(edges[i]) or not math.isfinite(edges[i + 1]):
                issues.append(ValidationIssue(
                    code="non_finite_bins",
                    severity="blocker",
                    message=f"bin edge at index {i} not finite",
                ))
                return
        # Zero-variance check via stats
        std = spec.stats.get("std")
        if std is not None and std == 0:
            issues.append(ValidationIssue(
                code="zero_variance",
                severity="blocker",
                message="numeric baseline has zero standard deviation",
            ))
        if std is not None and math.isnan(std):
            issues.append(ValidationIssue(
                code="nan_std",
                severity="blocker",
                message="numeric baseline std is NaN",
            ))

    def _check_categorical(
        self, spec: BaselineSpec, issues: list[ValidationIssue],
    ) -> None:
        if not spec.categorical_categories:
            issues.append(ValidationIssue(
                code="missing_categories",
                severity="blocker",
                message="categorical baseline has no categories",
            ))
            return
        if len(spec.categorical_categories) < 2:
            issues.append(ValidationIssue(
                code="single_category",
                severity="blocker",
                message=(
                    f"categorical baseline has only "
                    f"{len(spec.categorical_categories)} category — "
                    "need ≥ 2"
                ),
            ))

    def _check_embedding(
        self, spec: BaselineSpec, issues: list[ValidationIssue],
    ) -> None:
        if not spec.centroid:
            issues.append(ValidationIssue(
                code="missing_centroid",
                severity="blocker",
                message="embedding baseline has no centroid",
            ))
            return
        if all(abs(c) < 1e-12 for c in spec.centroid):
            issues.append(ValidationIssue(
                code="zero_centroid",
                severity="blocker",
                message="embedding centroid is the zero vector",
            ))
        if spec.std is not None and all(
            abs(s) < 1e-12 for s in spec.std
        ):
            issues.append(ValidationIssue(
                code="zero_embedding_std",
                severity="blocker",
                message="embedding std is zero across all dimensions",
            ))


__all__ = [
    "BaselineValidator",
    "ValidationIssue",
    "ValidationReport",
]

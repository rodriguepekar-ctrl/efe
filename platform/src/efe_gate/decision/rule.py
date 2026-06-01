"""Promotion rule — data class + YAML/JSON loader.

A rule captures *what* the gate enforces for a given
(``tenant_id``, ``task_id``) pair. Stored as JSON in
``promotion_gates.rule`` (see :mod:`efe_gate.db.models`).

Shape (see M05 architecture doc §11.2):

    primary:
      - metric: accuracy
        test: mcnemar          # optional override
        min_equivalence_margin: 0.005
    budget:
      - metric: p99_latency_ms
        hard_cap: 50
    fairness:
      groups: [language]
      metric: macro_f1
      max_per_group_drop: 0.02
      min_samples_per_group: 200
    review_policy:
      require_dual_signoff: true
      reviewers: [ml-lead@acme]
      timeout_hours: 24
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from efe_gate.errors import RuleParseError


@dataclass(frozen=True, slots=True)
class MetricRuleSpec:
    metric: str
    min_equivalence_margin: float | None = None
    test: str | None = None
    hard_cap: float | None = None


@dataclass(frozen=True, slots=True)
class FairnessRuleSpec:
    groups: tuple[str, ...] = ()
    metric: str = "macro_f1"
    max_per_group_drop: float = 0.02
    min_samples_per_group: int = 200


@dataclass(frozen=True, slots=True)
class ReviewPolicy:
    require_dual_signoff: bool = False
    reviewers: tuple[str, ...] = ()
    timeout_hours: int = 24


@dataclass(frozen=True, slots=True)
class PromotionRule:
    primary: tuple[MetricRuleSpec, ...] = ()
    budget: tuple[MetricRuleSpec, ...] = ()
    fairness: FairnessRuleSpec | None = None
    review_policy: ReviewPolicy = field(default_factory=ReviewPolicy)
    secondary: tuple[MetricRuleSpec, ...] = ()

    # ----- helpers -----

    def primary_metrics(self) -> list[str]:
        return [m.metric for m in self.primary]

    def budget_metrics(self) -> list[str]:
        return [m.metric for m in self.budget]

    def margin_for(self, metric: str) -> float | None:
        for m in self.primary:
            if m.metric == metric and m.min_equivalence_margin is not None:
                return m.min_equivalence_margin
        return None

    def cap_for(self, metric: str) -> float | None:
        for m in self.budget:
            if m.metric == metric and m.hard_cap is not None:
                return m.hard_cap
        return None

    def as_dict(self) -> dict[str, Any]:
        return {
            "primary": [
                {
                    "metric": m.metric,
                    **({"min_equivalence_margin": m.min_equivalence_margin}
                       if m.min_equivalence_margin is not None else {}),
                    **({"test": m.test} if m.test else {}),
                }
                for m in self.primary
            ],
            "secondary": [
                {"metric": m.metric} for m in self.secondary
            ],
            "budget": [
                {
                    "metric": m.metric,
                    **({"hard_cap": m.hard_cap}
                       if m.hard_cap is not None else {}),
                }
                for m in self.budget
            ],
            "fairness": (
                None if self.fairness is None else {
                    "groups": list(self.fairness.groups),
                    "metric": self.fairness.metric,
                    "max_per_group_drop": (
                        self.fairness.max_per_group_drop
                    ),
                    "min_samples_per_group": (
                        self.fairness.min_samples_per_group
                    ),
                }
            ),
            "review_policy": {
                "require_dual_signoff": (
                    self.review_policy.require_dual_signoff
                ),
                "reviewers": list(self.review_policy.reviewers),
                "timeout_hours": self.review_policy.timeout_hours,
            },
        }

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> PromotionRule:
        try:
            primary = tuple(
                MetricRuleSpec(
                    metric=str(m["metric"]),
                    min_equivalence_margin=_opt_float(
                        m.get("min_equivalence_margin"),
                    ),
                    test=m.get("test"),
                )
                for m in (raw.get("primary") or [])
            )
            secondary = tuple(
                MetricRuleSpec(metric=str(m["metric"]))
                for m in (raw.get("secondary") or [])
            )
            budget = tuple(
                MetricRuleSpec(
                    metric=str(m["metric"]),
                    hard_cap=_opt_float(m.get("hard_cap")),
                )
                for m in (raw.get("budget") or [])
            )
            fairness_raw = raw.get("fairness")
            if fairness_raw is None:
                fairness = None
            else:
                groups_list = list(fairness_raw.get("groups", []))
                fairness = FairnessRuleSpec(
                    groups=tuple(str(g) for g in groups_list),
                    metric=str(fairness_raw.get("metric", "macro_f1")),
                    max_per_group_drop=float(
                        fairness_raw.get("max_per_group_drop", 0.02),
                    ),
                    min_samples_per_group=int(
                        fairness_raw.get("min_samples_per_group", 200),
                    ),
                )
            rp_raw = raw.get("review_policy") or {}
            rp = ReviewPolicy(
                require_dual_signoff=bool(
                    rp_raw.get("require_dual_signoff", False),
                ),
                reviewers=tuple(
                    str(r) for r in rp_raw.get("reviewers", [])
                ),
                timeout_hours=int(rp_raw.get("timeout_hours", 24)),
            )
        except (KeyError, TypeError, ValueError) as exc:
            raise RuleParseError(f"malformed rule: {exc}") from exc
        return cls(
            primary=primary,
            secondary=secondary,
            budget=budget,
            fairness=fairness,
            review_policy=rp,
        )


def _opt_float(v: Any) -> float | None:
    return None if v is None else float(v)


__all__ = [
    "FairnessRuleSpec",
    "MetricRuleSpec",
    "PromotionRule",
    "ReviewPolicy",
]

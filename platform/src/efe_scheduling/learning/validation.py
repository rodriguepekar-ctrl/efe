"""Validation decision.

Given a report of evaluation metrics and a set of per-metric thresholds,
classify the artefact as ``ready | reject``.
"""

from __future__ import annotations

from dataclasses import dataclass, field

__all__ = ["ValidationReport", "ValidationThresholds", "validate_report"]


@dataclass(frozen=True, slots=True)
class ValidationThresholds:
    perplexity_max: float = 100.0
    rouge_l_min: float = 0.25
    bleu_min: float = 0.15
    exact_match_min: float = 0.20
    extra: dict[str, tuple[str, float]] = field(default_factory=dict)
    """Optional extra checks : ``metric_name → (op, value)``, op in
    ``{'<', '<=', '>', '>='}``."""


@dataclass(frozen=True, slots=True)
class ValidationReport:
    metrics: dict[str, float]
    ready: bool = False
    reasons: tuple[str, ...] = ()

    def get(self, name: str, default: float = 0.0) -> float:
        return self.metrics.get(name, default)


def _cmp(op: str, v: float, t: float) -> bool:
    match op:
        case "<":
            return v < t
        case "<=":
            return v <= t
        case ">":
            return v > t
        case ">=":
            return v >= t
        case _:
            raise ValueError(f"unsupported op {op}")


def validate_report(
    metrics: dict[str, float], thresholds: ValidationThresholds
) -> ValidationReport:
    reasons: list[str] = []

    pp = metrics.get("perplexity", float("inf"))
    if pp > thresholds.perplexity_max:
        reasons.append(f"perplexity {pp:.2f} > {thresholds.perplexity_max:.2f}")
    rouge = metrics.get("rouge_l", 0.0)
    if rouge < thresholds.rouge_l_min:
        reasons.append(f"rouge_l {rouge:.3f} < {thresholds.rouge_l_min:.3f}")
    bleu = metrics.get("bleu", 0.0)
    if bleu < thresholds.bleu_min:
        reasons.append(f"bleu {bleu:.3f} < {thresholds.bleu_min:.3f}")
    em = metrics.get("exact_match", 0.0)
    if em < thresholds.exact_match_min:
        reasons.append(f"exact_match {em:.3f} < {thresholds.exact_match_min:.3f}")

    for name, (op, value) in thresholds.extra.items():
        v = metrics.get(name)
        if v is None:
            reasons.append(f"missing metric {name}")
            continue
        if not _cmp(op, v, value):
            reasons.append(f"{name} {v:.3f} {op} {value:.3f} failed")

    return ValidationReport(
        metrics=dict(metrics),
        ready=not reasons,
        reasons=tuple(reasons),
    )

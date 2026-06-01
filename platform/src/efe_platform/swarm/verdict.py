"""Interpretation of :class:`Verdict` values.

The raw verdict enum is machine-speak. The bot and audit layer need a
*human-readable interpretation* with recommended actions. This module
turns each verdict into a :class:`VerdictReport` — a small structured
value describing:

- **label** — the verdict enum,
- **confident** — boolean: should the caller trust the output?
- **severity** — ``"ok"`` / ``"warn"`` / ``"error"``,
- **recommended_action** — human-readable imperative,
- **retryable** — whether a retry with the same spec could plausibly
  produce a different outcome.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from efe_platform.swarm.protocol import ExecuteResponse, Verdict

Severity = Literal["ok", "warn", "error"]


@dataclass(frozen=True, slots=True)
class VerdictReport:
    """Structured interpretation of a :class:`Verdict`."""

    label: Verdict
    confident: bool
    severity: Severity
    recommended_action: str
    retryable: bool

    def to_dict(self) -> dict[str, object]:
        return {
            "label": self.label.name,
            "confident": self.confident,
            "severity": self.severity,
            "recommended_action": self.recommended_action,
            "retryable": self.retryable,
        }


# ---------------------------------------------------------------------------
#  Interpretation tables
# ---------------------------------------------------------------------------


#: The minimal confidence under which a CONVERGED verdict is downgraded
#: to ``warn`` severity. Mirrors the Rust-side ``confidence_floor`` in
#: the arbiter config. Tunable via config override.
DEFAULT_CONFIDENCE_FLOOR = 0.55


def analyze_verdict(
    response: ExecuteResponse,
    *,
    confidence_floor: float = DEFAULT_CONFIDENCE_FLOOR,
) -> VerdictReport:
    """Turn a raw :class:`ExecuteResponse` into a :class:`VerdictReport`.

    The mapping is deterministic and carries no I/O, so it is safe to
    call from synchronous audit code.
    """
    v = response.verdict
    c = response.confidence

    if v is Verdict.CONVERGED:
        if c >= confidence_floor:
            return VerdictReport(
                label=v,
                confident=True,
                severity="ok",
                recommended_action=(
                    "Accept the decoded payload as authoritative."
                ),
                retryable=False,
            )
        return VerdictReport(
            label=v,
            confident=False,
            severity="warn",
            recommended_action=(
                f"Accept with caution — confidence {c:.2f} < floor "
                f"{confidence_floor:.2f}. Consider widening the cone or "
                f"increasing max_rounds."
            ),
            retryable=True,
        )

    if v is Verdict.CONTINUE:
        return VerdictReport(
            label=v,
            confident=False,
            severity="warn",
            recommended_action=(
                "Swarm exhausted max_rounds without convergence. Increase "
                "max_rounds or loosen convergence thresholds."
            ),
            retryable=True,
        )

    if v is Verdict.DIVERGENCE:
        return VerdictReport(
            label=v,
            confident=False,
            severity="error",
            recommended_action=(
                "Population split into incompatible basins. Do NOT use the "
                "payload. Review the cone and the problem embedding."
            ),
            retryable=False,
        )

    if v is Verdict.MODE_COLLAPSE:
        return VerdictReport(
            label=v,
            confident=False,
            severity="error",
            recommended_action=(
                "Population collapsed to a single trivial mode. Increase "
                "na-crit agents in the population or widen the cone."
            ),
            retryable=False,
        )

    # Verdict.UNSPECIFIED — server misbehaved.
    return VerdictReport(
        label=v,
        confident=False,
        severity="error",
        recommended_action=(
            "Server returned an unspecified verdict — treat as an outage "
            "and escalate."
        ),
        retryable=True,
    )


__all__ = [
    "DEFAULT_CONFIDENCE_FLOOR",
    "Severity",
    "Verdict",
    "VerdictReport",
    "analyze_verdict",
]

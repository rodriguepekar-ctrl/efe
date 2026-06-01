"""Severity classification — threshold → severity mapping.

Invariant **I3** (monotonicity) is enforced here : given a fixed
:class:`SeverityThresholds`, ``score → severity`` is monotone
non-decreasing. Two orientations are supported :

* ``orientation = "higher_is_worse"`` (default) — PSI, JS, MMD²,
  classifier AUC distance, Wasserstein : larger score → more drift.
* ``orientation = "lower_is_worse"`` — used for **p-values** (χ², KS
  when reported as p-value) : smaller p-value → more drift.

Default thresholds mirror M09 §12.4 / §3.4. They can be overridden
via an :class:`~efe_drift.alerting.rule.AlertRule`.
"""

from __future__ import annotations

from dataclasses import dataclass
from itertools import pairwise
from typing import Literal

Severity = Literal["none", "warning", "major", "critical"]
Orientation = Literal["higher_is_worse", "lower_is_worse"]


# ---------------------------------------------------------------------------
# SeverityThresholds
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SeverityThresholds:
    """Per-level numeric thresholds + orientation.

    Semantics :

    * ``higher_is_worse``: a score ≥ ``warning`` triggers WARNING,
      ≥ ``major`` triggers MAJOR, etc.
    * ``lower_is_worse``: a score ≤ ``warning`` triggers WARNING, etc.

    Any field can be ``None`` — that level is simply skipped. If all
    three are ``None``, every score classifies as ``none``.
    """

    warning: float | None = None
    major: float | None = None
    critical: float | None = None
    orientation: Orientation = "higher_is_worse"

    def __post_init__(self) -> None:
        # Validate monotonicity for the chosen orientation. A caller
        # that accidentally swaps warning and critical would otherwise
        # silently violate I3.
        vals = [(name, v) for name, v in (
            ("warning", self.warning),
            ("major", self.major),
            ("critical", self.critical),
        ) if v is not None]
        if self.orientation == "higher_is_worse":
            for (_, a), (_, b) in pairwise(vals):
                if a > b:
                    raise ValueError(
                        "higher_is_worse thresholds must be non-decreasing "
                        f"warning → major → critical, got {self!r}",
                    )
        else:
            for (_, a), (_, b) in pairwise(vals):
                if a < b:
                    raise ValueError(
                        "lower_is_worse thresholds must be non-increasing "
                        f"warning → major → critical, got {self!r}",
                    )

    def classify(self, score: float) -> Severity:
        """Return the severity label for ``score``."""
        if self.orientation == "higher_is_worse":
            if self.critical is not None and score >= self.critical:
                return "critical"
            if self.major is not None and score >= self.major:
                return "major"
            if self.warning is not None and score >= self.warning:
                return "warning"
            return "none"
        # lower_is_worse
        if self.critical is not None and score <= self.critical:
            return "critical"
        if self.major is not None and score <= self.major:
            return "major"
        if self.warning is not None and score <= self.warning:
            return "warning"
        return "none"


# ---------------------------------------------------------------------------
# Defaults per method (M09 §12.4)
# ---------------------------------------------------------------------------


DEFAULT_PSI: SeverityThresholds = SeverityThresholds(
    warning=0.10, major=0.25, critical=0.40,
    orientation="higher_is_worse",
)

DEFAULT_JS: SeverityThresholds = SeverityThresholds(
    warning=0.01, major=0.05, critical=0.15,
    orientation="higher_is_worse",
)

DEFAULT_KS_STAT: SeverityThresholds = SeverityThresholds(
    warning=0.10, major=0.20, critical=0.30,
    orientation="higher_is_worse",
)

# Chi-square severity lives on p-value, not on chi² directly.
DEFAULT_CHI_SQUARED_P: SeverityThresholds = SeverityThresholds(
    warning=0.05, major=0.01, critical=0.001,
    orientation="lower_is_worse",
)

DEFAULT_WASSERSTEIN: SeverityThresholds = SeverityThresholds(
    warning=0.10, major=0.25, critical=0.50,
    orientation="higher_is_worse",
)

DEFAULT_MMD: SeverityThresholds = SeverityThresholds(
    warning=0.05, major=0.10, critical=0.20,
    orientation="higher_is_worse",
)

DEFAULT_CLASSIFIER_AUC_DIST: SeverityThresholds = SeverityThresholds(
    warning=0.10, major=0.30, critical=0.50,
    orientation="higher_is_worse",
)


DEFAULTS: dict[str, SeverityThresholds] = {
    "psi": DEFAULT_PSI,
    "js_divergence": DEFAULT_JS,
    "ks": DEFAULT_KS_STAT,
    "chi_squared": DEFAULT_CHI_SQUARED_P,
    "wasserstein": DEFAULT_WASSERSTEIN,
    "mmd": DEFAULT_MMD,
    "classifier_based": DEFAULT_CLASSIFIER_AUC_DIST,
}


__all__ = [
    "DEFAULTS",
    "DEFAULT_CHI_SQUARED_P",
    "DEFAULT_CLASSIFIER_AUC_DIST",
    "DEFAULT_JS",
    "DEFAULT_KS_STAT",
    "DEFAULT_MMD",
    "DEFAULT_PSI",
    "DEFAULT_WASSERSTEIN",
    "Orientation",
    "Severity",
    "SeverityThresholds",
]

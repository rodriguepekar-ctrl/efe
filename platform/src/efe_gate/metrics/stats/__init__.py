"""Statistical tests — McNemar, bootstrap CI, TOST, Welch t."""

from __future__ import annotations

from efe_gate.metrics.stats.bootstrap import (
    BootstrapResult,
    bootstrap_ci,
    bootstrap_delta_ci,
)
from efe_gate.metrics.stats.mcnemar import McNemarResult, mcnemar_test
from efe_gate.metrics.stats.tost import TostResult, TostVerdict, tost_equivalence
from efe_gate.metrics.stats.welch import WelchResult, welch_test

__all__ = [
    "BootstrapResult",
    "McNemarResult",
    "TostResult",
    "TostVerdict",
    "WelchResult",
    "bootstrap_ci",
    "bootstrap_delta_ci",
    "mcnemar_test",
    "tost_equivalence",
    "welch_test",
]

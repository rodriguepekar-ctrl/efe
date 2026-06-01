"""Pure-math helpers for the M17 reasoning / diagnostic / composite tools.

These modules contain *no I/O, no networking, no LLM* — purely
deterministic operations on numpy arrays + small dataclasses. They are
called from the tool handlers when a backend EFE 1.0 server is in use
(the EFE 1.1 contract makes these computations server-side, but the
mcp-server can also run them locally for backward compatibility).
"""

from .reasoning import (
    explain_top1,
    compare_centroids,
    threshold_sensitivity_range,
)
from .stats import histogram, percentiles, recommend_threshold_f1
from .drift import per_class_drift, overall_drift_score
from .thresholds import calibrate_per_class

__all__ = [
    "explain_top1",
    "compare_centroids",
    "threshold_sensitivity_range",
    "histogram",
    "percentiles",
    "recommend_threshold_f1",
    "per_class_drift",
    "overall_drift_score",
    "calibrate_per_class",
]

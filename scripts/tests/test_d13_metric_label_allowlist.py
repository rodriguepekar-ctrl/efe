"""Sprint D13 — metric-label allowlist hardening.

The D13 metrics declared in
`crates/efe-telemetry/src/metrics/d13_instruments.rs` use a
strict, finite-set label vocabulary. This test pins the
allowlist : adding a label requires test edit AND cardinality
review.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_METRICS = _REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "d13_instruments.rs"


# Canonical (metric_name, expected_labels) tuples.
EXPECTED = [
    ("efe_catalog_register_duration_seconds", ["tenant", "result"]),
    ("efe_catalog_evolve_duration_seconds",   ["tenant", "op", "result"]),
    ("efe_catalog_get_duration_seconds",      ["tenant", "include_compiled", "result"]),
    ("efe_catalog_list_duration_seconds",     ["tenant", "result"]),
    ("efe_catalog_versions_per_name",         ["tenant", "name"]),
    ("efe_audit_events_total",                ["tenant", "action", "op_tag", "principal_present"]),
    ("efe_alert_fires_total",                 ["alertname", "severity", "tenant"]),
    ("efe_dashboard_query_duration_seconds",  ["dashboard", "panel", "outcome"]),
]


def _file_text() -> str:
    return _METRICS.read_text(encoding="utf-8")


@pytest.mark.parametrize("metric, expected_labels", EXPECTED)
def test_metric_has_expected_labels(metric, expected_labels):
    """Pin the label set declared in the Rust file."""
    text = _file_text()
    # The file contains the metric name as a string. Find the closest
    # `&[...]` slice declaration (label list) after it.
    idx = text.find(f'"{metric}"')
    assert idx != -1, f"metric {metric!r} not found in d13_instruments.rs"

    # Walk forward and find the next `&[` (label list).
    after = text[idx:]
    m = re.search(r"&\[([^\]]+)\]", after)
    assert m, f"could not find label list after {metric!r}"
    raw = m.group(1)
    labels = re.findall(r'"([^"]+)"', raw)
    assert labels == expected_labels, (
        f"label drift for {metric}:\n"
        f"  expected: {expected_labels}\n"
        f"  actual:   {labels}\n"
        f"If intentional, update EXPECTED in this test AND review "
        f"cardinality in docs/howto/cardinality-management.md."
    )


def test_no_metric_has_request_id_label():
    """Cardinality bomb defense — request_id is unbounded ; must NOT
    appear as a Prometheus label."""
    text = _file_text()
    forbidden = ('"request_id"', '"trace_id"', '"span_id"', '"text"',
                 '"customer_email"')
    for f in forbidden:
        assert f not in text, (
            f"forbidden high-cardinality label {f} appears in "
            f"d13_instruments.rs"
        )


def test_no_metric_has_more_than_4_labels():
    """Cardinality budget — each metric should have ≤ 4 labels."""
    for metric, labels in EXPECTED:
        assert len(labels) <= 4, (
            f"metric {metric} has {len(labels)} labels (> 4 risks "
            f"cardinality explosion)"
        )

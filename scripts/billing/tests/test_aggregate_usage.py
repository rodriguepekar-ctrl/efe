"""Tests for `scripts/billing/aggregate_usage.py` — Sprint D11.5.

Pinned invariants:
  1. `_sum_series_values` adds float values, skips NaN/null.
  2. `aggregate_metric` accumulates a metric into the output dict
     under the named key.
  3. `normalise_usage` coerces types + applies tenant allowlist.
  4. `_window_label` produces a Prometheus-compatible duration.
  5. CSV and JSON emitters produce the documented schema.
  6. CLI exits 2 on bad ISO 8601 input.
  7. CLI exits 2 on bad output extension.

Tests don't hit a real Prometheus — pure functional tests on the
intermediate data structures.
"""

from __future__ import annotations

import csv
import importlib.util
import json
import sys
from pathlib import Path

import pytest


_REPO = Path(__file__).resolve().parents[2]
_SCRIPT = _REPO / "billing" / "aggregate_usage.py"
_spec = importlib.util.spec_from_file_location("aggregate_usage", _SCRIPT)
assert _spec and _spec.loader
mod = importlib.util.module_from_spec(_spec)
sys.modules["aggregate_usage"] = mod
_spec.loader.exec_module(mod)


# ─────────────────────────────────────────────────────────────────────
# 1. _sum_series_values
# ─────────────────────────────────────────────────────────────────────

def test_sum_series_values_basic():
    series = {"values": [[1.0, "10"], [2.0, "20"], [3.0, "5"]]}
    assert mod._sum_series_values(series) == pytest.approx(35.0)


def test_sum_series_values_skips_nan_strings():
    series = {"values": [[1.0, "10"], [2.0, "NaN"], [3.0, "5"]]}
    # NaN-string is dropped, not summed.
    assert mod._sum_series_values(series) == pytest.approx(15.0)


def test_sum_series_values_skips_null():
    series = {"values": [[1.0, None], [2.0, "5"]]}
    assert mod._sum_series_values(series) == pytest.approx(5.0)


def test_sum_series_values_empty():
    assert mod._sum_series_values({"values": []}) == 0.0


def test_sum_series_values_no_values_key():
    assert mod._sum_series_values({"metric": {"tenant": "x"}}) == 0.0


# ─────────────────────────────────────────────────────────────────────
# 2. aggregate_metric
# ─────────────────────────────────────────────────────────────────────

def test_aggregate_metric_accumulates_per_tenant():
    payload = {
        "data": {
            "result": [
                {
                    "metric": {"tenant": "acme"},
                    "values": [[1.0, "10"], [2.0, "20"]],
                },
                {
                    "metric": {"tenant": "globex"},
                    "values": [[1.0, "5"]],
                },
            ]
        }
    }
    out: dict = {}
    mod.aggregate_metric(payload, "requests", out)
    assert out["acme"]["requests"] == pytest.approx(30.0)
    assert out["globex"]["requests"] == pytest.approx(5.0)


def test_aggregate_metric_handles_missing_tenant_label():
    payload = {
        "data": {
            "result": [
                {"metric": {}, "values": [[1.0, "7"]]},
            ]
        }
    }
    out: dict = {}
    mod.aggregate_metric(payload, "requests", out)
    assert out["_unknown"]["requests"] == pytest.approx(7.0)


# ─────────────────────────────────────────────────────────────────────
# 3. normalise_usage
# ─────────────────────────────────────────────────────────────────────

def test_normalise_usage_coerces_types():
    raw = {
        "acme": {
            "requests": 100.0,
            "compute_seconds": 12.345678,
            "egress_bytes": 1024.7,
        },
    }
    out = mod.normalise_usage(raw)
    assert out["acme"]["requests"] == 100        # int
    assert out["acme"]["compute_seconds"] == 12.35  # rounded 2 decimals
    assert out["acme"]["egress_bytes"] == 1024   # int (truncated)


def test_normalise_usage_drops_unknown_tenant():
    raw = {
        "acme": {"requests": 1, "compute_seconds": 1.0, "egress_bytes": 100},
        "_unknown": {"requests": 99, "compute_seconds": 99.0, "egress_bytes": 9900},
    }
    out = mod.normalise_usage(raw)
    assert "_unknown" not in out
    assert "acme" in out


def test_normalise_usage_applies_allowlist():
    raw = {
        "acme": {"requests": 1, "compute_seconds": 1.0, "egress_bytes": 100},
        "globex": {"requests": 2, "compute_seconds": 2.0, "egress_bytes": 200},
    }
    out = mod.normalise_usage(raw, allow_tenants=["acme"])
    assert "acme" in out
    assert "globex" not in out


def test_normalise_usage_handles_missing_metric_keys():
    raw = {"acme": {"requests": 100}}  # no compute_seconds, no egress_bytes
    out = mod.normalise_usage(raw)
    # Missing keys default to 0.
    assert out["acme"] == {"requests": 100, "compute_seconds": 0.0, "egress_bytes": 0}


# ─────────────────────────────────────────────────────────────────────
# 4. _window_label
# ─────────────────────────────────────────────────────────────────────

def test_window_label_short_ranges():
    assert mod._window_label("2026-04-01T00:00:00Z", "2026-04-01T00:00:30Z") == "30s"
    assert mod._window_label("2026-04-01T00:00:00Z", "2026-04-01T00:05:00Z") == "5m"
    assert mod._window_label("2026-04-01T00:00:00Z", "2026-04-01T01:00:00Z") == "1h"


def test_window_label_monthly():
    # 30 days for typical billing cycle.
    label = mod._window_label("2026-04-01T00:00:00Z", "2026-05-01T00:00:00Z")
    assert label == "30d"


def test_window_label_full_year():
    label = mod._window_label("2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z")
    assert label == "365d"


# ─────────────────────────────────────────────────────────────────────
# 5. emit_csv / emit_json schemas
# ─────────────────────────────────────────────────────────────────────

def test_emit_csv_columns_and_sorted_rows(tmp_path: Path):
    usage = {
        "globex": {"requests": 5, "compute_seconds": 1.0, "egress_bytes": 100},
        "acme":   {"requests": 9, "compute_seconds": 2.0, "egress_bytes": 200},
    }
    out = tmp_path / "u.csv"
    mod.emit_csv(usage, out)
    with out.open() as f:
        rows = list(csv.reader(f))
    assert rows[0] == ["tenant", "requests", "compute_seconds", "egress_bytes"]
    # Rows MUST be sorted alphabetically by tenant.
    assert rows[1][0] == "acme"
    assert rows[2][0] == "globex"


def test_emit_json_has_schema_metadata(tmp_path: Path):
    usage = {"acme": {"requests": 1, "compute_seconds": 1.0, "egress_bytes": 100}}
    out = tmp_path / "u.json"
    mod.emit_json(usage, out, metadata={"prom_url": "http://x:9090"})
    payload = json.loads(out.read_text())
    assert payload["schema_version"] == "1.0"
    assert "generated_at" in payload
    assert payload["metadata"]["prom_url"] == "http://x:9090"
    assert payload["tenants"] == usage


def test_emit_csv_creates_parent_directories(tmp_path: Path):
    out = tmp_path / "deep" / "nested" / "u.csv"
    mod.emit_csv({"acme": {"requests": 1, "compute_seconds": 1.0, "egress_bytes": 1}}, out)
    assert out.exists()


# ─────────────────────────────────────────────────────────────────────
# 6-7. CLI argument validation
# ─────────────────────────────────────────────────────────────────────

def test_cli_rejects_bad_iso_start(tmp_path: Path):
    out = tmp_path / "u.csv"
    rc = mod.main([
        "--prom-url", "http://localhost:9090",
        "--start", "not-an-iso",
        "--end", "2026-04-01T00:00:00Z",
        "--output", str(out),
    ])
    assert rc == 2


def test_cli_rejects_bad_output_extension(tmp_path: Path, monkeypatch):
    # Patch query_instant so we don't actually hit Prometheus.
    monkeypatch.setattr(
        mod, "query_instant",
        lambda *a, **kw: {"data": {"result": []}},
    )
    rc = mod.main([
        "--prom-url", "http://localhost:9090",
        "--start", "2026-04-01T00:00:00Z",
        "--end", "2026-05-01T00:00:00Z",
        "--output", str(tmp_path / "u.txt"),
    ])
    assert rc == 2


def test_cli_end_to_end_with_mock(tmp_path: Path, monkeypatch):
    """Full path using a stubbed Prometheus instant query.

    Audit fix C1 — the production path now uses ``query_instant``,
    so mock that function. The response shape is one ``value``
    pair per series (not a ``values`` list as in query_range).
    """
    queries_seen: list[tuple[str, str]] = []

    def fake_query_instant(prom_url, query, time_, **kw):
        queries_seen.append((query, time_))
        return {
            "data": {
                "result": [
                    {
                        "metric": {"tenant": "acme"},
                        "value": [1.0, "100"],
                    },
                ]
            }
        }

    monkeypatch.setattr(mod, "query_instant", fake_query_instant)
    out = tmp_path / "u.json"
    rc = mod.main([
        "--prom-url", "http://prom:9090",
        "--start", "2026-04-01T00:00:00Z",
        "--end", "2026-05-01T00:00:00Z",
        "--output", str(out),
    ])
    assert rc == 0
    assert len(queries_seen) == 3   # requests + compute + egress
    # All 3 queries evaluated AT t=end (audit fix C1).
    for _, time_ in queries_seen:
        assert time_ == "2026-05-01T00:00:00Z"
    payload = json.loads(out.read_text())
    assert payload["tenants"]["acme"]["requests"] == 100
    assert payload["tenants"]["acme"]["compute_seconds"] == 100.0
    assert payload["tenants"]["acme"]["egress_bytes"] == 100


# ─────────────────────────────────────────────────────────────────────
# Audit fix C1 — instant query shape support
# ─────────────────────────────────────────────────────────────────────

def test_aggregate_metric_handles_instant_query_shape():
    """Instant queries return a single ``value`` pair per series,
    not a ``values`` list. The aggregator must handle both."""
    payload = {
        "data": {
            "result": [
                {
                    "metric": {"tenant": "acme"},
                    "value": [1.0, "42"],
                },
            ]
        }
    }
    out: dict = {}
    mod.aggregate_metric(payload, "requests", out)
    assert out["acme"]["requests"] == pytest.approx(42.0)


def test_extract_instant_value_returns_value():
    series = {"value": [1.0, "42.5"]}
    assert mod._extract_instant_value(series) == pytest.approx(42.5)


def test_extract_instant_value_handles_nan():
    series = {"value": [1.0, "NaN"]}
    assert mod._extract_instant_value(series) == 0.0


def test_extract_instant_value_handles_missing_value():
    assert mod._extract_instant_value({"metric": {"tenant": "x"}}) == 0.0


def test_extract_instant_value_handles_malformed_pair():
    assert mod._extract_instant_value({"value": [1.0]}) == 0.0
    assert mod._extract_instant_value({"value": []}) == 0.0


def test_no_overcounting_in_instant_query_path():
    """The previous bug summed 720 values per series ; now each
    series carries one value. Verify a 100-element values list
    in `value` is NOT summed."""
    payload = {
        "data": {
            "result": [
                {
                    "metric": {"tenant": "acme"},
                    "value": [1.0, "100"],   # single value, not a list
                },
            ]
        }
    }
    out: dict = {}
    mod.aggregate_metric(payload, "requests", out)
    # 100 — not 100×N steps.
    assert out["acme"]["requests"] == pytest.approx(100.0)

"""Sprint D13 Phase 2 — cardinality_probe.py operational tool tests."""

from __future__ import annotations

import importlib.util
import sys
from io import StringIO
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_PROBE_PATH = _REPO / "scripts" / "observability" / "cardinality_probe.py"


def _load_probe():
    spec = importlib.util.spec_from_file_location("cardinality_probe", _PROBE_PATH)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["cardinality_probe"] = mod
    spec.loader.exec_module(mod)
    return mod


SAMPLE_METRICS = """\
# HELP efe_grpc_requests_total Total gRPC requests handled
# TYPE efe_grpc_requests_total counter
efe_grpc_requests_total{method="Classify",code="OK",tenant="acme"} 100
efe_grpc_requests_total{method="Classify",code="OK",tenant="globex"} 200
efe_grpc_requests_total{method="Register",code="OK",tenant="acme"} 5

# HELP efe_grpc_request_duration_seconds Request latency
# TYPE efe_grpc_request_duration_seconds histogram
efe_grpc_request_duration_seconds_bucket{method="Classify",tenant="acme",le="0.001"} 1
efe_grpc_request_duration_seconds_bucket{method="Classify",tenant="acme",le="0.01"} 10
efe_grpc_request_duration_seconds_count{method="Classify",tenant="acme"} 100

# Recording rule
efe:rpc_p99_latency_seconds:5m{tenant="acme"} 0.123

# Non-EFE metric (should be filtered out)
container_cpu_usage_seconds_total{namespace="default"} 42

# Comment
"""


def test_parse_cardinality_basic():
    mod = _load_probe()
    cardinality = mod.parse_cardinality(SAMPLE_METRICS)
    # 3 distinct (method, code, tenant) tuples for efe_grpc_requests_total
    assert cardinality["efe_grpc_requests_total"] == 3
    # 2 buckets + 1 count for the histogram
    assert cardinality["efe_grpc_request_duration_seconds_bucket"] == 2
    assert cardinality["efe_grpc_request_duration_seconds_count"] == 1
    # Recording rule counted
    assert cardinality["efe:rpc_p99_latency_seconds:5m"] == 1


def test_filter_efe_strips_non_efe_metrics():
    mod = _load_probe()
    cardinality = mod.parse_cardinality(SAMPLE_METRICS)
    filtered = mod.filter_efe(cardinality)
    assert "container_cpu_usage_seconds_total" not in filtered
    assert "efe_grpc_requests_total" in filtered
    assert "efe:rpc_p99_latency_seconds:5m" in filtered


def test_report_text_under_budget_returns_zero():
    mod = _load_probe()
    cardinality = mod.filter_efe(mod.parse_cardinality(SAMPLE_METRICS))
    out = StringIO()
    rc = mod.report_text(cardinality, top=10, panic_budget=10_000, out=out)
    assert rc == 0
    text = out.getvalue()
    assert "OK" in text
    assert "Total efe_*" in text


def test_report_text_over_budget_returns_one():
    mod = _load_probe()
    cardinality = mod.filter_efe(mod.parse_cardinality(SAMPLE_METRICS))
    out = StringIO()
    rc = mod.report_text(cardinality, top=10, panic_budget=1, out=out)
    assert rc == 1
    text = out.getvalue()
    assert "OVER BUDGET" in text


def test_report_json_emits_schema_versioned_doc():
    mod = _load_probe()
    cardinality = mod.filter_efe(mod.parse_cardinality(SAMPLE_METRICS))
    out = StringIO()
    rc = mod.report_json(cardinality, panic_budget=10_000, out=out)
    assert rc == 0
    import json
    doc = json.loads(out.getvalue())
    assert doc["schema_version"] == "1.0"
    assert doc["tool"] == "cardinality_probe"
    assert doc["over_budget"] is False
    assert "families" in doc


def test_main_with_input_file(tmp_path):
    mod = _load_probe()
    sample = tmp_path / "metrics.txt"
    sample.write_text(SAMPLE_METRICS)
    # Capture stdout via monkey-patching sys.stdout
    import io
    import contextlib
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = mod.main([
            "--input-file", str(sample),
            "--top", "5",
            "--panic-budget", "10000",
            "--format", "text",
        ])
    assert rc == 0
    assert "Total efe_*" in buf.getvalue()


def test_main_json_format(tmp_path):
    mod = _load_probe()
    sample = tmp_path / "metrics.txt"
    sample.write_text(SAMPLE_METRICS)
    import io
    import contextlib
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = mod.main([
            "--input-file", str(sample),
            "--panic-budget", "10000",
            "--format", "json",
        ])
    assert rc == 0
    import json
    doc = json.loads(buf.getvalue())
    assert doc["schema_version"] == "1.0"

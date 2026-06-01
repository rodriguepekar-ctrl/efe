"""Tests for the metrics registry + Prometheus rendering."""

from __future__ import annotations

from efe_platform.telemetry.metrics import MetricsRegistry, RequestMetrics


def test_render_includes_headers() -> None:
    m = MetricsRegistry()
    txt = m.render()
    # HELP + TYPE lines are always emitted even for empty metrics.
    assert "# HELP efe_platform_requests_total" in txt
    assert "# TYPE efe_platform_requests_total counter" in txt


def test_record_request_produces_line() -> None:
    m = MetricsRegistry()
    m.record_request(
        RequestMetrics(
            tenant="acme",
            outcome="ok",
            duration_s=0.25,
            tool_calls=2,
            llm_tokens=100,
            llm_usd=0.001,
        )
    )
    txt = m.render()
    assert 'efe_platform_requests_total{outcome="ok",tenant="acme"} 1.0' in txt
    assert 'efe_platform_request_duration_seconds_count{tenant="acme"} 1.0' in txt


def test_record_budget_exceeded() -> None:
    m = MetricsRegistry()
    m.record_budget_exceeded(dimension="tokens")
    m.record_budget_exceeded(dimension="tokens")
    snap = m.snapshot()
    assert snap["efe_platform_budget_exceeded_total"]["dimension=tokens"] == 2.0


def test_record_tool_call_ok_vs_error() -> None:
    m = MetricsRegistry()
    m.record_tool_call(tool="swarm.classify", ok=True)
    m.record_tool_call(tool="swarm.classify", ok=False)
    snap = m.snapshot()
    bucket = snap["efe_platform_tool_calls_total"]
    assert bucket["outcome=ok,tool=swarm.classify"] == 1.0
    assert bucket["outcome=error,tool=swarm.classify"] == 1.0


def test_label_escape_in_render() -> None:
    m = MetricsRegistry()
    m.record_request(
        RequestMetrics(
            tenant='bad"tenant',
            outcome="ok",
            duration_s=0.1,
            tool_calls=0,
            llm_tokens=0,
            llm_usd=0.0,
        )
    )
    txt = m.render()
    assert 'tenant="bad\\"tenant"' in txt

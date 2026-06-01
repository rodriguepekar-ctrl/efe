"""Tests for :mod:`efe_tool.integrations._metrics` and ``._tracing``."""

from __future__ import annotations

import pytest

from efe_tool.integrations import MetricsRegistry, Tracer
from efe_tool.integrations._metrics import _HAS_PROMETHEUS
from efe_tool.integrations._tracing import _HAS_OTEL


class TestMetricsNoProm:
    """These tests work both with and without prometheus_client installed.
    If prometheus_client is available, we register on a custom collector
    to avoid clashing with the default registry across tests."""

    def test_registry_constructs(self):
        m = MetricsRegistry(namespace="test_noprom")
        # Never raises — all calls are null-safe.
        m.record_classify(catalog="x", label="a", outcome="ok", elapsed_s=0.1)
        m.record_error(catalog="x", error_code="EFE_USAGE")
        m.inflight_inc(catalog="x")
        m.inflight_dec(catalog="x")
        m.observe_preload(seconds=1.0)
        m.set_ready(ready=True)


@pytest.mark.skipif(not _HAS_PROMETHEUS, reason="prometheus_client not installed")
class TestMetricsWithProm:
    def test_metrics_are_registered(self):
        from prometheus_client import CollectorRegistry
        reg = CollectorRegistry()
        m = MetricsRegistry(namespace="test_withprom", registry=reg)
        m.record_classify(catalog="x", label="a", outcome="ok", elapsed_s=0.2)
        # Sample the counter
        names = [c._name for c in reg._collector_to_names]  # type: ignore[attr-defined]
        assert any("test_withprom_classify" in n for n in names)


class TestTracerDisabled:
    def test_disabled_is_noop(self):
        t = Tracer(enabled=False)
        with t.span("efe.classify", attributes={"catalog": "x"}) as span:
            span.set_attribute("k", "v")  # should not raise
            span.record_exception(RuntimeError("x"))


@pytest.mark.skipif(not _HAS_OTEL, reason="opentelemetry-api not installed")
class TestTracerEnabled:
    def test_span_created(self):
        t = Tracer(enabled=True)
        with t.span("efe.classify", attributes={"catalog": "x"}) as span:
            # The span object has the OTel interface.
            assert hasattr(span, "set_attribute")

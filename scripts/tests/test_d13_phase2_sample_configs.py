"""Sprint D13 Phase 2 — sample configs for OTel collector,
Grafana datasources, Loki server."""

from __future__ import annotations

from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]


# ─────────────────────────────────────────────────────────────────────
# OTel collector
# ─────────────────────────────────────────────────────────────────────


def test_otel_collector_config_exists():
    p = _REPO / "deploy" / "observability" / "otel-collector" / "config.yaml"
    assert p.exists()


def test_otel_collector_has_otlp_receivers():
    p = _REPO / "deploy" / "observability" / "otel-collector" / "config.yaml"
    text = p.read_text()
    assert "otlp:" in text
    # Both gRPC + HTTP entrypoints documented.
    assert "4317" in text  # OTLP/gRPC
    assert "4318" in text  # OTLP/HTTP


def test_otel_collector_has_pii_blocking():
    """Defense vs T-13-3 (trace data exposure of PII)."""
    p = _REPO / "deploy" / "observability" / "otel-collector" / "config.yaml"
    text = p.read_text()
    for pii in ("text", "email", "phone", "address", "jwt", "token",
                "password", "secret", "api_key", "customer_email"):
        assert f"key: {pii}" in text, (
            f"otel-collector must delete attr {pii!r}"
        )


def test_otel_collector_has_memory_limiter():
    p = _REPO / "deploy" / "observability" / "otel-collector" / "config.yaml"
    text = p.read_text()
    assert "memory_limiter" in text, (
        "otel-collector must enable memory_limiter (back-pressure)"
    )


def test_otel_collector_exports_to_tempo():
    p = _REPO / "deploy" / "observability" / "otel-collector" / "config.yaml"
    text = p.read_text()
    assert "otlp/tempo" in text
    assert "tempo-distributor" in text


# ─────────────────────────────────────────────────────────────────────
# Grafana datasources
# ─────────────────────────────────────────────────────────────────────


def test_grafana_datasources_exists():
    p = _REPO / "deploy" / "observability" / "grafana" / "datasources" / "datasources.yaml"
    assert p.exists()


def test_grafana_datasources_includes_prom_loki_tempo():
    p = _REPO / "deploy" / "observability" / "grafana" / "datasources" / "datasources.yaml"
    text = p.read_text()
    assert "type: prometheus" in text
    assert "type: loki" in text
    assert "type: tempo" in text
    assert "type: alertmanager" in text


def test_grafana_prometheus_has_exemplar_correlation():
    """Exemplar correlation links Prom histograms to Tempo traces."""
    p = _REPO / "deploy" / "observability" / "grafana" / "datasources" / "datasources.yaml"
    text = p.read_text()
    assert "exemplarTraceIdDestinations" in text
    assert "datasourceUid: tempo" in text


def test_grafana_loki_has_derived_fields():
    """Derived fields make trace_id labels clickable from log lines."""
    p = _REPO / "deploy" / "observability" / "grafana" / "datasources" / "datasources.yaml"
    text = p.read_text()
    assert "derivedFields" in text
    assert "trace_id" in text


def test_grafana_tempo_has_traces_to_logs_correlation():
    p = _REPO / "deploy" / "observability" / "grafana" / "datasources" / "datasources.yaml"
    text = p.read_text()
    assert "tracesToLogs" in text
    assert "datasourceUid: loki" in text


# ─────────────────────────────────────────────────────────────────────
# Loki server config
# ─────────────────────────────────────────────────────────────────────


def test_loki_config_exists():
    p = _REPO / "deploy" / "observability" / "loki" / "loki-config.example.yaml"
    assert p.exists()


def test_loki_has_multitenancy_on():
    """Multi-tenant requires auth_enabled: true."""
    p = _REPO / "deploy" / "observability" / "loki" / "loki-config.example.yaml"
    text = p.read_text()
    assert "auth_enabled: true" in text


def test_loki_has_per_stream_retention():
    """Critical : audit stream MUST have 7-year retention."""
    p = _REPO / "deploy" / "observability" / "loki" / "loki-config.example.yaml"
    text = p.read_text()
    assert "retention_stream" in text
    # Audit stream — 7 years = 2555 days
    assert "2555d" in text
    assert 'stream="audit"' in text


def test_loki_has_ingester_rate_limits():
    """Per-tenant ingest quotas defend Loki from one tenant flooding."""
    p = _REPO / "deploy" / "observability" / "loki" / "loki-config.example.yaml"
    text = p.read_text()
    assert "ingestion_rate_mb" in text
    assert "ingestion_burst_size_mb" in text
    assert "max_streams_per_user" in text

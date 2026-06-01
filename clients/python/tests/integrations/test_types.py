"""Tests for :mod:`efe_tool.integrations._types`."""

from __future__ import annotations

import json

from efe_tool.integrations import FacadeStats, HealthStatus, Verdict


class TestVerdict:
    def test_zone_helpers(self):
        v = Verdict(
            label="fraud", confidence=0.9, reject=False, rounds=4,
            alignment_delta=0.1, catalog="x", zone="trusted", elapsed_ms=100,
        )
        assert v.is_trusted is True
        assert v.is_warn is False
        assert v.is_reject is False

    def test_warn(self):
        v = Verdict(
            label="x", confidence=0.6, reject=False, rounds=3,
            alignment_delta=0.1, catalog="x", zone="warn", elapsed_ms=100,
        )
        assert v.is_warn is True
        assert v.is_trusted is False

    def test_reject(self):
        v = Verdict(
            label="uncertain", confidence=0.2, reject=True, rounds=5,
            alignment_delta=0.3, catalog="x", zone="reject", elapsed_ms=200,
        )
        assert v.is_reject is True
        assert v.is_trusted is False

    def test_as_dict(self):
        v = Verdict(
            label="fraud", confidence=0.87654321, reject=False, rounds=4,
            alignment_delta=0.12, catalog="triage", zone="trusted",
            elapsed_ms=123, request_id="rid-abc",
        )
        d = v.as_dict()
        # JSON-serialisable round-trip
        assert json.loads(json.dumps(d)) == d
        assert d["label"] == "fraud"
        assert d["confidence"] == 0.876543  # rounded to 6 decimals
        assert d["zone"] == "trusted"


class TestHealthStatus:
    def test_serving(self):
        hs = HealthStatus(
            status="SERVING", latency_ms=10,
            checks={"grpc_reachable": True, "embedder_loaded": True},
        )
        assert hs.serving is True

    def test_not_serving(self):
        hs = HealthStatus(
            status="NOT_SERVING", latency_ms=1000,
            checks={"grpc_reachable": False},
            reason="server unreachable",
        )
        assert hs.serving is False
        assert hs.reason == "server unreachable"

    def test_as_dict(self):
        hs = HealthStatus(
            status="SERVING", latency_ms=10,
            checks={"a": True},
        )
        d = hs.as_dict()
        json.dumps(d)  # must be JSON-safe
        assert d["status"] == "SERVING"


class TestFacadeStats:
    def test_avg_latency_zero_when_no_samples(self):
        s = FacadeStats()
        assert s.avg_latency_ms == 0.0

    def test_avg_latency_computation(self):
        s = FacadeStats(latency_sum_ms=1500.0, latency_count=3)
        assert s.avg_latency_ms == 500.0

    def test_as_dict(self):
        s = FacadeStats(
            classify_total=10, classify_failed=2,
            classify_rejected=1, in_flight=1,
            latency_sum_ms=1000.0, latency_count=5,
            latency_p50_ms_last_window=150.0,
            latency_p95_ms_last_window=480.0,
            preload_ms=1200, is_ready=True,
            model="m", endpoint="h:1",
            catalogs_loaded=("a",),
        )
        d = s.as_dict()
        json.dumps(d)
        assert d["classify_total"] == 10
        assert d["avg_latency_ms"] == 200.0
        assert d["catalogs_loaded"] == ["a"]

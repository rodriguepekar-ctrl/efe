"""Sprint D13 Phase 2 — export_audit_archive.py operational tool."""

from __future__ import annotations

import gzip
import importlib.util
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_SCRIPT = _REPO / "scripts" / "observability" / "export_audit_archive.py"


def _load_script():
    spec = importlib.util.spec_from_file_location("export_audit_archive", _SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["export_audit_archive"] = mod
    spec.loader.exec_module(mod)
    return mod


def test_parse_iso_handles_z_suffix():
    mod = _load_script()
    dt = mod.parse_iso("2026-04-27T14:30:00Z")
    assert dt.tzinfo == timezone.utc
    assert dt.year == 2026 and dt.month == 4 and dt.day == 27


def test_parse_iso_assumes_utc_when_naive():
    mod = _load_script()
    # Some implementations might emit naive ; we assume UTC.
    dt = mod.parse_iso("2026-04-27T14:30:00")
    assert dt.tzinfo == timezone.utc


def test_to_unix_ns_round_trips():
    mod = _load_script()
    dt = datetime(2026, 4, 27, 14, 30, 0, tzinfo=timezone.utc)
    ns = mod.to_unix_ns(dt)
    assert isinstance(ns, int)
    # Unix seconds → nanoseconds : exactly 1e9 multiplier.
    expected_seconds = int(dt.timestamp())
    assert ns == expected_seconds * 1_000_000_000


def test_to_unix_ns_for_epoch():
    mod = _load_script()
    dt = datetime(1970, 1, 1, 0, 0, 0, tzinfo=timezone.utc)
    assert mod.to_unix_ns(dt) == 0


def test_validate_audit_event_accepts_canonical_event():
    mod = _load_script()
    ev = {
        "schema_version": "1.0",
        "stream": "audit",
        "timestamp": "2026-04-27T14:30:00.000000Z",
        "level": "INFO",
        "service": "efe-server",
        "service_version": "0.1.13",
        "tenant_id": "tenant-acme",
        "principal": "alice@acme.com",
        "action": "register",
        "resource_type": "catalog",
        "resource_key": "tenant-acme/billing@v1",
        "op_tag": "register",
    }
    assert mod.validate_audit_event(ev) is None


def test_validate_audit_event_rejects_missing_field():
    mod = _load_script()
    ev = {
        "schema_version": "1.0",
        "stream": "audit",
        # missing timestamp
        "level": "INFO",
        "service": "efe-server",
        "service_version": "0.1.13",
        "tenant_id": "tenant-acme",
        "principal": "alice@acme.com",
        "action": "register",
        "resource_type": "catalog",
        "resource_key": "tenant-acme/billing@v1",
        "op_tag": "register",
    }
    err = mod.validate_audit_event(ev)
    assert err is not None
    assert "timestamp" in err


def test_validate_audit_event_rejects_bad_action():
    mod = _load_script()
    ev = {
        "schema_version": "1.0",
        "stream": "audit",
        "timestamp": "2026-04-27T14:30:00Z",
        "level": "INFO",
        "service": "efe-server",
        "service_version": "0.1.13",
        "tenant_id": "tenant-acme",
        "principal": "alice@acme.com",
        "action": "BANANA",
        "resource_type": "catalog",
        "resource_key": "tenant-acme/billing@v1",
        "op_tag": "register",
    }
    err = mod.validate_audit_event(ev)
    assert err is not None
    assert "BANANA" in err or "action" in err


def test_validate_audit_event_rejects_bad_op_tag():
    mod = _load_script()
    ev = {
        "schema_version": "1.0",
        "stream": "audit",
        "timestamp": "2026-04-27T14:30:00Z",
        "level": "INFO",
        "service": "efe-server",
        "service_version": "0.1.13",
        "tenant_id": "tenant-acme",
        "principal": "alice@acme.com",
        "action": "evolve",
        "resource_type": "catalog",
        "resource_key": "tenant-acme/billing@v2",
        "op_tag": "BANANA",
    }
    err = mod.validate_audit_event(ev)
    assert err is not None


def test_main_writes_gzipped_ndjson(tmp_path, monkeypatch):
    mod = _load_script()
    fake_events = [
        {
            "schema_version": "1.0", "stream": "audit",
            "timestamp": "2026-04-27T14:30:00Z", "level": "INFO",
            "service": "efe-server", "service_version": "0.1.13",
            "tenant_id": "tenant-acme", "principal": "alice@acme.com",
            "action": "register", "resource_type": "catalog",
            "resource_key": "tenant-acme/billing@v1", "op_tag": "register",
        },
        {
            "schema_version": "1.0", "stream": "audit",
            "timestamp": "2026-04-27T14:31:00Z", "level": "INFO",
            "service": "efe-server", "service_version": "0.1.13",
            "tenant_id": "tenant-acme", "principal": "alice@acme.com",
            "action": "evolve", "resource_type": "catalog",
            "resource_key": "tenant-acme/billing@v2", "op_tag": "append_class",
        },
    ]
    monkeypatch.setattr(mod, "fetch_logs", lambda *a, **kw: fake_events)

    out = tmp_path / "audit.ndjson.gz"
    rc = mod.main([
        "--loki-url", "http://loki",
        "--tenant",   "tenant-acme",
        "--since",    "2026-04-01T00:00:00Z",
        "--until",    "2026-05-01T00:00:00Z",
        "--out",      str(out),
    ])
    assert rc == 0
    assert out.exists()
    # Decompress + count.
    with gzip.open(out, "rt") as f:
        lines = [json.loads(line) for line in f if line.strip()]
    assert len(lines) == 2
    assert lines[0]["action"] == "register"


def test_main_returns_1_on_invalid_event(tmp_path, monkeypatch):
    mod = _load_script()
    bad_events = [
        {"schema_version": "1.0", "stream": "audit"},  # too few fields
    ]
    monkeypatch.setattr(mod, "fetch_logs", lambda *a, **kw: bad_events)
    out = tmp_path / "audit.ndjson.gz"
    rc = mod.main([
        "--loki-url", "http://loki",
        "--tenant",   "tenant-acme",
        "--since",    "2026-04-01T00:00:00Z",
        "--until",    "2026-05-01T00:00:00Z",
        "--out",      str(out),
    ])
    assert rc == 1


def test_main_rejects_inverted_window(tmp_path, monkeypatch):
    mod = _load_script()
    monkeypatch.setattr(mod, "fetch_logs", lambda *a, **kw: [])
    out = tmp_path / "audit.ndjson.gz"
    rc = mod.main([
        "--loki-url", "http://loki",
        "--tenant",   "tenant-acme",
        "--since",    "2026-05-01T00:00:00Z",
        "--until",    "2026-04-01T00:00:00Z",  # < since
        "--out",      str(out),
    ])
    assert rc == 2

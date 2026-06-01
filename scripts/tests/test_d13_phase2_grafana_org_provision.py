"""Sprint D13 Phase 2 — grafana_org_provision.py operational tool."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_SCRIPT = _REPO / "scripts" / "observability" / "grafana_org_provision.py"


def _load_script():
    spec = importlib.util.spec_from_file_location("grafana_org_provision", _SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["grafana_org_provision"] = mod
    spec.loader.exec_module(mod)
    return mod


def test_dry_run_does_not_call_grafana(monkeypatch, capsys):
    mod = _load_script()

    def boom(*a, **kw):
        raise AssertionError("Grafana API was called in --dry-run mode")
    monkeypatch.setattr(mod, "find_or_create_org",                  boom)
    monkeypatch.setattr(mod, "upsert_prometheus_datasource",        boom)
    rc = mod.main([
        "--grafana-url", "http://grafana:3000",
        "--token",       "fake",
        "--tenant",      "tenant-acme",
        "--prometheus-url", "http://mimir:9009/prometheus",
        "--dry-run",
    ])
    assert rc == 0
    captured = capsys.readouterr()
    assert "DRY RUN" in captured.out
    assert "tenant-acme" in captured.out


def test_org_name_defaults_to_tenant(capsys, monkeypatch):
    mod = _load_script()

    seen = {}
    def fake_find(_url, _tok, name):
        seen["org_name"] = name
        return 42
    def fake_upsert(*a, **kw):
        return None
    monkeypatch.setattr(mod, "find_or_create_org",           fake_find)
    monkeypatch.setattr(mod, "upsert_prometheus_datasource", fake_upsert)
    rc = mod.main([
        "--grafana-url", "http://grafana:3000",
        "--token",       "fake",
        "--tenant",      "tenant-acme",
        "--prometheus-url", "http://mimir:9009/prometheus",
    ])
    assert rc == 0
    assert seen["org_name"] == "tenant-acme"


def test_org_name_can_be_overridden(monkeypatch):
    mod = _load_script()
    seen = {}
    def fake_find(_url, _tok, name):
        seen["org_name"] = name
        return 1
    monkeypatch.setattr(mod, "find_or_create_org",           fake_find)
    monkeypatch.setattr(mod, "upsert_prometheus_datasource", lambda *a, **kw: None)
    rc = mod.main([
        "--grafana-url", "http://grafana:3000",
        "--token",       "fake",
        "--tenant",      "tenant-acme",
        "--prometheus-url", "http://mimir:9009/prometheus",
        "--org-name",    "EFE-Acme-Custom",
    ])
    assert rc == 0
    assert seen["org_name"] == "EFE-Acme-Custom"

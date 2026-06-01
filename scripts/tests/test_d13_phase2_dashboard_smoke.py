"""Sprint D13 Phase 2 — dashboard_smoke_test.py operational tool tests."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_SCRIPT = _REPO / "scripts" / "observability" / "dashboard_smoke_test.py"


def _load_smoke():
    spec = importlib.util.spec_from_file_location("dashboard_smoke_test", _SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["dashboard_smoke_test"] = mod
    spec.loader.exec_module(mod)
    return mod


def test_d13_uids_match_dashboard_files():
    mod = _load_smoke()
    expected = {
        "efe-d13-01-operator-overview",
        "efe-d13-02-tenant-drill-down",
        "efe-d13-03-catalog-version-distribution",
        "efe-d13-04-cost-attribution",
        "efe-d13-05-audit-stream",
        "efe-d13-06-capacity-planning",
    }
    assert set(mod.D13_DASHBOARD_UIDS) == expected


def test_d13_uids_count_is_6():
    mod = _load_smoke()
    assert len(mod.D13_DASHBOARD_UIDS) == 6


def test_uids_in_smoke_match_uids_in_dashboard_files():
    """Bidirectional check : every UID listed in the smoke script
    actually exists as a JSON file. The architecture pins both."""
    import json
    mod = _load_smoke()
    dash_dir = _REPO / "deploy" / "observability" / "grafana" / "dashboards" / "d13"
    file_uids = set()
    for f in dash_dir.glob("*.json"):
        d = json.loads(f.read_text())
        file_uids.add(d["uid"])
    assert set(mod.D13_DASHBOARD_UIDS) == file_uids

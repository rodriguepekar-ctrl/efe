"""Sprint D13 — surface pinning of the 6 dashboards.

Each dashboard's UID + title + folder + tag set is part of the
SaaS-of-EFE operator contract. Removal/renaming = SemVer-breaking
for partners who bookmarked a UID. This test enforces stability.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_DASH = _REPO / "deploy" / "observability" / "grafana" / "dashboards" / "d13"


EXPECTED = {
    "01_operator_overview": {
        "uid":   "efe-d13-01-operator-overview",
        "title": "EFE — 01 Operator Overview",
        "tags":  {"efe", "d13", "operator", "sre"},
    },
    "02_tenant_drill_down": {
        "uid":   "efe-d13-02-tenant-drill-down",
        "title": "EFE — 02 Tenant Drill-Down",
        "tags":  {"efe", "d13", "tenant", "support"},
    },
    "03_catalog_version_distribution": {
        "uid":   "efe-d13-03-catalog-version-distribution",
        "title": "EFE — 03 Catalog Version Distribution",
        "tags":  {"efe", "d13", "catalog", "curator"},
    },
    "04_cost_attribution": {
        "uid":   "efe-d13-04-cost-attribution",
        "title": "EFE — 04 Cost Attribution",
        "tags":  {"efe", "d13", "cost", "finance", "finops"},
    },
    "05_audit_stream": {
        "uid":   "efe-d13-05-audit-stream",
        "title": "EFE — 05 Audit Stream",
        "tags":  {"efe", "d13", "audit", "security"},
    },
    "06_capacity_planning": {
        "uid":   "efe-d13-06-capacity-planning",
        "title": "EFE — 06 Capacity Planning",
        "tags":  {"efe", "d13", "capacity", "sre"},
    },
}


@pytest.mark.parametrize("slug", list(EXPECTED.keys()))
def test_dashboard_uid_pinned(slug):
    d = json.loads((_DASH / f"{slug}.json").read_text())
    assert d["uid"] == EXPECTED[slug]["uid"], (
        f"dashboard {slug} uid drifted from {EXPECTED[slug]['uid']!r}"
    )


@pytest.mark.parametrize("slug", list(EXPECTED.keys()))
def test_dashboard_title_pinned(slug):
    d = json.loads((_DASH / f"{slug}.json").read_text())
    assert d["title"] == EXPECTED[slug]["title"], (
        f"dashboard {slug} title drifted from "
        f"{EXPECTED[slug]['title']!r}"
    )


@pytest.mark.parametrize("slug", list(EXPECTED.keys()))
def test_dashboard_tags_superset(slug):
    """The dashboard MAY add tags ; it MUST keep the canonical set."""
    d = json.loads((_DASH / f"{slug}.json").read_text())
    actual = set(d.get("tags", []))
    expected = EXPECTED[slug]["tags"]
    missing = expected - actual
    assert not missing, (
        f"dashboard {slug} missing tags: {missing}"
    )

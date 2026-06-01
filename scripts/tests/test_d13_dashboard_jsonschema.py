"""Sprint D13 — Grafana dashboard JSON sanity checks.

Validates :

  1. The 6 D13 dashboards exist at the canonical path.
  2. Each parses as JSON.
  3. Each carries a unique stable UID.
  4. Each declares schemaVersion ≥ 39 (Grafana 10+).
  5. Each has at least one panel.
  6. Each lists Prometheus as a required datasource.
  7. The 6 UIDs follow the `efe-d13-NN-<slug>` convention.
"""

from __future__ import annotations

import json
from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_DASH = _REPO / "deploy" / "observability" / "grafana" / "dashboards" / "d13"


EXPECTED_DASHBOARDS = (
    ("01_operator_overview",            "efe-d13-01-operator-overview"),
    ("02_tenant_drill_down",            "efe-d13-02-tenant-drill-down"),
    ("03_catalog_version_distribution", "efe-d13-03-catalog-version-distribution"),
    ("04_cost_attribution",             "efe-d13-04-cost-attribution"),
    ("05_audit_stream",                 "efe-d13-05-audit-stream"),
    ("06_capacity_planning",            "efe-d13-06-capacity-planning"),
)


def test_all_6_dashboards_exist():
    for slug, _ in EXPECTED_DASHBOARDS:
        p = _DASH / f"{slug}.json"
        assert p.exists(), f"missing dashboard {p}"


def test_each_dashboard_parses():
    for slug, _ in EXPECTED_DASHBOARDS:
        with (_DASH / f"{slug}.json").open() as fp:
            json.load(fp)


def test_each_dashboard_has_uid_and_title():
    for slug, expected_uid in EXPECTED_DASHBOARDS:
        d = json.loads((_DASH / f"{slug}.json").read_text())
        assert d["uid"] == expected_uid, (
            f"dashboard {slug} uid drifted: got {d['uid']!r}, "
            f"expected {expected_uid!r}"
        )
        assert d.get("title", "").startswith("EFE — "), (
            f"dashboard {slug} title must start with 'EFE — '"
        )


def test_each_dashboard_uses_grafana_10_schema():
    for slug, _ in EXPECTED_DASHBOARDS:
        d = json.loads((_DASH / f"{slug}.json").read_text())
        sv = d.get("schemaVersion", 0)
        assert sv >= 39, (
            f"dashboard {slug} schemaVersion {sv} < 39 (Grafana 10+)"
        )


def test_each_dashboard_has_at_least_3_panels():
    for slug, _ in EXPECTED_DASHBOARDS:
        d = json.loads((_DASH / f"{slug}.json").read_text())
        n = len(d.get("panels", []))
        assert n >= 3, f"dashboard {slug} has only {n} panels (< 3)"


def test_each_dashboard_requires_prometheus_datasource():
    for slug, _ in EXPECTED_DASHBOARDS:
        d = json.loads((_DASH / f"{slug}.json").read_text())
        requires = d.get("__requires", [])
        names = {r.get("name") for r in requires}
        assert "Prometheus" in names, (
            f"dashboard {slug} must declare Prometheus as required"
        )


def test_uids_are_unique():
    uids = []
    for slug, _ in EXPECTED_DASHBOARDS:
        d = json.loads((_DASH / f"{slug}.json").read_text())
        uids.append(d["uid"])
    assert len(uids) == len(set(uids)), f"duplicate UIDs: {uids}"


def test_uids_follow_naming_convention():
    """efe-d13-NN-<slug-with-dashes>"""
    import re
    pattern = re.compile(r"^efe-d13-\d{2}-[a-z]+(-[a-z]+)*$")
    for slug, _ in EXPECTED_DASHBOARDS:
        d = json.loads((_DASH / f"{slug}.json").read_text())
        uid = d["uid"]
        assert pattern.match(uid), (
            f"UID {uid!r} doesn't match efe-d13-NN-<slug>"
        )


def test_each_dashboard_links_to_runbooks_or_other_dashboards():
    for slug, _ in EXPECTED_DASHBOARDS:
        d = json.loads((_DASH / f"{slug}.json").read_text())
        links = d.get("links", [])
        # At least one link to either a runbook or another dashboard.
        assert links, f"dashboard {slug} has no links"
        urls = " ".join(L.get("url", "") for L in links)
        assert "/d/" in urls or "runbooks" in urls.lower(), (
            f"dashboard {slug} links don't reference runbooks "
            f"or sibling dashboards"
        )


def test_each_dashboard_tags_with_efe_and_d13():
    for slug, _ in EXPECTED_DASHBOARDS:
        d = json.loads((_DASH / f"{slug}.json").read_text())
        tags = set(d.get("tags", []))
        assert "efe" in tags, f"dashboard {slug} missing 'efe' tag"
        assert "d13" in tags, f"dashboard {slug} missing 'd13' tag"


def test_total_panel_count_at_least_60():
    """Sanity bound — the architecture promised ~ 80 panels. Allow
    margin but pin a floor."""
    total = 0
    for slug, _ in EXPECTED_DASHBOARDS:
        d = json.loads((_DASH / f"{slug}.json").read_text())
        total += len(d.get("panels", []))
    assert total >= 60, f"total panel count {total} < 60 (regression?)"

"""Sprint D13 audit — pin the fixes from `docs/audits/d13_observability_audit_report.md`.

Each test corresponds to one finding from the surgical audit pass.
Removal of any test below means the corresponding fix is back to a
risky state ; treat as a regression.

Findings :

  * **C-1** — `efe_audit_events_total` had no `principal` label
    but the alert + dashboard tried to filter on `principal=""`.
    Pre-fix the query returned empty and the alert never fired.
    Fix : add `principal_present` (bool string "true"|"false")
    label ; update alert + dashboard to filter
    `principal_present="false"`.

  * **C-2** — Helm `.Files.Glob` / `.Files.Get` used `../..`
    paths that escape the chart sandbox. Helm silently strips
    `..` ; rendered ConfigMaps were empty.
    Fix : copy canonical assets into `deploy/helm/efe/d13_files/`
    + use chart-local paths in templates + sync script keeps
    them in sync (pinned by `test_d13_helm_files_in_sync.py`).

  * **M-1** — `register_all_d13()` was DEFINED but never CALLED
    from `efe_telemetry::init()`. So the 8 D13 metric families
    were missing from `/metrics` until the first emit-site
    fired (deferred to D12-bis). Recording rules + alerts saw
    "no data" indefinitely.
    Fix : invoke `register_all_d13()` from `init()` so families
    appear in `/metrics` from scrape #1.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]


# ─────────────────────────────────────────────────────────────────────
# C-1 : principal_present label exists, queries use it
# ─────────────────────────────────────────────────────────────────────


def test_audit_events_total_has_principal_present_label():
    """The Rust metric MUST declare the `principal_present` label
    (bool string "true"|"false"). Pre-fix the metric had only
    [tenant, action, op_tag]."""
    text = (_REPO / "crates" / "efe-telemetry" / "src" / "metrics" /
            "d13_instruments.rs").read_text()
    # Find the `audit_events_total` declaration block.
    block = re.search(
        r"audit_events_total,\s*IntCounterVec,.*?expect\(",
        text, re.DOTALL,
    )
    assert block, "could not locate audit_events_total declaration"
    assert '"principal_present"' in block.group(0), (
        "audit C-1 regression : `principal_present` label removed "
        "from efe_audit_events_total. The alert "
        "D13ObsAuditPrincipalUnknown queries this label ; without "
        "it the alert query returns empty."
    )


def test_alert_d13obs_audit_principal_unknown_queries_principal_present():
    """The alert MUST filter `principal_present="false"`, NOT the
    pre-fix `principal=""` (which referenced a non-existent label)."""
    text = (_REPO / "deploy" / "observability" / "prometheus" /
            "rules.d13" / "alerts.yaml").read_text()
    # Find the alert block for D13ObsAuditPrincipalUnknown.
    m = re.search(
        r"alert: D13ObsAuditPrincipalUnknown.*?(?=alert: |\Z)",
        text, re.DOTALL,
    )
    assert m, "missing alert D13ObsAuditPrincipalUnknown"
    block = m.group(0)
    assert 'principal_present="false"' in block, (
        "audit C-1 regression : alert D13ObsAuditPrincipalUnknown "
        "must use principal_present=\"false\" filter"
    )
    # Pre-fix wording must NOT come back.
    assert 'principal=""' not in block, (
        "audit C-1 regression : alert reverted to pre-fix "
        "`principal=\"\"` filter (label doesn't exist)"
    )


def test_dashboard_05_audit_panel_uses_principal_present():
    text = (_REPO / "deploy" / "observability" / "grafana" /
            "dashboards" / "d13" / "05_audit_stream.json").read_text()
    assert 'principal_present=\\"false\\"' in text, (
        "audit C-1 regression : dashboard 05 audit panel must "
        "use principal_present=\"false\" (escaped JSON)"
    )
    assert 'principal=\\"\\"' not in text, (
        "audit C-1 regression : dashboard 05 panel reverted to "
        "pre-fix principal=\"\" filter"
    )


# ─────────────────────────────────────────────────────────────────────
# C-2 : Helm path escape
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("template_name", [
    "grafana-dashboards.yaml",
    "prometheusrule-d13.yaml",
    "promtail-config.yaml",
])
def test_helm_template_uses_chart_local_paths(template_name):
    """Audit C-2 fix : Helm directives must reference paths INSIDE
    the chart (deploy/helm/efe/...). Anything with `..` is silently
    stripped by Helm and the resulting ConfigMap is empty."""
    p = _REPO / "deploy" / "helm" / "efe" / "templates" / template_name
    text = p.read_text()
    # Strip Helm comment blocks.
    no_comments = re.sub(
        r"\{\{-?\s*/\*.*?\*/\s*-?\}\}",
        "",
        text, flags=re.DOTALL,
    )
    files_paths = re.findall(
        r"\.Files\.(?:Get|Glob)\s+\"([^\"]+)\"",
        no_comments,
    )
    assert files_paths, f"{template_name} has no Files.Get/Glob directive"
    for path in files_paths:
        assert ".." not in path, (
            f"audit C-2 regression : {template_name} has Files "
            f"directive on path {path!r} containing '..' (Helm sandbox "
            f"escape ; ConfigMap will render empty)"
        )
        assert "d13_files/" in path, (
            f"audit C-2 regression : {template_name} Files directive "
            f"path {path!r} must use the chart-local d13_files/ mirror"
        )


def test_chart_mirror_directory_exists():
    """The mirror directory under the chart MUST exist with the
    canonical sub-trees."""
    base = _REPO / "deploy" / "helm" / "efe" / "d13_files"
    assert base.is_dir()
    assert (base / "dashboards").is_dir()
    assert (base / "prometheus").is_dir()
    assert (base / "loki").is_dir()
    # 6 dashboards, 2 rule files, 1 promtail
    assert len(list((base / "dashboards").glob("*.json"))) == 6
    assert (base / "prometheus" / "recording.yaml").exists()
    assert (base / "prometheus" / "alerts.yaml").exists()
    assert (base / "loki" / "promtail-config.example.yaml").exists()


def test_sync_script_is_documented_in_workflow():
    """The CI workflow must run the sync script + verify no diff."""
    p = _REPO / ".github" / "workflows" / "bench-observability.yml"
    text = p.read_text()
    assert "sync_helm_d13_assets.sh" in text, (
        "bench-observability.yml must run sync_helm_d13_assets.sh "
        "and verify no diff (audit C-2 enforcement)"
    )


# ─────────────────────────────────────────────────────────────────────
# M-1 : register_all_d13() called from init()
# ─────────────────────────────────────────────────────────────────────


def test_telemetry_init_registers_d13_metrics():
    """The 8 D13 instruments must be registered by efe_telemetry::init,
    so they appear in /metrics from the first scrape."""
    text = (_REPO / "crates" / "efe-telemetry" / "src" / "init.rs").read_text()
    assert "register_all_d13" in text, (
        "audit M-1 regression : efe_telemetry::init() no longer calls "
        "register_all_d13(). The 8 D13 metric families won't appear "
        "in /metrics until the first emit-site fires (currently "
        "deferred to D12-bis)."
    )


def test_telemetry_init_calls_d13_after_existing_register_all():
    """The D13 register call must come AFTER the existing
    `instruments::register_all()` so the existing M9 metrics still
    register first (no semantic difference, but audit hygiene)."""
    text = (_REPO / "crates" / "efe-telemetry" / "src" / "init.rs").read_text()
    pos_existing = text.find("instruments::register_all()")
    pos_d13      = text.find("register_all_d13()")
    assert pos_existing != -1
    assert pos_d13 != -1
    assert pos_existing < pos_d13, (
        "audit M-1 hygiene : `register_all_d13()` should be invoked "
        "after the existing `instruments::register_all()` call"
    )


# ─────────────────────────────────────────────────────────────────────
# Cross-cutting : metric label allowlist test was updated
# ─────────────────────────────────────────────────────────────────────


def test_label_allowlist_test_pin_was_updated_for_principal_present():
    """The metric label allowlist test must pin the new 4-label set
    [tenant, action, op_tag, principal_present]. Pre-fix it pinned
    only [tenant, action, op_tag]."""
    text = (_REPO / "scripts" / "tests" /
            "test_d13_metric_label_allowlist.py").read_text()
    assert "principal_present" in text, (
        "test_d13_metric_label_allowlist.py must include "
        "`principal_present` in the EXPECTED tuple for "
        "efe_audit_events_total"
    )

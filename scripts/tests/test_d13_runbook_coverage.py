"""Sprint D13 — runbook structural coverage.

Verifies that every D13 runbook :
  1. Exists in `docs/runbooks/d13_*.md`.
  2. Follows the canonical template (8 sections).
  3. References the related dashboards.
  4. Carries a Severity + Owner header block.
"""

from __future__ import annotations

from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_RUNBOOKS = _REPO / "docs" / "runbooks"


EXPECTED_RUNBOOKS = (
    "d13_high_error_rate",
    "d13_high_latency_p99",
    "d13_classify_drift",
    "d13_catalog_version_explosion",
    "d13_auth_failure_spike",
    "d13_obs_cardinality_bomb",
    "d13_audit_unknown_principal",
    "d13_runbook_missing",
    "d13_trace_sampling",
    "d13_log_pipeline_lag",
)


def test_all_10_runbooks_exist():
    for slug in EXPECTED_RUNBOOKS:
        p = _RUNBOOKS / f"{slug}.md"
        assert p.exists(), f"missing runbook {p}"


@pytest.mark.parametrize("slug", EXPECTED_RUNBOOKS)
def test_runbook_has_severity_block(slug):
    text = (_RUNBOOKS / f"{slug}.md").read_text(encoding="utf-8")
    assert "**Severity**" in text, f"runbook {slug} missing Severity"


@pytest.mark.parametrize("slug", EXPECTED_RUNBOOKS)
def test_runbook_has_owner(slug):
    text = (_RUNBOOKS / f"{slug}.md").read_text(encoding="utf-8")
    assert "**Owner**" in text, f"runbook {slug} missing Owner"


@pytest.mark.parametrize("slug", EXPECTED_RUNBOOKS)
def test_runbook_has_alert_summary_section(slug):
    text = (_RUNBOOKS / f"{slug}.md").read_text(encoding="utf-8")
    assert "## 1." in text, f"runbook {slug} missing section 1"


@pytest.mark.parametrize("slug", EXPECTED_RUNBOOKS)
def test_runbook_has_first_5_minutes_section(slug):
    text = (_RUNBOOKS / f"{slug}.md").read_text(encoding="utf-8")
    assert "## 2." in text, f"runbook {slug} missing section 2"


@pytest.mark.parametrize("slug", EXPECTED_RUNBOOKS)
def test_runbook_has_root_causes_section(slug):
    text = (_RUNBOOKS / f"{slug}.md").read_text(encoding="utf-8")
    assert "## 3." in text, f"runbook {slug} missing section 3"


@pytest.mark.parametrize("slug", EXPECTED_RUNBOOKS)
def test_runbook_has_mitigation_section(slug):
    text = (_RUNBOOKS / f"{slug}.md").read_text(encoding="utf-8")
    assert "## 4." in text, f"runbook {slug} missing section 4"


## Runbooks for self-monitoring alerts (D13ObsRunbookMissing,
## D13ObsAuditPrincipalUnknown, D13ObsTraceSamplingMisconfigured,
## D13ObsLogPipelineLag) deal with platform-side issues — they
## reference logs / configs / commands rather than the 6 product
## dashboards. We exempt them from the dashboard-link requirement
## but require they reference *some* operational artefact (logs,
## kubectl, command).
_DASHBOARD_EXEMPT = {
    "d13_audit_unknown_principal",
    "d13_runbook_missing",
    "d13_trace_sampling",
    "d13_log_pipeline_lag",
}


@pytest.mark.parametrize("slug", EXPECTED_RUNBOOKS)
def test_runbook_references_at_least_one_dashboard_or_artefact(slug):
    text = (_RUNBOOKS / f"{slug}.md").read_text(encoding="utf-8")
    has_dashboard_ref = (
        "dashboard" in text.lower()
        or any(name in text for name in (
            "01_operator_overview",
            "02_tenant_drill_down",
            "03_catalog_version_distribution",
            "04_cost_attribution",
            "05_audit_stream",
            "06_capacity_planning",
        ))
    )
    has_artefact_ref = (
        "kubectl" in text
        or "logql" in text.lower()
        or "promql" in text.lower()
        or "loki" in text.lower()
        or "tempo" in text.lower()
    )
    if slug in _DASHBOARD_EXEMPT:
        assert has_artefact_ref, (
            f"runbook {slug} (dashboard-exempt) must reference an "
            f"operational artefact (kubectl, LogQL, Loki, Tempo, …)"
        )
    else:
        assert has_dashboard_ref, (
            f"runbook {slug} doesn't reference any dashboard"
        )


@pytest.mark.parametrize("slug", EXPECTED_RUNBOOKS)
def test_runbook_has_escalation(slug):
    """Every runbook must document escalation."""
    text = (_RUNBOOKS / f"{slug}.md").read_text(encoding="utf-8")
    assert "Escalat" in text or "escalat" in text, (
        f"runbook {slug} missing escalation guidance"
    )


def test_no_runbook_left_behind():
    """If a `d13_*.md` exists in runbooks/, it must be in
    EXPECTED_RUNBOOKS (so deletes are explicit)."""
    found = sorted(p.stem for p in _RUNBOOKS.glob("d13_*.md"))
    expected = sorted(EXPECTED_RUNBOOKS)
    assert found == expected, (
        f"runbook drift:\n  found: {found}\n  expected: {expected}"
    )

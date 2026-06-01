"""Sprint D13 — runbook URL coverage on every alert.

The architecture (§ 10) makes ``runbook_url`` annotation MANDATORY
on every D13-OBS alert. This test enforces it AND verifies that
every referenced runbook file exists in `docs/runbooks/`.

A new alert without a runbook = build red.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_ALERTS_PATH = _REPO / "deploy" / "observability" / "prometheus" / "rules.d13" / "alerts.yaml"
_RUNBOOKS_DIR = _REPO / "docs" / "runbooks"


def _load_alerts():
    """Parse the alerts YAML without YAML lib (avoid dev dep). The
    file is hand-formatted ; we use a simple regex extractor."""
    text = _ALERTS_PATH.read_text(encoding="utf-8")
    # Each alert : `- alert: <name>` followed by metadata. We harvest
    # the alert names + runbook_url annotations.
    alerts: list[dict[str, str]] = []
    cur: dict[str, str] = {}
    for raw in text.splitlines():
        line = raw.rstrip()
        m = re.match(r"\s*- alert:\s*(\S+)", line)
        if m:
            if cur:
                alerts.append(cur)
            cur = {"alert": m.group(1)}
            continue
        m = re.match(r"\s*runbook_url:\s*(\S+)", line)
        if m and cur is not None:
            cur["runbook_url"] = m.group(1)
        m = re.match(r"\s*for:\s*(\S+)", line)
        if m and cur is not None and "for" not in cur:
            cur["for"] = m.group(1)
        m = re.match(r"\s*severity:\s*(\S+)", line)
        if m and cur is not None and "severity" not in cur:
            cur["severity"] = m.group(1)
    if cur:
        alerts.append(cur)
    return alerts


# ─────────────────────────────────────────────────────────────────────
# 1. Alert inventory
# ─────────────────────────────────────────────────────────────────────

EXPECTED_ALERTS = (
    "D13ObsHighErrorRate",
    "D13ObsHighLatencyP99",
    "D13ObsClassifyDriftHigh",
    "D13ObsCatalogVersionExplosion",
    "D13ObsAuthFailureSpike",
    "D13ObsCardinalityBomb",
    "D13ObsAuditPrincipalUnknown",
    "D13ObsTraceSamplingMisconfigured",
    "D13ObsLogPipelineLag",
    "D13ObsRunbookMissing",
)


def test_all_10_alerts_present():
    alerts = _load_alerts()
    names = {a["alert"] for a in alerts}
    missing = set(EXPECTED_ALERTS) - names
    assert not missing, f"missing D13 alerts: {missing}"


def test_no_unannounced_alerts():
    alerts = _load_alerts()
    names = {a["alert"] for a in alerts}
    extras = names - set(EXPECTED_ALERTS)
    assert not extras, (
        f"D13 alerts added without test update: {extras}. "
        f"Update EXPECTED_ALERTS in this file + write the runbook."
    )


# ─────────────────────────────────────────────────────────────────────
# 2. Each alert has a runbook_url
# ─────────────────────────────────────────────────────────────────────

@pytest.mark.parametrize("alert_name", EXPECTED_ALERTS)
def test_each_alert_has_runbook_url(alert_name):
    alerts = _load_alerts()
    found = next(a for a in alerts if a["alert"] == alert_name)
    assert "runbook_url" in found, (
        f"alert {alert_name} is missing the runbook_url annotation. "
        f"This is REQUIRED by D13 § 10. Add it AND write the runbook."
    )
    url = found["runbook_url"]
    assert url, f"alert {alert_name} runbook_url is empty"
    # Must point to docs.efe.ai/runbooks/<file>.
    assert "runbooks" in url, (
        f"alert {alert_name} runbook_url {url!r} doesn't reference "
        f"the runbooks directory"
    )


# ─────────────────────────────────────────────────────────────────────
# 3. Runbook files exist
# ─────────────────────────────────────────────────────────────────────

@pytest.mark.parametrize("alert_name", EXPECTED_ALERTS)
def test_runbook_file_exists_for_alert(alert_name):
    alerts = _load_alerts()
    found = next(a for a in alerts if a["alert"] == alert_name)
    url = found.get("runbook_url", "")
    # Extract the file slug from the URL.
    # Example : https://docs.efe.ai/runbooks/d13_high_error_rate
    m = re.search(r"runbooks/(\w+)", url)
    assert m, f"cannot extract runbook slug from URL {url!r}"
    slug = m.group(1)
    runbook = _RUNBOOKS_DIR / f"{slug}.md"
    assert runbook.exists(), (
        f"alert {alert_name} references runbook {slug}.md but the "
        f"file doesn't exist at {runbook}"
    )


# ─────────────────────────────────────────────────────────────────────
# 4. for clause minimum (5m) — D13 § 10.5 alert hygiene
# ─────────────────────────────────────────────────────────────────────

@pytest.mark.parametrize("alert_name", EXPECTED_ALERTS)
def test_each_alert_has_for_clause_at_least_5m(alert_name):
    alerts = _load_alerts()
    found = next(a for a in alerts if a["alert"] == alert_name)
    for_clause = found.get("for", "")
    # Format : "5m", "10m", "15m". Extract minutes.
    m = re.match(r"(\d+)m$", for_clause)
    if m:
        minutes = int(m.group(1))
    elif for_clause.endswith("h"):
        minutes = int(for_clause[:-1]) * 60
    else:
        pytest.fail(f"alert {alert_name} has unexpected for: {for_clause!r}")
    assert minutes >= 5, (
        f"alert {alert_name} has for: {for_clause} (< 5m). "
        f"Risk of flapping ; raise to ≥ 5m per D13 § 10.5."
    )


# ─────────────────────────────────────────────────────────────────────
# 5. severity is in the canonical 3-tier set
# ─────────────────────────────────────────────────────────────────────

@pytest.mark.parametrize("alert_name", EXPECTED_ALERTS)
def test_each_alert_has_canonical_severity(alert_name):
    alerts = _load_alerts()
    found = next(a for a in alerts if a["alert"] == alert_name)
    sev = found.get("severity", "")
    assert sev in ("critical", "warning", "info"), (
        f"alert {alert_name} severity {sev!r} not in canonical set "
        f"(critical / warning / info)"
    )


# ─────────────────────────────────────────────────────────────────────
# 6. runbook structure check (each runbook has the expected sections)
# ─────────────────────────────────────────────────────────────────────

EXPECTED_RUNBOOK_SECTIONS = (
    "## 1.",  # Alert summary
    "## 2.",  # First 5 minutes
    "## 3.",  # Common root causes
    "## 4.",  # Mitigation
)


def test_runbooks_follow_template():
    for runbook in _RUNBOOKS_DIR.glob("d13_*.md"):
        text = runbook.read_text(encoding="utf-8")
        for section in EXPECTED_RUNBOOK_SECTIONS:
            assert section in text, (
                f"runbook {runbook.name} missing section starting "
                f"with {section!r}"
            )

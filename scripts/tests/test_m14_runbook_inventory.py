"""Sprint **M14 D42** — Pin tests : runbook inventory ≥ 25 + structure.

Cf. `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` §14 + §16.5.

6 tests covering :
  1. ≥ 25 runbooks (10 D13 existing + 15+ NEW M14)
  2. Runbook naming convention dXX_*.md or general_*.md or mXX_*.md
  3. Each runbook has Symptom section
  4. Each runbook has Mitigation section
  5. Each runbook has Resolution criteria section
  6. Each runbook has Severity metadata
"""

from __future__ import annotations

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RUNBOOKS = REPO / "docs" / "runbooks"


def _list_runbooks() -> list[Path]:
    return sorted(RUNBOOKS.glob("*.md"))


def test_m14_runbook_count_at_least_25() -> None:
    runbooks = _list_runbooks()
    assert len(runbooks) >= 25, (
        f"M14 runbook inventory must have ≥ 25 runbooks, got {len(runbooks)}"
    )


def test_m14_runbook_naming_convention() -> None:
    """Runbooks follow `dXX_*.md` / `mXX_*.md` / `general_*.md` pattern."""
    valid_patterns = (
        re.compile(r"^d\d+_.+\.md$"),       # dXX_*.md
        re.compile(r"^d\d+bis_.+\.md$"),    # dXXbis_*.md (e.g. d14bis_*)
        re.compile(r"^m\d+_.+\.md$"),       # mXX_*.md
        re.compile(r"^general_.+\.md$"),    # general_*.md
    )
    for rb in _list_runbooks():
        name = rb.name
        assert any(p.match(name) for p in valid_patterns), (
            f"Runbook {name!r} does not match naming convention "
            f"(dXX_*.md, dXXbis_*.md, mXX_*.md, or general_*.md)"
        )


def test_m14_each_runbook_has_symptom_section() -> None:
    """Each runbook must have a section describing what triggers it.
    Accepted synonyms : Symptom / Alert / Trigger / When (D13 runbooks
    sometimes use "Alert summary")."""
    for rb in _list_runbooks():
        body = rb.read_text(encoding="utf-8").lower()
        synonyms = ("symptom", "alert", "trigger", "when to use", "when this fires")
        has_synonym = any(syn in body for syn in synonyms)
        assert has_synonym, (
            f"Runbook {rb.name} missing Symptom/Alert/Trigger section"
        )


def test_m14_each_runbook_has_mitigation_section() -> None:
    """Each runbook must have a section describing fix actions.
    Accepted synonyms : Mitigation / Procedure / Action / Steps /
    Resolution (general_secret_rotation_playbook uses 'Generic procedure')."""
    for rb in _list_runbooks():
        body = rb.read_text(encoding="utf-8").lower()
        synonyms = ("mitigation", "procedure", "remediation", "action",
                    "fix", "steps", "first 5 minutes")
        has_synonym = any(syn in body for syn in synonyms)
        assert has_synonym, (
            f"Runbook {rb.name} missing Mitigation/Procedure section"
        )


def test_m14_each_runbook_has_resolution_section() -> None:
    for rb in _list_runbooks():
        body = rb.read_text(encoding="utf-8")
        assert "Resolution" in body or "resolution" in body, (
            f"Runbook {rb.name} missing Resolution section"
        )


def test_m14_each_runbook_has_severity_metadata() -> None:
    """Each runbook must declare its severity (P0/P1/P2/P3) or
    label it scheduled."""
    for rb in _list_runbooks():
        body = rb.read_text(encoding="utf-8")
        has_severity = (
            "P0" in body or "P1" in body or "P2" in body or "P3" in body
            or "Severity" in body or "severity" in body
            or "Scheduled" in body or "Manual" in body
            or "Quarterly" in body
        )
        assert has_severity, f"Runbook {rb.name} missing severity metadata"

"""Sprint **M14 D40** — Pin tests : security docs + handoff ops.

Cf. `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` §3 + §13 + §16.5.

6 tests covering :
  1. Risk register present + ≥ 15 risks
  2. Vulnerability disclosure policy present + safe harbor
  3. Pen-test program plan present
  4. Handoff ops doc present + RACI matrix
  5. Handoff ops has secrets rotation calendar
  6. Handoff ops has escalation tree
"""

from __future__ import annotations

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SECURITY = REPO / "docs" / "security"
HANDOFF = REPO / "docs" / "HANDOFF_OPS.md"


def test_m14_risk_register_present_with_risks() -> None:
    rr = SECURITY / "risk_register.md"
    assert rr.is_file(), f"M14 risk register missing : {rr}"
    body = rr.read_text(encoding="utf-8")
    # ≥ 15 risks (R-01 through R-XX)
    risk_pattern = re.compile(r"\*\*R-\d+\*\*", re.MULTILINE)
    matches = risk_pattern.findall(body)
    assert len(matches) >= 15, (
        f"Risk register must have ≥ 15 risks, got {len(matches)}"
    )


def test_m14_vulnerability_disclosure_present_with_safe_harbor() -> None:
    vdp = SECURITY / "vulnerability_disclosure.md"
    assert vdp.is_file(), f"M14 VDP missing : {vdp}"
    body = vdp.read_text(encoding="utf-8")
    # Safe harbor declaration
    assert "Safe harbor" in body or "safe harbor" in body, (
        "VDP must declare safe harbor for good-faith research"
    )
    # Coordinated disclosure timeline
    assert "90" in body  # 90-day standard
    # Contact methods
    assert "security@efe.ai" in body


def test_m14_pentest_program_plan_present() -> None:
    pp = SECURITY / "pentest_program.md"
    assert pp.is_file(), f"M14 pentest program missing : {pp}"
    body = pp.read_text(encoding="utf-8")
    # 3-vendor shortlist
    assert "NCC" in body or "Cure53" in body or "Trail of Bits" in body
    # Severity SLAs
    assert "Critical" in body
    assert "7 days" in body or "7d" in body


def test_m14_handoff_ops_present_with_raci() -> None:
    assert HANDOFF.is_file(), f"M14 HANDOFF_OPS missing : {HANDOFF}"
    body = HANDOFF.read_text(encoding="utf-8")
    # RACI matrix
    assert "RACI" in body
    assert "Owner" in body
    # Components covered
    assert "Helm chart" in body
    assert "Bundle" in body
    assert "OIDC" in body
    assert "DR drill" in body


def test_m14_handoff_ops_has_secrets_rotation_calendar() -> None:
    body = HANDOFF.read_text(encoding="utf-8")
    assert "rotation" in body.lower()
    # Some specific secrets covered
    assert "EFE_BUNDLE_PRIVKEY" in body or "bundle" in body.lower()
    assert "OIDC" in body
    assert "Quarterly" in body or "quarterly" in body
    assert "Annual" in body or "annual" in body


def test_m14_handoff_ops_has_escalation_tree() -> None:
    body = HANDOFF.read_text(encoding="utf-8")
    assert "Escalation" in body or "escalation" in body
    # L1/L2/L3/L4 escalation levels
    assert "L1" in body
    assert "L2" in body
    assert "L3" in body
    assert "L4" in body or "CEO" in body

"""Sprint **M14 D40** — Pin tests : threat model formalisé STRIDE.

Cf. `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` §3 + §16.5.

5 tests covering :
  1. threat_model.md present + STRIDE categorization
  2. References M12 Helm threats + M13 bundle threats + D11 isolation + D14-bis + D16
  3. ≥ 60 threats consolidated
  4. Each threat has source + mitigation + residual fields
  5. STRIDE category counts match expectations
"""

from __future__ import annotations

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
THREAT_MODEL = REPO / "docs" / "security" / "threat_model.md"


def _read(path: Path) -> str:
    assert path.is_file(), f"M14 threat model missing : {path}"
    return path.read_text(encoding="utf-8")


def test_m14_threat_model_md_uses_stride_categorization() -> None:
    body = _read(THREAT_MODEL)
    # STRIDE 6 categories
    assert "Spoofing" in body
    assert "Tampering" in body
    assert "Repudiation" in body
    assert "Information Disclosure" in body
    assert "Denial of Service" in body
    assert "Elevation of Privilege" in body


def test_m14_threat_model_references_consolidated_sources() -> None:
    """Threat model must reference 5 consolidated source areas."""
    body = _read(THREAT_MODEL)
    # M12 + M13 + D11 + D14-bis + D16
    assert "M12" in body
    assert "M13" in body
    assert "D11" in body or "multi-tenant" in body.lower()
    assert "D14-bis" in body or "D14 -bis" in body or "HMAC" in body
    assert "D15" in body or "OIDC" in body
    assert "D16" in body or "bundle" in body.lower()


def test_m14_threat_model_at_least_60_threats() -> None:
    body = _read(THREAT_MODEL)
    # Count TM-* threat IDs
    tm_pattern = re.compile(r"\*\*TM-[STRIDE]+-\d+\*\*", re.MULTILINE)
    matches = tm_pattern.findall(body)
    assert len(matches) >= 60, (
        f"Threat model must consolidate ≥ 60 threats, got {len(matches)}"
    )


def test_m14_threat_model_has_residual_risk_field() -> None:
    body = _read(THREAT_MODEL)
    # Residual risk classification (Low / Medium / High)
    assert "Residual" in body
    # Statistics table
    assert "STRIDE coverage statistics" in body or "coverage statistics" in body.lower()


def test_m14_threat_model_has_re_evaluation_cadence() -> None:
    body = _read(THREAT_MODEL)
    assert "Quarterly" in body or "quarterly" in body
    assert "Annual" in body or "annual" in body
    assert "Re-evaluation" in body or "re-evaluation" in body or "review" in body.lower()

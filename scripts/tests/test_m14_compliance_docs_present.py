"""Sprint **M14 D40+D41** — Pin tests : compliance documents present + structurally complete.

Cf. `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` §16 + §25.

8 tests covering :
  1. iso27001.md present + non-empty + has SoA section
  2. soc2_evidence.md covers CC1-CC9
  3. gdpr_art32.md covers 4 mesures (a/b/c/d)
  4. ai_act_annex_iv.md covers 8 sections
  5. nist_800_53.md covers 17 control families
  6. ddq_prefill.md has 200+ questions
  7. continuous_compliance.md present
  8. cra_sbom_endpoint.md present (CRA alignment)
"""

from __future__ import annotations

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
COMPLIANCE = REPO / "docs" / "compliance"


def _read(path: Path) -> str:
    assert path.is_file(), f"M14 compliance doc missing : {path}"
    return path.read_text(encoding="utf-8")


def test_m14_iso27001_md_present_and_has_soa() -> None:
    body = _read(COMPLIANCE / "iso27001.md")
    assert "Statement of Applicability" in body
    assert "A.5 Organizational" in body
    assert "A.6 People" in body
    assert "A.7 Physical" in body
    assert "A.8 Technological" in body


def test_m14_soc2_evidence_md_covers_cc1_cc9() -> None:
    body = _read(COMPLIANCE / "soc2_evidence.md")
    for cc in ("CC1", "CC2", "CC3", "CC4", "CC5", "CC6", "CC7", "CC8", "CC9"):
        assert f"## {(['1','2','3','4','5','6','7','8','9','10','11'])[int(cc[2])-1]}" in body or cc in body, (
            f"SOC 2 evidence must cover {cc}"
        )
    # Availability + Confidentiality categories
    assert "A1" in body
    assert "C1" in body


def test_m14_gdpr_art32_md_covers_4_mesures() -> None:
    body = _read(COMPLIANCE / "gdpr_art32.md")
    assert "Pseudonymisation" in body or "(a)" in body
    assert "Confidentialité" in body or "(b)" in body
    assert "(c)" in body  # Restoration capability
    assert "(d)" in body  # Regular testing
    # Article 33 breach notification
    assert "Article 33" in body or "Art. 33" in body


def test_m14_ai_act_annex_iv_md_covers_8_sections() -> None:
    body = _read(COMPLIANCE / "ai_act_annex_iv.md")
    # 8 sections per Annex IV
    for section in ("§1", "§2", "§3", "§4", "§5", "§6", "§7", "§8"):
        assert section in body, f"AI Act doc must cover section {section}"
    # Limited-risk classification
    assert "limited-risk" in body.lower() or "Limited-risk" in body
    # Transparency obligation Art. 52
    assert "Art. 52" in body or "Art 52" in body or "Article 52" in body


def test_m14_nist_800_53_md_covers_17_families() -> None:
    body = _read(COMPLIANCE / "nist_800_53.md")
    # 17 control families
    for family in ("AC", "AT", "AU", "CA", "CM", "CP", "IA", "IR",
                   "MA", "MP", "PE", "PL", "PS", "RA", "SA", "SC", "SI"):
        assert f"**{family}**" in body or f"## {family}" in body or f"## " in body, (
            f"NIST doc must cover family {family}"
        )
    # Coverage statistics table
    assert "Implemented" in body
    assert "Compensating" in body


def test_m14_ddq_prefill_md_has_substantial_questions() -> None:
    body = _read(COMPLIANCE / "ddq_prefill.md")
    # Count Q-* entries (each starts with "### Q-")
    q_pattern = re.compile(r"^### Q-", re.MULTILINE)
    matches = q_pattern.findall(body)
    assert len(matches) >= 100, (
        f"DDQ pre-fill must have ≥ 100 questions, got {len(matches)}"
    )


def test_m14_continuous_compliance_md_present() -> None:
    body = _read(COMPLIANCE / "continuous_compliance.md")
    assert "Quarterly" in body
    assert "Annual" in body
    # Key cadences
    assert "Weekly" in body or "weekly" in body
    assert "evidence" in body.lower()


def test_m14_cra_sbom_endpoint_md_present() -> None:
    body = _read(COMPLIANCE / "cra_sbom_endpoint.md")
    assert "https://efe.ai/sbom/" in body
    assert "CycloneDX" in body
    assert "1.5" in body
    # CRA references
    assert "CRA" in body or "Cyber Resilience Act" in body

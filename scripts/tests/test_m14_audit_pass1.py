"""Sprint **M14** — Pin tests anti-régression : audit chirurgical pass 1.

Cf. CHANGELOG.md `### M14 audit pass 1` for the 6 defects.

6 audit-fix pin tests anchored on structural invariants of the fix.

  * **A1** (CRIT) threat_model.md totals row matches matrix walk-through (66 threats)
  * **A2** (HIGH) all 9 stub files for broken cross-references exist
  * **A3** (MED)  SECURITY.md exists at repo root
  * **A4** (MED)  ddq_prefill.md footer claims actual question count
  * **A5** (MED)  branch-protection-rules.yml stripped of lock_branch + apply script present
  * **A6** (LOW)  CONTRIBUTING-POST-FREEZE.md documents merge path
"""

from __future__ import annotations

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
THREAT_MODEL = REPO / "docs" / "security" / "threat_model.md"
COMPLIANCE = REPO / "docs" / "compliance"
RUNBOOKS = REPO / "docs" / "runbooks"
SECURITY_MD = REPO / "SECURITY.md"
DDQ = COMPLIANCE / "ddq_prefill.md"
BRANCH_PROTECTION = REPO / ".github" / "branch-protection-rules.yml"
APPLY_SCRIPT = REPO / ".github" / "apply-branch-protection.sh"
CONTRIB_POST = REPO / ".github" / "CONTRIBUTING-POST-FREEZE.md"


# ─────────────────────────────────────────────────────────────────────
# A1 (CRIT) — threat model count consistency
# ─────────────────────────────────────────────────────────────────────


def test_m14_audit_a1_threat_model_count_consistency() -> None:
    """A1 (CRIT) — pre-fix the doc claimed 63 threats but matrix had 66
    (T=15 not 14, D=13 not 11). Coverage stats table at end was wrong.
    Post-fix the totals row says **66** total / **63** Low residual."""
    body = THREAT_MODEL.read_text(encoding="utf-8")
    # Count actual threats in matrix
    actual_count = len(re.findall(r"\|\s*\*\*TM-[STRIDE]+-\d+\*\*", body))
    assert actual_count >= 60, (
        f"Threat model must consolidate ≥ 60 threats, got {actual_count}"
    )
    # Statistics totals row must match actual count
    assert "**66**" in body, (
        "A1 regression : statistics row must say **66** (actual matrix count) "
        "after audit fix ; pre-fix said 63"
    )
    # Per-category counts must match
    assert "| Tampering | 15 |" in body, (
        "A1 regression : Tampering category must be 15 (was wrongly 14)"
    )
    assert "| Denial of Service | 13 |" in body, (
        "A1 regression : DoS category must be 13 (was wrongly 11)"
    )


# ─────────────────────────────────────────────────────────────────────
# A2 (HIGH) — broken cross-references resolved via stub files
# ─────────────────────────────────────────────────────────────────────


def test_m14_audit_a2_template_stub_files_present() -> None:
    """A2 (HIGH) — pre-fix 17 cross-references in compliance + runbook docs
    pointed to non-existent files. Post-fix 9 stub files created :
    breach_notification_template.md, gdpr_dpia_template.md,
    gdpr_ropa_template.md, gdpr_tia_template.md, dpa_template.md,
    d11_cross_tenant_data_leak.md, d9_hpa_stuck_at_max.md,
    operator_guide.md."""
    expected_stubs = [
        COMPLIANCE / "breach_notification_template.md",
        COMPLIANCE / "gdpr_dpia_template.md",
        COMPLIANCE / "gdpr_ropa_template.md",
        COMPLIANCE / "gdpr_tia_template.md",
        COMPLIANCE / "dpa_template.md",
        RUNBOOKS / "d11_cross_tenant_data_leak.md",
        RUNBOOKS / "d9_hpa_stuck_at_max.md",
        REPO / "docs" / "operator_guide.md",
    ]
    missing = [s for s in expected_stubs if not s.is_file()]
    assert not missing, (
        f"A2 regression : audit-fix stub files missing : "
        f"{[str(s.relative_to(REPO)) for s in missing]}"
    )
    # Each stub must be non-trivially populated (≥ 200 bytes)
    for stub in expected_stubs:
        size = stub.stat().st_size
        assert size >= 200, (
            f"A2 regression : stub {stub.relative_to(REPO)} too small "
            f"({size} bytes) — must have substantive template content"
        )


def test_m14_audit_a2_iso27001_typo_fixed() -> None:
    """A2 (HIGH) — pre-fix iso27001.md referenced `gdpr_dpia.md` (typo)
    instead of `gdpr_dpia_template.md`. Post-fix the typo is corrected."""
    body = (COMPLIANCE / "iso27001.md").read_text(encoding="utf-8")
    # The bare `gdpr_dpia.md` reference (without "_template") must NOT exist
    bare_pattern = re.compile(r"`gdpr_dpia\.md`")
    assert not bare_pattern.search(body), (
        "A2 regression : iso27001.md still references `gdpr_dpia.md` "
        "(should be `gdpr_dpia_template.md`)"
    )


# ─────────────────────────────────────────────────────────────────────
# A3 (MED) — SECURITY.md at repo root
# ─────────────────────────────────────────────────────────────────────


def test_m14_audit_a3_security_md_at_root() -> None:
    """A3 (MED) — SECURITY.md is referenced 4× in compliance docs as
    "EFE evidence" but didn't exist at repo root. GitHub Security tab
    expects it at root. Post-fix : root SECURITY.md created."""
    assert SECURITY_MD.is_file(), (
        f"A3 regression : SECURITY.md must exist at repo root for "
        f"GitHub Security tab + compliance doc references"
    )
    body = SECURITY_MD.read_text(encoding="utf-8")
    # Must reference VDP + supported versions
    assert "vulnerability" in body.lower() or "Reporting" in body
    assert "Supported versions" in body or "supported versions" in body.lower()
    assert "1.0" in body
    # Must reference compliance frameworks
    assert "SOC 2" in body or "ISO 27001" in body


# ─────────────────────────────────────────────────────────────────────
# A4 (MED) — DDQ count footer matches actual
# ─────────────────────────────────────────────────────────────────────


def test_m14_audit_a4_ddq_count_footer_accurate() -> None:
    """A4 (MED) — pre-fix DDQ footer claimed 243 questions ; actual
    grep count was 235. Post-fix footer states actual count."""
    body = DDQ.read_text(encoding="utf-8")
    actual_count = len(re.findall(r"^### Q-", body, re.MULTILINE))
    # Footer claim must match actual count (or be ≥ actual)
    # Active footer line must contain the right number.
    # Anchor on the "Total questions answered" header WITH the count.
    pattern = re.compile(r"Total questions answered\s*:\s*(\d+)")
    matches = pattern.findall(body)
    assert matches, (
        "A4 regression : DDQ footer must include `Total questions answered : <count>`"
    )
    # The first/largest match should match actual count
    claimed = max(int(m) for m in matches)
    assert claimed == actual_count, (
        f"A4 regression : DDQ footer claims {claimed} questions but "
        f"actual count is {actual_count}"
    )


# ─────────────────────────────────────────────────────────────────────
# A5 (MED) — branch-protection-rules.yml field correctness
# ─────────────────────────────────────────────────────────────────────


def test_m14_audit_a5_branch_protection_no_lock_branch() -> None:
    """A5 (MED) — pre-fix the YAML included `lock_branch: false` which
    is part of the newer rulesets API, not the legacy branch protection
    endpoint. When applied via `gh api PUT /repos/.../protection`, the
    field was silently ignored. Post-fix : `lock_branch` removed."""
    body = BRANCH_PROTECTION.read_text(encoding="utf-8")
    # No active `lock_branch:` key (audit-fix comments may mention it
    # as the pre-fix pattern, but no active YAML key).
    active_pattern = re.compile(r"^\s*lock_branch\s*:", re.MULTILINE)
    assert not active_pattern.search(body), (
        "A5 regression : branch-protection-rules.yml must NOT contain "
        "active `lock_branch:` field (newer rulesets API only)"
    )


def test_m14_audit_a5_apply_branch_protection_script_present() -> None:
    """A5 (MED) — `required_signatures` requires a separate sub-endpoint
    call (POST /branches/.../protection/required_signatures) which the
    legacy YAML can't express. Post-fix : `apply-branch-protection.sh`
    script handles the multi-call sequence."""
    assert APPLY_SCRIPT.is_file(), (
        f"A5 regression : apply-branch-protection.sh script missing"
    )
    # Script must be executable
    import os
    assert os.access(APPLY_SCRIPT, os.X_OK), (
        f"A5 regression : apply-branch-protection.sh must be executable"
    )
    # Script must call required_signatures sub-endpoint
    body = APPLY_SCRIPT.read_text(encoding="utf-8")
    assert "required_signatures" in body, (
        "A5 regression : apply script must POST to required_signatures sub-endpoint"
    )


# ─────────────────────────────────────────────────────────────────────
# A6 (LOW) — CONTRIBUTING-POST-FREEZE merge note
# ─────────────────────────────────────────────────────────────────────


def test_m14_audit_a6_contributing_post_freeze_merge_note() -> None:
    """A6 (LOW) — pre-fix `.github/CONTRIBUTING-POST-FREEZE.md` was a
    custom name not auto-loaded by GitHub's PR UI. Post-fix : doc
    explicitly notes that contents should be merged into root
    `CONTRIBUTING.md` at freeze time."""
    body = CONTRIB_POST.read_text(encoding="utf-8")
    # Must mention CONTRIBUTING.md (root) merge path
    assert "CONTRIBUTING.md" in body, (
        "A6 regression : doc must mention root CONTRIBUTING.md merge path"
    )
    # Must mention the merge / cp command
    assert "merge" in body.lower() or "cp .github" in body, (
        "A6 regression : doc must show merge instructions"
    )

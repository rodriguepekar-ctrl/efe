"""Sprint **M14 D42** — Pin tests : freeze procedure + branch protection.

Cf. `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` §15 + §25.

3 tests covering :
  1. branch-protection-rules.yml present
  2. Requires 2 reviewers + signed commits + status checks
  3. CONTRIBUTING-POST-FREEZE.md documents allowed PR types
"""

from __future__ import annotations

from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[2]
BRANCH_PROTECTION = REPO / ".github" / "branch-protection-rules.yml"
CONTRIB_POST_FREEZE = REPO / ".github" / "CONTRIBUTING-POST-FREEZE.md"
HANDOFF_OPS = REPO / "docs" / "HANDOFF_OPS.md"


def test_m14_branch_protection_yml_present() -> None:
    assert BRANCH_PROTECTION.is_file(), (
        f"M14 freeze branch-protection-rules.yml missing : {BRANCH_PROTECTION}"
    )
    doc = yaml.safe_load(BRANCH_PROTECTION.read_text(encoding="utf-8"))
    assert "main" in doc, "Branch protection must declare main branch rules"


def test_m14_branch_protection_requires_2_reviewers_and_signed() -> None:
    doc = yaml.safe_load(BRANCH_PROTECTION.read_text(encoding="utf-8"))
    main = doc["main"]
    # 2-reviewer rule
    reviews = main.get("required_pull_request_reviews", {})
    assert reviews.get("required_approving_review_count") == 2, (
        "Branch protection must require 2 reviewers post-freeze"
    )
    assert reviews.get("require_code_owner_reviews") is True
    # Signed commits
    assert main.get("required_signatures") is True, (
        "Branch protection must require signed commits"
    )
    # Status checks
    status = main.get("required_status_checks", {})
    contexts = status.get("contexts", [])
    assert len(contexts) >= 5, (
        "Branch protection must require ≥ 5 status checks "
        "(build / test / polaris / pin-tests / sbom-validate / license-gate / etc.)"
    )


def test_m14_post_freeze_contributing_documents_allowed_pr_types() -> None:
    assert CONTRIB_POST_FREEZE.is_file(), (
        f"M14 freeze CONTRIBUTING-POST-FREEZE.md missing"
    )
    body = CONTRIB_POST_FREEZE.read_text(encoding="utf-8")
    # Allowed PR types
    assert "Security" in body
    assert "Critical" in body or "bug-fix" in body.lower() or "bug fix" in body.lower()
    assert "Compliance" in body
    assert "Documentation" in body or "docs" in body.lower()
    # FROZEN marker
    assert "FROZEN" in body or "frozen" in body.lower()
    # Refused types
    assert ("not accepted" in body.lower() or "REFUSED" in body
            or "closed without merge" in body.lower())

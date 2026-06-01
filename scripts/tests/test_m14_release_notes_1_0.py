"""Sprint **M14 D42** — Pin tests : release notes 1.0 GA-ready.

Cf. `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` §12 + §24.

3 tests covering :
  1. RELEASE_NOTES_1.0.md present
  2. Has breaking changes section
  3. Has migration guide + compatibility matrix + LTS policy
"""

from __future__ import annotations

from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RELEASE_NOTES = REPO / "docs" / "RELEASE_NOTES_1.0.md"


def test_m14_release_notes_1_0_md_present() -> None:
    assert RELEASE_NOTES.is_file(), f"M14 release notes missing : {RELEASE_NOTES}"
    body = RELEASE_NOTES.read_text(encoding="utf-8")
    assert "EFE 1.0" in body
    assert "GA" in body or "General Availability" in body


def test_m14_release_notes_has_breaking_changes_section() -> None:
    body = RELEASE_NOTES.read_text(encoding="utf-8")
    assert "Breaking changes" in body or "breaking changes" in body.lower()
    # At least 3 breaking changes documented (B-1, B-2, B-3, etc.)
    assert "B-1" in body
    assert "B-2" in body
    assert "B-3" in body


def test_m14_release_notes_has_migration_and_lts() -> None:
    body = RELEASE_NOTES.read_text(encoding="utf-8")
    # Migration guide
    assert "Migration" in body
    assert "0.x" in body
    assert "1.0" in body
    # Compatibility matrix
    assert "Compatibility matrix" in body or "compatibility matrix" in body.lower()
    assert "Kubernetes" in body
    assert "Postgres" in body
    # LTS policy
    assert "LTS" in body
    assert "2031" in body  # 5-year LTS through 2031-12-31

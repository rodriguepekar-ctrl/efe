"""Tests for PII scrubbing."""

from __future__ import annotations

import pytest

from efe_ft.errors import PiiBlocked
from efe_ft.ingest.pii import find_pii, scrub


def test_no_pii_passthrough() -> None:
    s, k = scrub("Hello, nothing sensitive here.")
    assert s == "Hello, nothing sensitive here."
    assert k == []


def test_email_detected() -> None:
    s, k = scrub("Contact alice@acme.fr please.")
    assert "<EMAIL>" in s
    assert "alice@acme.fr" not in s
    assert "email" in k


def test_phone_fr_detected_national_format() -> None:
    s, k = scrub("Appelle-moi au 06 12 34 56 78 demain.")
    assert "<PHONE_FR>" in s
    assert "06 12 34 56 78" not in s
    assert "phone_fr" in k


def test_iban_detected() -> None:
    s, k = scrub("IBAN: FR76 3000 4000 0300 1234 5678 901")
    assert "<IBAN>" in s
    assert "iban" in k


def test_ssn_fr_detected() -> None:
    s, k = scrub("NIR: 1 85 12 75 108 111 42")
    assert "<SSN_FR>" in s
    assert "ssn_fr" in k


def test_reject_policy_raises() -> None:
    with pytest.raises(PiiBlocked) as exc:
        scrub("hello alice@acme.fr", policy="reject")
    assert "email" in exc.value.detail["kinds"]


def test_multiple_kinds_sorted_distinct() -> None:
    text = "alice@acme.fr +33 6 12 34 56 78 and 06 12 34 56 78"
    _, kinds = scrub(text)
    assert "email" in kinds
    # Only distinct kinds once, sorted
    assert sorted(kinds) == kinds
    assert len(kinds) == len(set(kinds))


def test_overlap_priority() -> None:
    """Email comes before phone in priority — never double-claimed."""
    text = "ping me alice@acme.fr — 06 01 02 03 04"
    hits = find_pii(text)
    # No hit should overlap
    for i, h1 in enumerate(hits):
        for h2 in hits[i + 1 :]:
            assert h1.end <= h2.start


def test_scrub_preserves_non_pii() -> None:
    text = "order 12345 received on 2025-01-03"
    s, k = scrub(text)
    assert "order 12345 received on" in s
    # The trailing date could match CC regex if 13+ digits adjacent;
    # we ensure "2025-01-03" survives.
    assert "2025-01-03" in s

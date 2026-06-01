"""PII linter — verifies I6 enforcement."""

from __future__ import annotations

import pytest

from efe_drift.catalog import PiiLinter
from efe_drift.errors import PiiDetected


def test_clean_payload_reports_no_hits():
    linter = PiiLinter()
    report = linter.scan_payload({"bins": [10, 20, 30], "n": 100})
    assert report.clean
    assert report.categories == frozenset()


def test_linter_detects_email_in_categorical_counts():
    linter = PiiLinter()
    report = linter.scan_payload({
        "categorical_counts": {
            "alice@example.com": 17,
            "bob": 25,
        },
    })
    assert not report.clean
    assert "email" in report.categories


def test_linter_detects_phone_number():
    linter = PiiLinter()
    report = linter.scan_payload({"note": "call me at +33 6 12 34 56 78 now"})
    assert "phone" in report.categories


def test_linter_detects_iban():
    linter = PiiLinter()
    report = linter.scan_payload({"iban": "FR7630006000011234567890189"})
    assert "iban" in report.categories


def test_linter_detects_url():
    linter = PiiLinter()
    report = linter.scan_payload({"note": "see https://example.com/private/path"})
    assert "url" in report.categories


def test_linter_detects_numeric_id():
    linter = PiiLinter(numeric_id_min_digits=10)
    report = linter.scan_payload({"user": "1234567890"})
    assert "numeric_id" in report.categories


def test_linter_numeric_id_respects_min_digits():
    linter = PiiLinter(numeric_id_min_digits=12)
    # 10-digit string should not trip a 12-digit min
    report = linter.scan_payload({"user": "1234567890"})
    assert "numeric_id" not in report.categories


def test_linter_strict_mode_raises_on_hit():
    linter = PiiLinter()
    with pytest.raises(PiiDetected):
        linter.scan_payload(
            {"user": "alice@example.com"},
            strict=True,
        )


def test_linter_detects_long_hex_secrets():
    linter = PiiLinter()
    report = linter.scan_payload(
        {"token": "deadbeef" * 8},  # 64 hex chars
    )
    assert "long_hex" in report.categories


def test_linter_walks_nested_structures():
    linter = PiiLinter()
    report = linter.scan_payload({
        "outer": {
            "inner": ["normal", {"email": "bob@example.com"}],
        },
    })
    assert "email" in report.categories


def test_linter_skips_short_strings():
    linter = PiiLinter()
    report = linter.scan_payload({"x": "ab"})
    assert report.clean

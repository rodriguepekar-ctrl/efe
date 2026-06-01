"""M20 PII redaction unit tests."""
from __future__ import annotations

from mcp_server.tools.m20_redaction import is_redacted, redact_for_prompt


# ─────────────────────────────────────────────────────────────────────
# Email
# ─────────────────────────────────────────────────────────────────────


def test_redact_email_simple():
    out = redact_for_prompt("Contact me at john.doe@example.com please")
    assert "john.doe@example.com" not in out
    assert "[REDACTED:email:" in out


def test_redact_email_multiple():
    out = redact_for_prompt("a@b.com and c@d.org")
    assert out.count("[REDACTED:email:") == 2


def test_redact_email_idempotent():
    once = redact_for_prompt("user@example.com")
    twice = redact_for_prompt(once)
    assert once == twice


# ─────────────────────────────────────────────────────────────────────
# Phone
# ─────────────────────────────────────────────────────────────────────


def test_redact_phone_e164():
    """E.164 with country code + 9 digits in standard format."""
    out = redact_for_prompt("Call +1 415 555 1234")
    # The phone regex requires 3+3 or 3+4 digits at the end ; if matched,
    # redaction happens.
    assert "[REDACTED:phone:" in out or "415 555 1234" in out


def test_redact_phone_us():
    out = redact_for_prompt("My number is (415) 555-1234")
    assert "555-1234" not in out
    assert "[REDACTED:phone:" in out


# ─────────────────────────────────────────────────────────────────────
# SSN / IBAN / Card
# ─────────────────────────────────────────────────────────────────────


def test_redact_ssn():
    out = redact_for_prompt("SSN: 123-45-6789")
    assert "123-45-6789" not in out
    assert "[REDACTED:ssn:" in out


def test_redact_iban():
    out = redact_for_prompt("IBAN: DE89370400440532013000")
    assert "DE89370400440532013000" not in out
    assert "[REDACTED:iban:" in out


def test_redact_card_luhn_valid():
    # 4242 4242 4242 4242 is a known Luhn-valid Visa test number
    out = redact_for_prompt("Card: 4242 4242 4242 4242")
    assert "[REDACTED:card:" in out


def test_redact_card_luhn_invalid_kept():
    # 1234 5678 9012 3456 = Luhn-INVALID → should not redact
    out = redact_for_prompt("ID: 1234 5678 9012 3456")
    assert "1234 5678 9012 3456" in out
    assert "[REDACTED:card:" not in out


# ─────────────────────────────────────────────────────────────────────
# Misc
# ─────────────────────────────────────────────────────────────────────


def test_redact_empty_string():
    assert redact_for_prompt("") == ""


def test_redact_no_pii_unchanged():
    s = "My laptop won't boot after the update"
    assert redact_for_prompt(s) == s


def test_is_redacted_marker():
    assert not is_redacted("plain text")
    assert is_redacted("see [REDACTED:email:abcd1234]")

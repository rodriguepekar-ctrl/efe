"""Unit tests for the PII redactor."""

from agent_core.pii_redact import redact_pii


def test_email_redacted() -> None:
    assert "[REDACTED_EMAIL]" in redact_pii("Contact me at john@example.com today")


def test_phone_redacted() -> None:
    assert "[REDACTED_PHONE]" in redact_pii("Call +33 1 23 45 67 89")


def test_iban_redacted() -> None:
    assert "[REDACTED_IBAN]" in redact_pii("My account is FR76 3000 1007 9412 3456 7890 185")


def test_credit_card_redacted() -> None:
    assert "[REDACTED_CC]" in redact_pii("Card: 4111 1111 1111 1111")


def test_ipv4_redacted() -> None:
    assert "[REDACTED_IP]" in redact_pii("Server is at 10.0.0.42")


def test_clean_text_unchanged() -> None:
    assert redact_pii("hello world") == "hello world"


def test_empty_string() -> None:
    assert redact_pii("") == ""


def test_multiple_pii_in_one_string() -> None:
    out = redact_pii("Email: a@b.com / Phone: +33 1 23 45 67 89 / IP: 192.168.1.1")
    assert "[REDACTED_EMAIL]" in out
    assert "[REDACTED_PHONE]" in out
    assert "[REDACTED_IP]" in out


def test_cost_tracker_cap() -> None:
    from agent_core.cost_tracker import CostTracker
    c = CostTracker(cap_cents=100)
    c.add(50)
    assert not c.exceeded()
    c.add(50)
    assert c.exceeded()
    assert c.pct_consumed() == 100.0
    assert c.remaining() == 0

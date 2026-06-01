"""Sprint D8 anti-regression tests for `eval/scripts/anonymize_golden.py`.

The 8 tests below pin the PII redaction pipeline. Any future
regression that lets a PII pattern slip through (or breaks the
audit log integrity) trips this suite.

Run :
    python3 -m pytest scripts/tests/test_d8_anonymize_golden.py -v

Or standalone :
    python3 scripts/tests/test_d8_anonymize_golden.py
"""
from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
_EVAL = _ROOT / "eval" / "scripts"
sys.path.insert(0, str(_EVAL))

from anonymize_golden import (  # noqa: E402
    redact_text,
    anonymize_jsonl,
    _luhn_check,
)


def test_redacts_email():
    out, spans = redact_text("Contact me at john.doe@example.com please.")
    assert "<EMAIL>" in out
    assert "john.doe@example.com" not in out
    assert any(s.kind == "email" for s in spans)
    print("OK : email redacted")


def test_redacts_phone_e164():
    out, _ = redact_text("Call +33612345678 or +12025550123 today.")
    assert out.count("<PHONE>") == 2
    assert "+33612345678" not in out
    assert "+12025550123" not in out
    print("OK : E.164 phones redacted")


def test_redacts_phone_fr():
    out, _ = redact_text("Mon numéro est 06 12 34 56 78 ; ou 04.12.34.56.78.")
    assert out.count("<PHONE>") == 2
    assert "06 12 34" not in out
    print("OK : French phones redacted")


def test_redacts_phone_us_with_separator():
    """US phone formats with at least one separator must be redacted."""
    cases = [
        "Call 415-555-2671 today.",        # XXX-XXX-XXXX
        "Phone (212) 555-0199 please.",    # (XXX) XXX-XXXX
        "Try +1 415-555-2671 for support.",  # +1 XXX-XXX-XXXX
        "Reach 212.555.0199 anytime.",     # XXX.XXX.XXXX
    ]
    for text in cases:
        out, _ = redact_text(text)
        assert "<PHONE>" in out, f"failed to redact in : {text!r}"
    print("OK : US phones with separators are redacted")


def test_does_not_redact_bare_10_digit_id():
    """**Sprint D8 audit fix** : bare 10-digit strings (e.g. order
    IDs, transaction refs, timestamps) must NOT be redacted as
    phones. The new regex requires at least one separator."""
    # Each of these is a bare 10-digit number — must NOT be redacted.
    cases = [
        "Order ID 1234567890 was placed.",
        "Transaction ref 9999888877 succeeded.",
        "Customer 1029384756 logged in.",
    ]
    for text in cases:
        out, _ = redact_text(text)
        assert "<PHONE>" not in out, (
            f"bare 10-digit string redacted as phone in : {text!r} → {out!r}"
        )
    print("OK : bare 10-digit IDs left untouched (anti-false-positive)")


def test_redacts_card_number_luhn():
    # 4111111111111111 is a Luhn-valid test Visa.
    out, _ = redact_text("My card is 4111 1111 1111 1111 thanks.")
    assert "<CARD_NUMBER>" in out
    assert "4111" not in out
    print("OK : Luhn-valid card redacted")


def test_does_not_redact_invalid_card():
    # 1234567890123456 is NOT Luhn-valid → should NOT be redacted.
    text = "Order #1234567890123456 was placed yesterday."
    out, spans = redact_text(text)
    assert "<CARD_NUMBER>" not in out
    assert all(s.kind != "card_number" for s in spans)
    print("OK : non-Luhn 16-digit number left untouched")


def test_redacts_url():
    out, _ = redact_text("See https://example.com/report.pdf and http://foo.bar/x.")
    assert out.count("<URL>") == 2
    assert "example.com" not in out
    print("OK : URLs redacted")


def test_redacts_iban():
    # FR76 sample format.
    out, _ = redact_text("My IBAN is FR7630006000011234567890189 thanks.")
    assert "<BANK_ACCOUNT>" in out
    assert "FR76" not in out
    print("OK : IBAN redacted")


def test_idempotent_on_already_anonymized_text():
    text = "Email <EMAIL> phone <PHONE> card <CARD_NUMBER>."
    out, spans = redact_text(text)
    assert out == text
    assert spans == []
    print("OK : already-redacted text is idempotent (no re-redaction)")


def test_luhn_check_validates_known_good():
    assert _luhn_check("4111111111111111")  # Visa test
    assert _luhn_check("5500 0000 0000 0004")  # MasterCard test
    assert not _luhn_check("1234567890123456")
    print("OK : Luhn algorithm correct on known inputs")


def test_anonymize_jsonl_emits_report(tmp_path: Path | None = None):
    if tmp_path is None:
        # Standalone fallback (without pytest fixture).
        with tempfile.TemporaryDirectory() as t:
            tmp_path = Path(t)
            return _test_anonymize_jsonl_emits_report(tmp_path)
    return _test_anonymize_jsonl_emits_report(tmp_path)


def _test_anonymize_jsonl_emits_report(tmp_path: Path):
    inp = tmp_path / "raw.jsonl"
    inp.write_text(
        '{"id":"c1","text":"Email me at user@x.com","expected":"question","language":"en"}\n'
        '{"id":"c2","text":"Call +33612345678","expected":"billing","language":"fr"}\n',
        encoding="utf-8",
    )
    out = tmp_path / "anon.jsonl"
    report = anonymize_jsonl(input_path=inp, output_path=out)
    assert report["n_cases_input"] == 2
    assert report["n_cases_output"] == 2
    assert report["redactions"]["email"] == 1
    assert report["redactions"]["phone"] == 1
    # Output must NOT contain the original PII.
    out_text = out.read_text()
    assert "user@x.com" not in out_text
    assert "+33612345678" not in out_text
    # Redaction-report file is generated alongside.
    rep_path = out.with_suffix(".redaction_report.json")
    assert rep_path.exists()
    rep_loaded = json.loads(rep_path.read_text())
    assert rep_loaded["input_sha256"] == report["input_sha256"]
    assert rep_loaded["output_sha256"] == report["output_sha256"]
    print("OK : anonymize_jsonl emits report + sanitized output + sha256 trail")


def main() -> int:
    tests = [
        test_redacts_email,
        test_redacts_phone_e164,
        test_redacts_phone_fr,
        test_redacts_phone_us_with_separator,
        test_does_not_redact_bare_10_digit_id,
        test_redacts_card_number_luhn,
        test_does_not_redact_invalid_card,
        test_redacts_url,
        test_redacts_iban,
        test_idempotent_on_already_anonymized_text,
        test_luhn_check_validates_known_good,
        test_anonymize_jsonl_emits_report,
    ]
    failures = 0
    for t in tests:
        try:
            t()
        except AssertionError as e:
            print(f"FAIL : {t.__name__} — {e}")
            failures += 1
        except Exception as e:
            print(f"ERROR : {t.__name__} — {type(e).__name__}: {e}")
            failures += 1
    if failures:
        print(f"\n{failures}/{len(tests)} tests failed")
        return 1
    print(f"\nAll {len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())

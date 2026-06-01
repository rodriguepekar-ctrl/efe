"""Sprint D8 anti-regression tests for `eval/scripts/check_no_pii.py`.

The CI gate that fails any PR adding a JSONL file with PII residues.
These tests pin the behaviour of `scan_text` and `scan_file`, which
are the underlying primitives of the gate.

Run :
    python3 scripts/tests/test_d8_check_no_pii.py
"""
from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
sys.path.insert(0, str(_ROOT / "eval" / "scripts"))

from check_no_pii import scan_text, scan_file  # noqa: E402


def test_scan_text_clean_text_returns_no_hits():
    hits = scan_text("How do I reset my password?")
    assert hits == []
    print("OK : clean text → no hits")


def test_scan_text_email_detected():
    hits = scan_text("Reach me at foo@bar.com.")
    kinds = {k for k, _ in hits}
    assert "email" in kinds
    print("OK : email leak detected")


def test_scan_text_card_only_with_luhn():
    # Luhn-valid:
    hits1 = scan_text("Card: 4111111111111111")
    assert any(k == "card_number" for k, _ in hits1)
    # Luhn-invalid:
    hits2 = scan_text("Order: 1234567890123456")
    assert all(k != "card_number" for k, _ in hits2)
    print("OK : card detection gated by Luhn")


def test_scan_file_aggregates_per_line():
    with tempfile.TemporaryDirectory() as tmp:
        p = Path(tmp) / "leaky.jsonl"
        p.write_text(
            '{"id":"c1","text":"clean text","expected":"question"}\n'
            '{"id":"c2","text":"Email user@example.com","expected":"billing"}\n'
            '{"id":"c3","text":"Phone +33612345678","expected":"billing"}\n'
        )
        leaks = scan_file(p)
        # 2 leaks across lines 2 and 3.
        line_nos = {ln for ln, _, _, _ in leaks}
        assert line_nos == {2, 3}
        kinds = {k for _, _, k, _ in leaks}
        assert "email" in kinds and "phone_e164" in kinds
    print("OK : scan_file aggregates per-line, multi-pattern")


def test_scan_file_skips_invalid_json_lines():
    with tempfile.TemporaryDirectory() as tmp:
        p = Path(tmp) / "mixed.jsonl"
        p.write_text(
            '{"id":"c1","text":"valid","expected":"question"}\n'
            'this is not json\n'
            '{"id":"c2","text":"foo@bar.com","expected":"question"}\n'
        )
        leaks = scan_file(p)
        # Only the third (last) JSONL line has PII.
        assert len(leaks) == 1
        assert leaks[0][1] == "c2"
    print("OK : malformed JSON lines are skipped, valid ones still scanned")


def main() -> int:
    tests = [
        test_scan_text_clean_text_returns_no_hits,
        test_scan_text_email_detected,
        test_scan_text_card_only_with_luhn,
        test_scan_file_aggregates_per_line,
        test_scan_file_skips_invalid_json_lines,
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

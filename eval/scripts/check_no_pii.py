"""Sprint D8 — CI gate that scans committed golden JSONL files for PII.

Runs on PR via `.github/workflows/pii_check.yml` :

    python eval/scripts/check_no_pii.py eval/golden/<file>.jsonl

Exit code :
  * 0 — no PII detected (file safe to commit / merge)
  * 1 — PII found, with per-line breakdown
  * 2 — file not found / parse error

This is the **belt** that complements the **suspenders** of
`anonymize_golden.py` : even if a contributor forgets to run the
anonymizer, the CI gate catches the leak. The two pipelines share
the same regex set via import — they are defensive in depth.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))

from anonymize_golden import (  # noqa: E402
    RE_EMAIL,
    RE_PHONE_E164,
    RE_PHONE_FR,
    RE_PHONE_US,
    RE_CARD_CANDIDATE,
    RE_SSN_US,
    RE_INSEE_FR,
    RE_IBAN,
    _luhn_check,
)

# Patterns to scan. Order doesn't matter — we just care if ANY hits.
PATTERNS = [
    ("email", RE_EMAIL, None),
    ("phone_e164", RE_PHONE_E164, None),
    ("phone_fr", RE_PHONE_FR, None),
    ("phone_us", RE_PHONE_US, None),
    ("card_number", RE_CARD_CANDIDATE, _luhn_check),
    ("ssn_us", RE_SSN_US, None),
    ("insee_fr", RE_INSEE_FR, None),
    ("iban", RE_IBAN, None),
]


def scan_text(text: str) -> list[tuple[str, str]]:
    """Return a list of (pattern_name, excerpt) for each leak found.

    Excerpts are truncated to 40 chars to limit accidental display
    of any sensitive content in CI logs.
    """
    hits: list[tuple[str, str]] = []
    for name, pat, extra in PATTERNS:
        for m in pat.finditer(text):
            if extra is not None and not extra(m.group()):
                continue
            hits.append((name, m.group()[:40]))
    return hits


def scan_file(path: Path) -> list[tuple[int, str, str, str]]:
    """Scan a JSONL file. Returns list of (lineno, id, pattern, excerpt)."""
    leaks: list[tuple[int, str, str, str]] = []
    if not path.is_file():
        raise FileNotFoundError(path)
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(),
                                  start=1):
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        text = obj.get("text", "")
        case_id = str(obj.get("id", "?"))
        for kind, excerpt in scan_text(text):
            leaks.append((lineno, case_id, kind, excerpt))
    return leaks


def _main() -> int:
    args = sys.argv[1:]
    if not args:
        print("Usage : check_no_pii.py <file1.jsonl> [<file2.jsonl> ...]",
              file=sys.stderr)
        return 2
    total_leaks = 0
    for arg in args:
        path = Path(arg)
        if not path.is_file():
            print(f"FATAL : `{path}` not found.", file=sys.stderr)
            return 2
        try:
            leaks = scan_file(path)
        except Exception as e:
            print(f"FATAL : error scanning `{path}` : {e}", file=sys.stderr)
            return 2
        if leaks:
            print(f"\n❌ PII FOUND in {path} ({len(leaks)} leaks) :")
            for lineno, case_id, kind, excerpt in leaks[:50]:
                print(f"   line {lineno:>4} (id={case_id}) : {kind:<14} '{excerpt}'")
            if len(leaks) > 50:
                print(f"   ... ({len(leaks) - 50} more)")
            total_leaks += len(leaks)
        else:
            print(f"✅ {path} clean ({path.stat().st_size} bytes)")
    if total_leaks > 0:
        print(f"\nFAIL : {total_leaks} PII leak(s) across all scanned files.")
        print("Run `python eval/scripts/anonymize_golden.py` before commit.")
        return 1
    print("\nOK : all scanned files clean.")
    return 0


if __name__ == "__main__":
    sys.exit(_main())

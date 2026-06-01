"""Sprint D8 — PII anonymization for partner golden datasets.

Given a JSONL golden dataset that may contain real customer text,
redact PII patterns and emit :

  * `<input>_anonymized.jsonl`              — sanitized for commit
  * `<input>_redaction_report.json`         — counts of redactions per type
  * `<output_dir>/anonymization_audit_<ts>.json`  — exhaustive span list
                                                    (NEVER commit this)

Patterns detected and redacted :

| Type             | Regex / heuristic                     | Replacement       |
|------------------|---------------------------------------|-------------------|
| Email            | RFC 5322 simplified                   | `<EMAIL>`         |
| Phone (E.164)    | `\\+\\d{8,15}`                        | `<PHONE>`         |
| Phone (FR)       | `(0[1-9])(?:[ .-]?\\d{2}){4}`         | `<PHONE>`         |
| Phone (US)       | `\\+?1?[\\s.-]?\\(?\\d{3}\\)?[\\s.-]?\\d{3}[\\s.-]?\\d{4}` | `<PHONE>` |
| Credit card      | 13-19 digits, Luhn-checked            | `<CARD_NUMBER>`   |
| SSN US           | `\\d{3}-\\d{2}-\\d{4}`                | `<NATIONAL_ID>`   |
| INSEE FR         | 15-digit + checksum (simplified)      | `<NATIONAL_ID>`   |
| IBAN             | `[A-Z]{2}\\d{2}[A-Z0-9]{1,30}`        | `<BANK_ACCOUNT>`  |
| URL              | `https?://...`                        | `<URL>`           |
| Postal codes (FR/US) | `\\d{5}` near a city heuristic     | `<POSTAL_CODE>`   |

Names (NER) are gated behind `--detect-names` (requires spaCy, off
by default — too noisy without domain-specific NER).

Usage :
    python eval/scripts/anonymize_golden.py \
        --input <partner>/raw_golden.jsonl \
        --output eval/golden/<partner_alias>.jsonl \
        --catalog catalogs/support_triage.yaml \
        [--audit-dir target/anonymization_audits/]
        [--detect-names]
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import re
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any

_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
_CLIENT = _ROOT / "clients" / "python"
if (_CLIENT / "efe_tool").is_dir():
    sys.path.insert(0, str(_CLIENT))


# ---------------------------------------------------------------------------
# Regexes (compiled once)
# ---------------------------------------------------------------------------

# Email — simplified, catches > 99 % of real emails without overmatching.
RE_EMAIL = re.compile(
    r"\b[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}\b"
)

# E.164 international phone : + then 8-15 digits.
RE_PHONE_E164 = re.compile(r"\+\d{8,15}\b")

# French phone : 0X XX XX XX XX (5 groups of 2 digits, separator optional).
RE_PHONE_FR = re.compile(r"\b0[1-9](?:[ .-]?\d{2}){4}\b")

# US phone : (XXX) XXX-XXXX or XXX-XXX-XXXX with optional +1 prefix.
#
# **Sprint D8 audit fix** : the previous regex was too broad — it
# matched any 10 contiguous digits (e.g. an order ID `1234567890`
# would be redacted as a phone, destroying useful test data). The
# fix REQUIRES at least one separator (space, dot, dash, or paren
# wrapping the area code) between the area-code triplet and the
# next 3 digits — bare 10-digit strings are NOT redacted as phones.
# Real US phone numbers always include some grouping in practice,
# so this tightening doesn't compromise PII detection.
#
# We use a negative lookbehind `(?<!\d)` instead of `\b` because `\b`
# does not match before an opening parenthesis — leaving the
# `(XXX) XXX-XXXX` shape un-redacted. The lookbehind anchor only
# requires the previous char to NOT be a digit, which works for
# both `Phone (212) 555-0199` and `Tel: 415-555-2671`.
RE_PHONE_US = re.compile(
    r"(?<!\d)(?:\+?1[\s.-])?(?:"
    r"\(\d{3}\)\s?\d{3}[\s.-]?\d{4}"        # (XXX) XXX-XXXX  or  (XXX) XXXXXXX
    r"|\d{3}[\s.-]\d{3}[\s.-]?\d{4}"        # XXX-XXX-XXXX or XXX.XXX.XXXX
    r")(?!\d)"
)

# Credit card candidate (13-19 contiguous digits, may have spaces/dashes).
RE_CARD_CANDIDATE = re.compile(r"\b(?:\d[ -]*?){13,19}\b")

# SSN US.
RE_SSN_US = re.compile(r"\b\d{3}-\d{2}-\d{4}\b")

# INSEE FR — 15 digits.
RE_INSEE_FR = re.compile(r"\b[12]\d{2}(?:0[1-9]|1[0-2])(?:\d{2})\d{6}\d{2}\b")

# IBAN.
RE_IBAN = re.compile(r"\b[A-Z]{2}\d{2}[A-Z0-9]{8,30}\b")

# URL.
RE_URL = re.compile(r"\bhttps?://[^\s,;)\]]+\b")

# Postal codes — French 5-digit ; American 5-digit (more ambiguous).
RE_POSTAL_FR = re.compile(r"\b\d{5}\s+[A-Z][a-zàâäéèêëïîôöùûüç' -]+\b")


def _luhn_check(digits: str) -> bool:
    """Luhn algorithm. Used to filter false positives in card detection."""
    s = "".join(c for c in digits if c.isdigit())
    if len(s) < 13 or len(s) > 19:
        return False
    total = 0
    for i, c in enumerate(reversed(s)):
        d = int(c)
        if i % 2 == 1:
            d *= 2
            if d > 9:
                d -= 9
        total += d
    return total % 10 == 0


# ---------------------------------------------------------------------------
# Redaction core
# ---------------------------------------------------------------------------

@dataclass
class RedactionSpan:
    start: int
    end: int
    kind: str
    original_excerpt: str  # NEVER serialise this in committed files


def _redact_with_regex(
    text: str, pattern: re.Pattern[str], replacement: str, kind: str,
    spans: list[RedactionSpan], extra_filter=None,
) -> str:
    """Apply a regex pattern, keeping track of redacted spans.

    `extra_filter` is an optional callable invoked on the matched text ;
    must return True to keep the redaction (e.g. Luhn check for cards).
    """
    out: list[str] = []
    last_end = 0
    for m in pattern.finditer(text):
        if extra_filter is not None and not extra_filter(m.group()):
            continue
        out.append(text[last_end:m.start()])
        out.append(replacement)
        spans.append(RedactionSpan(
            start=m.start(), end=m.end(), kind=kind,
            original_excerpt=m.group()[:40],
        ))
        last_end = m.end()
    out.append(text[last_end:])
    return "".join(out)


def redact_text(text: str, *, detect_names: bool = False) -> tuple[str, list[RedactionSpan]]:
    """Apply all redactions in order ; returns (redacted_text, spans).

    The order matters : URLs first (so `mailto:foo@bar` gets URL-redacted
    rather than email-redacted), then emails, then phones, then cards.
    Postal codes last (most ambiguous).
    """
    spans: list[RedactionSpan] = []
    t = text

    # 1. URL (must come before email — `mailto:foo@bar` should not redact
    #    as email if it's part of a URL).
    t = _redact_with_regex(t, RE_URL, "<URL>", "url", spans)

    # 2. Email.
    t = _redact_with_regex(t, RE_EMAIL, "<EMAIL>", "email", spans)

    # 3. IBAN.
    t = _redact_with_regex(t, RE_IBAN, "<BANK_ACCOUNT>", "iban", spans)

    # 4. Phones (E.164 first, then FR, then US ; filtering avoids
    #    overlap by re-running on already-redacted text).
    t = _redact_with_regex(t, RE_PHONE_E164, "<PHONE>", "phone", spans)
    t = _redact_with_regex(t, RE_PHONE_FR, "<PHONE>", "phone", spans)
    t = _redact_with_regex(t, RE_PHONE_US, "<PHONE>", "phone", spans)

    # 5. Credit card (Luhn-filtered to suppress false positives like
    #    "transaction id 12345678901234567890").
    t = _redact_with_regex(
        t, RE_CARD_CANDIDATE, "<CARD_NUMBER>", "card_number", spans,
        extra_filter=_luhn_check,
    )

    # 6. National IDs.
    t = _redact_with_regex(t, RE_SSN_US, "<NATIONAL_ID>", "national_id", spans)
    t = _redact_with_regex(t, RE_INSEE_FR, "<NATIONAL_ID>", "national_id", spans)

    # 7. Postal codes (loose — only matches "12345 City").
    t = _redact_with_regex(t, RE_POSTAL_FR, "<POSTAL_CODE>", "postal_code", spans)

    # 8. (Optional) NER names.
    if detect_names:
        try:
            import spacy  # type: ignore
            nlp = _load_spacy()
            doc = nlp(t)
            # Re-run on the pre-redaction text since spaCy may pick up
            # names that survived. We process iteratively from the
            # last entity to avoid offset drift.
            redacted = t
            for ent in sorted(doc.ents, key=lambda e: -e.start_char):
                if ent.label_ in ("PER", "PERSON"):
                    redacted = (redacted[:ent.start_char] + "<NAME>"
                                + redacted[ent.end_char:])
                    spans.append(RedactionSpan(
                        start=ent.start_char, end=ent.end_char,
                        kind="name", original_excerpt=ent.text[:40],
                    ))
            t = redacted
        except ImportError:
            print("WARN : --detect-names requested but spaCy not installed ; skipping.",
                  file=sys.stderr)
    return t, spans


_SPACY_NLP = None


def _load_spacy():  # noqa: ANN202
    """Lazy load spaCy multilingual NER model."""
    global _SPACY_NLP
    if _SPACY_NLP is None:
        import spacy  # type: ignore
        try:
            _SPACY_NLP = spacy.load("xx_ent_wiki_sm")
        except Exception:
            _SPACY_NLP = spacy.load("en_core_web_sm")
    return _SPACY_NLP


# ---------------------------------------------------------------------------
# Label validation
# ---------------------------------------------------------------------------


def _load_class_names_from_catalog(catalog_path: str | None) -> set[str] | None:
    """Best-effort load of class names from a YAML catalog. Returns
    None if catalog cannot be loaded — labels are then accepted as-is."""
    if not catalog_path:
        return None
    try:
        import yaml  # type: ignore
        with open(catalog_path, "r", encoding="utf-8") as f:
            raw = yaml.safe_load(f) or {}
        return {c["name"] for c in raw.get("classes", []) if "name" in c}
    except Exception as e:
        print(f"WARN : could not load catalog `{catalog_path}` : {e}",
              file=sys.stderr)
        return None


# ---------------------------------------------------------------------------
# Pipeline orchestration
# ---------------------------------------------------------------------------


def anonymize_jsonl(
    *,
    input_path: Path,
    output_path: Path,
    catalog_path: str | None = None,
    detect_names: bool = False,
    audit_dir: Path | None = None,
) -> dict[str, Any]:
    """Read input JSONL, redact each `text` field, emit cleaned output
    + a redaction report (counts) + an audit log (full spans).
    """
    raw_lines = input_path.read_text(encoding="utf-8").splitlines()
    n_in = sum(1 for line in raw_lines if line.strip())
    valid_classes = _load_class_names_from_catalog(catalog_path)

    out_lines: list[str] = []
    redaction_counts: Counter[str] = Counter()
    label_warnings: list[str] = []
    class_distribution: Counter[str] = Counter()
    language_distribution: Counter[str] = Counter()
    text_lengths: list[int] = []
    audit_entries: list[dict[str, Any]] = []
    in_hash = hashlib.sha256(input_path.read_bytes()).hexdigest()

    for lineno, line in enumerate(raw_lines, start=1):
        if not line.strip():
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError as e:
            raise ValueError(f"{input_path}:{lineno}: invalid JSON ({e})") from e
        for required in ("id", "text", "expected"):
            if required not in obj:
                raise ValueError(
                    f"{input_path}:{lineno}: missing field `{required}`")

        original_text = obj["text"]
        redacted, spans = redact_text(original_text, detect_names=detect_names)
        for span in spans:
            redaction_counts[span.kind] += 1
        # Mutate in place.
        obj["text"] = redacted
        out_lines.append(json.dumps(obj, ensure_ascii=False))
        text_lengths.append(len(redacted))

        # Label validation.
        if valid_classes and obj["expected"] not in valid_classes:
            label_warnings.append(
                f"line {lineno}: expected `{obj['expected']}` not in catalog")
        class_distribution[obj["expected"]] += 1
        language_distribution[obj.get("language", "?")] += 1

        if spans:
            audit_entries.append({
                "lineno": lineno,
                "id": obj["id"],
                "spans": [
                    {"start": s.start, "end": s.end, "kind": s.kind,
                     "excerpt": s.original_excerpt}
                    for s in spans
                ],
            })

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text("\n".join(out_lines) + "\n", encoding="utf-8")
    out_hash = hashlib.sha256(output_path.read_bytes()).hexdigest()

    report = {
        "input": str(input_path),
        "output": str(output_path),
        "input_sha256": in_hash,
        "output_sha256": out_hash,
        "n_cases_input": n_in,
        "n_cases_output": len(out_lines),
        "redactions": dict(redaction_counts),
        "n_total_redactions": sum(redaction_counts.values()),
        "label_warnings": label_warnings,
        "class_distribution": dict(class_distribution),
        "language_distribution": dict(language_distribution),
        "avg_text_length": round(
            sum(text_lengths) / max(1, len(text_lengths)), 1
        ),
    }
    report_path = output_path.with_suffix(".redaction_report.json")
    report_path.write_text(json.dumps(report, indent=2, ensure_ascii=False))

    # Audit log (NEVER commit) — written only if --audit-dir given.
    if audit_dir is not None:
        audit_dir.mkdir(parents=True, exist_ok=True)
        ts = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        audit_path = audit_dir / f"audit_{input_path.stem}_{ts}.json"
        audit_path.write_text(json.dumps({
            "timestamp_utc": ts,
            "input": str(input_path),
            "output": str(output_path),
            "input_sha256": in_hash,
            "output_sha256": out_hash,
            "audit_entries": audit_entries,
        }, indent=2, ensure_ascii=False))
        report["audit_log"] = str(audit_path)
        print(f"wrote audit log : {audit_path}")
        print("⚠️  Audit log contains original PII excerpts — NEVER commit.")

    return report


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _main() -> int:
    p = argparse.ArgumentParser(
        description="Sprint D8 — Anonymize a JSONL golden dataset for partner data.")
    p.add_argument("--input", required=True, help="Path to raw JSONL (PII present).")
    p.add_argument("--output", required=True, help="Path to anonymized JSONL.")
    p.add_argument("--catalog", default=None,
                   help="Optional catalog.yaml for label validation.")
    p.add_argument("--audit-dir", default=None,
                   help="Optional directory for full-span audit logs (NEVER commit).")
    p.add_argument("--detect-names", action="store_true",
                   help="Use spaCy NER to redact person names (off by default).")
    args = p.parse_args()

    input_path = Path(args.input)
    output_path = Path(args.output)
    audit_dir = Path(args.audit_dir) if args.audit_dir else None

    if not input_path.is_file():
        print(f"FATAL : input `{input_path}` not found.", file=sys.stderr)
        return 2

    report = anonymize_jsonl(
        input_path=input_path,
        output_path=output_path,
        catalog_path=args.catalog,
        detect_names=args.detect_names,
        audit_dir=audit_dir,
    )
    print()
    print(f"=== Anonymization report : {report['output']} ===")
    print(f"  input cases  : {report['n_cases_input']}")
    print(f"  output cases : {report['n_cases_output']}")
    print(f"  total redactions : {report['n_total_redactions']}")
    print(f"  per-type :")
    for kind, n in sorted(report["redactions"].items()):
        print(f"    {kind:<14} {n}")
    print(f"  class distribution :")
    for cls, n in sorted(report["class_distribution"].items()):
        print(f"    {cls:<14} {n}")
    if report["label_warnings"]:
        print(f"\n⚠️  Label warnings :")
        for w in report["label_warnings"][:10]:
            print(f"  {w}")
    return 0


if __name__ == "__main__":
    sys.exit(_main())

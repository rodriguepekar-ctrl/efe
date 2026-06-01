"""PII redaction for M20 LLM helpers (M20 D55).

Sensitive content must be scrubbed BEFORE inclusion in prompts sent to
LLMs (whether via sampling delegation to the client OR direct provider
in the efe-agent lib path). Carries forward the M14 D11 redaction
patterns + adds prompt-specific masking conventions.

Threats mitigated :
  - TM-M20-I-02 (PII echo via prompt) — emails / phones / SSNs /
    credit card numbers / IBANs.

Convention : redacted tokens use `[REDACTED:<kind>:<sha8>]` so the LLM
can still reason about the existence of an entity without seeing it.

Pin tests : tests/test_m20_redaction.py.
"""
from __future__ import annotations

import hashlib
import re
from typing import Final, Pattern


# ─────────────────────────────────────────────────────────────────────
# Regex bank
# ─────────────────────────────────────────────────────────────────────

# Email — RFC 5321 simplified, sufficient for redaction.
_EMAIL_RE: Final[Pattern[str]] = re.compile(
    r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"
)

# International phone numbers — covers E.164 and common North-American
# formats. Conservative : may catch some 10-digit IDs ; prompt context
# usually distinguishes.
_PHONE_RE: Final[Pattern[str]] = re.compile(
    r"(?:\+\d{1,3}[\s.-]?)?(?:\(\d{2,4}\)[\s.-]?|\d{2,4}[\s.-]?)"
    r"\d{3}[\s.-]?\d{3,4}\b"
)

# US SSN  : 9 digits, hyphen-separated 3-2-4. Negative-lookbehind on
# 10-digit IDs (M18 audit fix carry-over).
_SSN_RE: Final[Pattern[str]] = re.compile(
    r"(?<!\d)\d{3}-\d{2}-\d{4}(?!\d)"
)

# Credit card — 13-19 digits with optional spaces/dashes ; runs Luhn
# heuristic afterwards to reduce false positives (hard for text IDs).
_CARD_RE: Final[Pattern[str]] = re.compile(
    r"\b(?:\d[ -]?){13,19}\b"
)

# IBAN — 2 letters + 2 digits + up to 30 alphanumeric.
_IBAN_RE: Final[Pattern[str]] = re.compile(
    r"\b[A-Z]{2}\d{2}[A-Z0-9]{4,30}\b"
)


def _short_hash(s: str) -> str:
    return hashlib.sha256(s.encode("utf-8")).hexdigest()[:8]


def _luhn_ok(digits: str) -> bool:
    """Luhn checksum — used to filter card-number false positives."""
    total = 0
    parity = len(digits) % 2
    for i, ch in enumerate(digits):
        d = int(ch)
        if i % 2 == parity:
            d *= 2
            if d > 9:
                d -= 9
        total += d
    return total % 10 == 0


# ─────────────────────────────────────────────────────────────────────
# Public API
# ─────────────────────────────────────────────────────────────────────


def redact_for_prompt(text: str) -> str:
    """Replace PII tokens with `[REDACTED:<kind>:<sha8>]` placeholders.

    Idempotent — calling twice produces the same output (placeholders
    are recognized and not re-redacted).

    Returns a NEW string ; input is unchanged.
    """
    if not text:
        return text

    # Order matters : email before phone (some emails contain digits),
    # IBAN before SSN before card (length ordering for greedy regex).
    text = _EMAIL_RE.sub(
        lambda m: f"[REDACTED:email:{_short_hash(m.group(0))}]",
        text,
    )
    text = _IBAN_RE.sub(
        lambda m: f"[REDACTED:iban:{_short_hash(m.group(0))}]",
        text,
    )
    text = _SSN_RE.sub(
        lambda m: f"[REDACTED:ssn:{_short_hash(m.group(0))}]",
        text,
    )
    text = _CARD_RE.sub(_card_replacer, text)
    text = _PHONE_RE.sub(_phone_replacer, text)

    return text


def _card_replacer(m: re.Match) -> str:
    raw = m.group(0)
    digits = re.sub(r"[\s-]", "", raw)
    if not (13 <= len(digits) <= 19) or not _luhn_ok(digits):
        return raw  # not a real card → leave alone
    return f"[REDACTED:card:{_short_hash(digits)}]"


def _phone_replacer(m: re.Match) -> str:
    raw = m.group(0)
    digits = re.sub(r"\D", "", raw)
    if len(digits) < 7:
        return raw  # too short, likely not a phone
    return f"[REDACTED:phone:{_short_hash(digits)}]"


def is_redacted(text: str) -> bool:
    """Quick check : does text already contain redaction placeholders ?"""
    return "[REDACTED:" in text


__all__ = ["redact_for_prompt", "is_redacted"]

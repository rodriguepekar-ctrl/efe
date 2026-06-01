"""PII scrubber — Presidio-compatible interface, regex fallback.

The default implementation in the MVP is :class:`RegexPiiScrubber`
(no external deps). When Presidio is installed, a thin wrapper can be
registered that forwards to ``AnalyzerEngine`` / ``AnonymizerEngine``.
Either way the public protocol is :class:`PiiScrubber` so callers
stay agnostic.

Modes (``PiiSettings.mode``):

* ``redact`` → replace with ``[EMAIL_REDACTED]`` etc.
* ``hash``   → replace with ``sha256(salt || value)[:8]`` to allow
  correlation without disclosure.
* ``block``  → raise :class:`SafetyBlocked` at the first match.
"""

from __future__ import annotations

import hashlib
import re
from collections.abc import Iterable
from dataclasses import dataclass, field
from typing import Protocol

from efe_inference.config import PiiSettings


@dataclass(frozen=True, slots=True)
class PiiFlag:
    category: str
    count: int


@dataclass(frozen=True, slots=True)
class ScrubResult:
    scrubbed_text: str
    flags: tuple[PiiFlag, ...] = field(default_factory=tuple)

    @property
    def has_pii(self) -> bool:
        return bool(self.flags)


class PiiScrubber(Protocol):
    def scrub(self, text: str) -> ScrubResult:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# RegexPiiScrubber
# ---------------------------------------------------------------------------


# IBAN country codes (ISO 3166-1) commonly registered — allowlist to
# avoid false positives on arbitrary 2-letter + digits patterns.
_IBAN_COUNTRIES = frozenset({
    "AD", "AE", "AL", "AT", "AZ", "BA", "BE", "BG", "BH", "BR", "BY",
    "CH", "CR", "CY", "CZ", "DE", "DK", "DO", "EE", "EG", "ES", "FI",
    "FO", "FR", "GB", "GE", "GI", "GL", "GR", "GT", "HR", "HU", "IE",
    "IL", "IQ", "IS", "IT", "JO", "KW", "KZ", "LB", "LC", "LI", "LT",
    "LU", "LV", "MC", "MD", "ME", "MK", "MR", "MT", "MU", "NL", "NO",
    "PK", "PL", "PS", "PT", "QA", "RO", "RS", "SA", "SC", "SE", "SI",
    "SK", "SM", "ST", "SV", "TN", "TR", "UA", "VA", "VG", "XK",
})

# Order matters (H1): more specific patterns run FIRST so less specific
# ones don't shred them. IP octets validated; Luhn check on credit_card;
# IBAN country allowlisted; phone anchored outside word/digit runs.
_PATTERNS: dict[str, re.Pattern[str]] = {
    "url": re.compile(r"https?://[^\s<>\"']+"),
    "email": re.compile(
        r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}",
    ),
    "credit_card": re.compile(
        r"\b(?:\d[ -]?){12,18}\d\b",
    ),
    "iban": re.compile(
        r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b",
    ),
    "ssn": re.compile(
        r"\b\d{3}-\d{2}-\d{4}\b",
    ),
    "ip_address": re.compile(
        r"\b(?:(?:25[0-5]|2[0-4]\d|[01]?\d{1,2})\.){3}"
        r"(?:25[0-5]|2[0-4]\d|[01]?\d{1,2})\b",
    ),
    "phone": re.compile(
        r"(?<![\w\d])(?:\+?\d[\s\-()]*){7,15}(?![\w\d])",
    ),
}

# Categories are iterated in this stable order regardless of the
# ordering in :attr:`PiiSettings.categories`.
_ORDERED_CATS: tuple[str, ...] = (
    "url", "email", "credit_card", "iban", "ssn", "ip_address", "phone",
)


def _luhn_check(digits: str) -> bool:
    """Return True iff ``digits`` satisfies the Luhn checksum."""
    cleaned = [c for c in digits if c.isdigit()]
    if len(cleaned) < 12:
        return False
    total = 0
    parity = len(cleaned) % 2
    for i, c in enumerate(cleaned):
        d = int(c)
        if i % 2 == parity:
            d *= 2
            if d > 9:
                d -= 9
        total += d
    return total % 10 == 0


class RegexPiiScrubber:
    """Production-safe regex scrubber.

    Hardened by the M07 audit (H1) :

    * **Ordering** — specific patterns run before generic ones so the
      phone regex cannot eat a credit card number.
    * **Luhn** — credit_card matches go through a Luhn checksum.
    * **Octet range** — IP addresses only match 0-255 per octet.
    * **IBAN country allowlist** — only recognised country codes match.
    * **Phone anchoring** — phone candidates must start/end outside a
      word/digit run (prevents matching the middle of an IBAN).
    """

    def __init__(self, settings: PiiSettings) -> None:
        self._settings = settings
        enabled = {c for c in settings.categories if c in _PATTERNS}
        self._enabled = tuple(c for c in _ORDERED_CATS if c in enabled)
        self._salt = bytes.fromhex(settings.salt_hex)

    def scrub(self, text: str) -> ScrubResult:
        if not text:
            return ScrubResult(scrubbed_text=text)
        scrubbed = text
        counts: dict[str, int] = {}
        for cat in self._enabled:
            pattern = _PATTERNS[cat]

            def _repl(
                m: re.Match[str],
                cat_name: str = cat,
                counts: dict[str, int] = counts,
            ) -> str:
                match_text = m.group(0)
                if not self._category_valid(cat_name, match_text):
                    return match_text
                counts[cat_name] = counts.get(cat_name, 0) + 1
                return self._replacement(cat_name, match_text)

            scrubbed = pattern.sub(_repl, scrubbed)
        flags = tuple(
            PiiFlag(category=c, count=n) for c, n in sorted(counts.items())
        )
        return ScrubResult(scrubbed_text=scrubbed, flags=flags)

    def _category_valid(self, cat: str, match: str) -> bool:
        if cat == "credit_card":
            return _luhn_check(match)
        if cat == "iban":
            return match[:2] in _IBAN_COUNTRIES
        return True

    def _replacement(self, category: str, match: str) -> str:
        if self._settings.mode == "redact":
            return f"[{category.upper()}_REDACTED]"
        if self._settings.mode == "hash":
            digest = hashlib.sha256(self._salt + match.encode("utf-8")).hexdigest()
            return f"[{category.upper()}_{digest[:8]}]"
        # mode == 'block' is handled by the caller via ScrubResult.has_pii
        return match


def iter_categories(scrubber: PiiScrubber) -> Iterable[str]:
    if isinstance(scrubber, RegexPiiScrubber):
        return scrubber._enabled
    return ()


__all__ = [
    "PiiFlag",
    "PiiScrubber",
    "RegexPiiScrubber",
    "ScrubResult",
    "iter_categories",
]

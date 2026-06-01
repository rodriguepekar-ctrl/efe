"""PII classifier — regex detectors with NFKC normalisation.

Detected kinds : email, phone, IBAN, SSN-US, credit_card (Luhn), IPv4,
IPv6, date-of-birth (FR/ISO). Unicode is normalised (NFKC) before
matching to prevent full-width/homoglyph bypass.
"""

from __future__ import annotations

import enum
import re
import unicodedata
from dataclasses import dataclass, field
from typing import Iterable

__all__ = [
    "Classification",
    "PiiClassifier",
    "PiiHit",
    "PiiKind",
]


class Classification(str, enum.Enum):
    public = "public"
    internal = "internal"
    pii = "pii"
    phi = "phi"
    secret = "secret"


class PiiKind(str, enum.Enum):
    email = "email"
    phone = "phone"
    iban = "iban"
    ssn_us = "ssn_us"
    credit_card = "credit_card"
    ipv4 = "ipv4"
    ipv6 = "ipv6"
    dob = "dob"
    name = "name"  # heuristic, opt-in


# ---------------------------------------------------------------------------
# Regexes (compiled lazily via module-level cache)
# ---------------------------------------------------------------------------

_EMAIL_RE = re.compile(r"[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}", re.IGNORECASE)
_PHONE_RE = re.compile(
    r"""
    (?:
      \+?\d{1,3}[\s.\-]?      # country code
    )?
    (?:\(?\d{1,4}\)?[\s.\-]?) # area
    (?:\d{2,4}[\s.\-]?){2,4}  # rest
    \d{2,4}
    """,
    re.VERBOSE,
)
_IBAN_RE = re.compile(r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b", re.IGNORECASE)
_SSN_US_RE = re.compile(r"\b\d{3}-\d{2}-\d{4}\b")
_CC_RE = re.compile(r"\b(?:\d[ -]?){13,19}\b")
_IPV4_RE = re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")
_IPV6_RE = re.compile(r"\b(?:[0-9a-f]{1,4}:){2,7}[0-9a-f]{1,4}\b", re.IGNORECASE)
_DOB_RE = re.compile(
    r"\b(?:\d{4}-\d{2}-\d{2}|\d{2}/\d{2}/\d{4})\b"
)


# ---------------------------------------------------------------------------
# Luhn for credit-card validation
# ---------------------------------------------------------------------------


def _luhn_ok(digits: str) -> bool:
    cleaned = [int(c) for c in digits if c.isdigit()]
    if not (13 <= len(cleaned) <= 19):
        return False
    total = 0
    for i, d in enumerate(reversed(cleaned)):
        if i % 2 == 1:
            d *= 2
            if d > 9:
                d -= 9
        total += d
    return total % 10 == 0


# ---------------------------------------------------------------------------
# Hit
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class PiiHit:
    kind: PiiKind
    classification: Classification
    start: int
    end: int
    matched: str


# ---------------------------------------------------------------------------
# Classifier
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class PiiClassifier:
    """Stateless classifier ; all state lives in :class:`PiiPolicy`."""

    enabled_kinds: frozenset[PiiKind] = field(
        default_factory=lambda: frozenset(
            {
                PiiKind.email,
                PiiKind.phone,
                PiiKind.iban,
                PiiKind.ssn_us,
                PiiKind.credit_card,
                PiiKind.ipv4,
                PiiKind.ipv6,
                PiiKind.dob,
            }
        )
    )
    max_depth: int = 10
    min_phone_digits: int = 7

    # ------------------------------------------------------------------
    # Normalisation
    # ------------------------------------------------------------------

    @staticmethod
    def normalise(s: str) -> str:
        return unicodedata.normalize("NFKC", s)

    # ------------------------------------------------------------------
    # Main entry points
    # ------------------------------------------------------------------

    def find(self, text: str) -> list[PiiHit]:
        if not text:
            return []
        norm = self.normalise(text)
        hits: list[PiiHit] = []
        if PiiKind.email in self.enabled_kinds:
            for m in _EMAIL_RE.finditer(norm):
                hits.append(
                    PiiHit(PiiKind.email, Classification.pii, m.start(), m.end(), m.group(0))
                )
        if PiiKind.iban in self.enabled_kinds:
            for m in _IBAN_RE.finditer(norm):
                hits.append(PiiHit(PiiKind.iban, Classification.pii, m.start(), m.end(), m.group(0)))
        if PiiKind.ssn_us in self.enabled_kinds:
            for m in _SSN_US_RE.finditer(norm):
                hits.append(PiiHit(PiiKind.ssn_us, Classification.pii, m.start(), m.end(), m.group(0)))
        if PiiKind.credit_card in self.enabled_kinds:
            for m in _CC_RE.finditer(norm):
                if _luhn_ok(m.group(0)):
                    hits.append(
                        PiiHit(PiiKind.credit_card, Classification.pii, m.start(), m.end(), m.group(0))
                    )
        if PiiKind.ipv4 in self.enabled_kinds:
            for m in _IPV4_RE.finditer(norm):
                hits.append(PiiHit(PiiKind.ipv4, Classification.pii, m.start(), m.end(), m.group(0)))
        if PiiKind.ipv6 in self.enabled_kinds:
            for m in _IPV6_RE.finditer(norm):
                hits.append(PiiHit(PiiKind.ipv6, Classification.pii, m.start(), m.end(), m.group(0)))
        if PiiKind.dob in self.enabled_kinds:
            for m in _DOB_RE.finditer(norm):
                hits.append(PiiHit(PiiKind.dob, Classification.pii, m.start(), m.end(), m.group(0)))
        if PiiKind.phone in self.enabled_kinds:
            for m in _PHONE_RE.finditer(norm):
                match = m.group(0)
                digits = sum(c.isdigit() for c in match)
                if digits >= self.min_phone_digits:
                    hits.append(
                        PiiHit(PiiKind.phone, Classification.pii, m.start(), m.end(), match)
                    )
        # Deduplicate overlapping hits — keep the longest.
        hits.sort(key=lambda h: (h.start, -h.end))
        filtered: list[PiiHit] = []
        last_end = -1
        for h in hits:
            if h.start >= last_end:
                filtered.append(h)
                last_end = h.end
        return filtered

    def classify(self, text: str) -> Classification:
        return Classification.pii if self.find(text) else Classification.internal

    def walk(self, obj: object, *, depth: int = 0) -> Iterable[tuple[str, PiiHit]]:
        """Walk an arbitrary dict/list tree and yield ``(path, hit)``.

        Depth-limited for safety.
        """
        if depth > self.max_depth:
            return
        if isinstance(obj, str):
            for h in self.find(obj):
                yield "", h
        elif isinstance(obj, dict):
            for k, v in obj.items():
                for path, h in self.walk(v, depth=depth + 1):
                    yield (f"{k}.{path}" if path else str(k)), h
        elif isinstance(obj, (list, tuple)):
            for i, v in enumerate(obj):
                for path, h in self.walk(v, depth=depth + 1):
                    yield (f"[{i}].{path}" if path else f"[{i}]"), h

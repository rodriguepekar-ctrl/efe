"""PII scrubbing — ordered regex pipeline with mask-or-reject policy.

Regexes are **intentionally conservative**: we tolerate a small false
positive rate (e.g. catching an IBAN-looking product reference) to
keep false negatives near zero. The canonical fixture used to validate
this module lives in ``tests_ft/fixtures/pii_samples.json``.

Supported kinds (extend with care — each entry must be CI-backed):
    email | iban | phone_fr | phone_us | cc | ssn_fr
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Literal

from efe_ft.errors import PiiBlocked

Policy = Literal["mask", "reject"]


@dataclass(frozen=True, slots=True)
class Hit:
    """One PII match in a text."""

    kind: str
    start: int
    end: int


# Order matters: more-specific patterns first to avoid overlap.
# SSN_FR comes BEFORE `cc` because its 15-digit pattern would also
# match the broader 13..19-digit CC regex. Specific-wins policy.
_PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    ("email", re.compile(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,24}")),
    ("iban", re.compile(r"\b[A-Z]{2}\d{2}(?:[ ]?[A-Z0-9]{4}){3,7}\b")),
    # French SSN (NIR): 1|2 YY MM DD PPP NNN CC (15 digits with optional spaces).
    ("ssn_fr", re.compile(
        r"(?<!\d)[12][ ]?\d{2}[ ]?\d{2}[ ]?\d{2}[ ]?\d{3}[ ]?\d{3}[ ]?\d{2}(?!\d)"
    )),
    ("phone_fr", re.compile(r"(?<!\d)(?:\+33|0)[1-9](?:[ .\-]?\d{2}){4}(?!\d)")),
    ("phone_us", re.compile(r"(?<!\d)(?:\+?1[ \-.]?)?\(?\d{3}\)?[ \-.]?\d{3}[ \-.]?\d{4}(?!\d)")),
    # Credit card Luhn-ish (13..19 digits with optional spaces / dashes).
    ("cc", re.compile(r"(?<!\d)(?:\d[ \-]?){13,19}(?!\d)")),
]


def find_pii(text: str) -> list[Hit]:
    """Return every match as a ``Hit`` ordered by position.

    Overlapping matches are resolved by pattern priority: if two
    patterns claim the same range, the one declared earlier in
    ``_PATTERNS`` wins.
    """
    hits: list[Hit] = []
    claimed: list[tuple[int, int]] = []
    for kind, pat in _PATTERNS:
        for m in pat.finditer(text):
            start, end = m.start(), m.end()
            if any(not (end <= s or start >= e) for s, e in claimed):
                continue
            hits.append(Hit(kind=kind, start=start, end=end))
            claimed.append((start, end))
    hits.sort(key=lambda h: h.start)
    return hits


def scrub(text: str, *, policy: Policy = "mask") -> tuple[str, list[str]]:
    """Apply the scrub policy.

    * ``mask`` — replace each hit by ``<KIND>`` (preserving length roughly).
    * ``reject`` — raise :class:`PiiBlocked` if any hit found.

    Returns ``(scrubbed_text, distinct_kinds)``.
    """
    hits = find_pii(text)
    if not hits:
        return text, []
    if policy == "reject":
        raise PiiBlocked(
            f"pii_blocked: {sorted({h.kind for h in hits})}",
            detail={"kinds": sorted({h.kind for h in hits})},
        )
    # mask — replace from the end so indices stay valid
    out = list(text)
    for h in sorted(hits, key=lambda h: h.end, reverse=True):
        out[h.start:h.end] = f"<{h.kind.upper()}>"
    return "".join(out), sorted({h.kind for h in hits})


__all__ = ["Hit", "Policy", "find_pii", "scrub"]

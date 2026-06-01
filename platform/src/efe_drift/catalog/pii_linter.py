"""PII linter — nightly scanner that enforces invariant I6.

M09 architecture §14.3 states :

> « Les ``feature_snapshots`` ne contiennent que des **histogrammes
> agrégés** ou des **hash-bins**. Aucune valeur brute (prompt, réponse
> utilisateur) n'est persistée au-delà de l'embedding vectoriel
> normalisé. Un lint-job quotidien vérifie l'absence de chaînes de
> caractères "brutes". »

This module is the lint job. It scans :class:`FeatureSnapshot` rows
(or any dict containing ``histogram_json`` / ``stats_json`` /
``categorical_counts_json``) for signs of PII :

* email addresses (RFC-5322 simplified regex) ;
* phone numbers (international format) ;
* IBANs (country code + MOD-97 skeleton) ;
* numeric identifiers of ≥ N digits (configurable) ;
* URLs (potential redirection targets) ;
* bearer tokens / JWTs ;
* long hex strings that look like secrets (≥ 32 hex) ;
* base64 blobs ≥ 64 chars.

When a hit is found the linter raises :class:`PiiDetected` (strict
mode) or returns a :class:`PiiReport` (audit mode).

The linter is **configurable** : in prod it runs in audit mode and
feeds a Prometheus counter + a NATS event. In tests we use strict
mode to make violations loud.
"""

from __future__ import annotations

import re
from collections.abc import Mapping
from dataclasses import asdict, dataclass, field
from typing import Any

from efe_drift.errors import PiiDetected

# ---------------------------------------------------------------------------
# Patterns
# ---------------------------------------------------------------------------

_EMAIL_RE = re.compile(
    r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
)

# International phone : optional +, 9–15 digits possibly separated by
# spaces / dashes. Deliberately permissive — false positives are
# cheaper than false negatives on this axis.
_PHONE_RE = re.compile(
    r"(?:\+?\d[\d \-]{8,14}\d)",
)

# IBAN : ISO 3166-1 alpha-2 + 2 check digits + up to 30 alphanumerics.
_IBAN_RE = re.compile(
    r"\b[A-Z]{2}\d{2}[A-Z0-9]{10,30}\b",
)

# URL : http(s)://host + optional path.
_URL_RE = re.compile(
    r"https?://[^\s\"']+",
)

# JWT : three base64url segments separated by dots.
_JWT_RE = re.compile(
    r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
)

# Long hex strings — suspected secrets / API keys.
_LONG_HEX_RE = re.compile(
    r"\b[0-9a-fA-F]{32,}\b",
)

# Base64 blobs (≥ 64 chars, padded optional).
_BASE64_RE = re.compile(
    r"\b[A-Za-z0-9+/]{64,}={0,2}\b",
)


@dataclass(frozen=True, slots=True)
class PiiHit:
    category: str
    excerpt: str           # 32-char truncated + ellipsis
    location: str          # path within the scanned structure

    @classmethod
    def new(cls, category: str, raw: str, location: str) -> PiiHit:
        truncated = raw if len(raw) <= 32 else f"{raw[:29]}..."
        return cls(category=category, excerpt=truncated, location=location)


@dataclass(frozen=True, slots=True)
class PiiReport:
    hits: tuple[PiiHit, ...] = field(default_factory=tuple)

    @property
    def clean(self) -> bool:
        return len(self.hits) == 0

    @property
    def categories(self) -> frozenset[str]:
        return frozenset(h.category for h in self.hits)


# ---------------------------------------------------------------------------
# PiiLinter
# ---------------------------------------------------------------------------


class PiiLinter:
    """Scans arbitrary :class:`Mapping` / string payloads for PII.

    Usage :

    >>> linter = PiiLinter(numeric_id_min_digits=10)
    >>> report = linter.scan_payload({"categorical_counts": {
    ...     "alice@example.com": 17,
    ...     "bob": 25,
    ... }})
    >>> report.clean
    False
    >>> "email" in report.categories
    True
    """

    __slots__ = ("_numeric_id_re",)

    def __init__(self, *, numeric_id_min_digits: int = 10) -> None:
        if numeric_id_min_digits < 6 or numeric_id_min_digits > 32:
            raise ValueError(
                "numeric_id_min_digits must be in [6, 32]",
            )
        self._numeric_id_re = re.compile(
            rf"\b\d{{{numeric_id_min_digits},}}\b",
        )

    # ---- Public API ----------------------------------------------------

    def scan_payload(
        self,
        payload: Mapping[str, Any] | str,
        *,
        strict: bool = False,
        root: str = "$",
    ) -> PiiReport:
        hits: list[PiiHit] = []
        self._walk(payload, hits, root)
        if strict and hits:
            raise PiiDetected(
                f"PII detected in payload: "
                f"{sorted({h.category for h in hits})}",
                detail={"hits": [asdict(h) for h in hits]},
            )
        return PiiReport(hits=tuple(hits))

    # ---- Internal ------------------------------------------------------

    def _walk(
        self,
        value: Any,
        hits: list[PiiHit],
        path: str,
    ) -> None:
        if isinstance(value, str):
            self._scan_string(value, hits, path)
        elif isinstance(value, Mapping):
            for key, sub in value.items():
                key_str = str(key)
                # Keys are also scanned (category names are the most
                # common leak vector in categorical histograms).
                self._scan_string(key_str, hits, f"{path}.{key_str}")
                self._walk(sub, hits, f"{path}.{key_str}")
        elif isinstance(value, (list, tuple)):
            for i, sub in enumerate(value):
                self._walk(sub, hits, f"{path}[{i}]")
        # numeric / bool / None — skipped; never carries PII on its own

    def _scan_string(
        self,
        value: str,
        hits: list[PiiHit],
        path: str,
    ) -> None:
        if not value or len(value) < 4:
            return
        # Long-pattern detectors run first so we tag specific
        # categories rather than the generic "numeric_id".
        for cat, pattern in (
            ("email", _EMAIL_RE),
            ("iban", _IBAN_RE),
            ("jwt", _JWT_RE),
            ("url", _URL_RE),
            ("long_hex", _LONG_HEX_RE),
            ("base64_blob", _BASE64_RE),
            ("phone", _PHONE_RE),
            ("numeric_id", self._numeric_id_re),
        ):
            for match in pattern.finditer(value):
                hits.append(PiiHit.new(cat, match.group(0), path))


__all__ = ["PiiHit", "PiiLinter", "PiiReport"]

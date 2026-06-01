"""Masker — transform PII hits in strings and objects.

Two modes :

* ``redact`` — replace by ``[PII:<kind>]``.
* ``partial`` — keep first / last N chars of the match, star the middle.
"""

from __future__ import annotations

import enum
from dataclasses import dataclass
from typing import Any

from efe_security.pii.classifier import PiiClassifier, PiiHit, PiiKind

__all__ = ["MaskMode", "Masker", "scrub"]


class MaskMode(str, enum.Enum):
    redact = "redact"
    partial = "partial"


@dataclass(slots=True)
class Masker:
    classifier: PiiClassifier
    mode: MaskMode = MaskMode.redact
    keep_chars: int = 2

    def mask_text(self, text: str) -> str:
        if not text:
            return text
        hits = self.classifier.find(text)
        if not hits:
            return text
        # Walk the string, splicing replacements in reverse to preserve indices.
        result = list(text)
        for h in reversed(hits):
            replacement = self._replacement(h)
            result[h.start : h.end] = list(replacement)
        return "".join(result)

    def _replacement(self, h: PiiHit) -> str:
        if self.mode == MaskMode.redact:
            return f"[PII:{h.kind.value}]"
        # partial mode
        keep = max(0, self.keep_chars)
        if len(h.matched) <= 2 * keep:
            return "*" * len(h.matched)
        return h.matched[:keep] + "*" * (len(h.matched) - 2 * keep) + h.matched[-keep:]

    def mask_object(self, obj: Any) -> Any:
        if isinstance(obj, str):
            return self.mask_text(obj)
        if isinstance(obj, dict):
            return {k: self.mask_object(v) for k, v in obj.items()}
        if isinstance(obj, list):
            return [self.mask_object(v) for v in obj]
        if isinstance(obj, tuple):
            return tuple(self.mask_object(v) for v in obj)
        return obj


# Convenience single-shot helper
def scrub(
    obj: Any,
    *,
    classifier: PiiClassifier | None = None,
    mode: MaskMode = MaskMode.redact,
) -> Any:
    m = Masker(classifier=classifier or PiiClassifier(), mode=mode)
    return m.mask_object(obj)


_ = PiiHit, PiiKind

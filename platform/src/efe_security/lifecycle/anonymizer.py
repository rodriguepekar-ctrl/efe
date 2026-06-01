"""Irreversible anonymisation helpers (non-cryptographic)."""

from __future__ import annotations

from efe_security.pii.classifier import PiiClassifier
from efe_security.pii.masker import MaskMode, Masker

__all__ = ["anonymize_text"]


def anonymize_text(text: str) -> str:
    """Redact known PII kinds in one call. One-way — no DEK involved."""
    classifier = PiiClassifier()
    return Masker(classifier=classifier, mode=MaskMode.redact).mask_text(text)

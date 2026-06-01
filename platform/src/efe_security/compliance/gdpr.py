"""GDPR article → M11 subsystem mapping."""

from __future__ import annotations

from dataclasses import dataclass

__all__ = ["GDPR_MAPPING", "GdprArticle"]


@dataclass(frozen=True, slots=True)
class GdprArticle:
    number: str
    title: str
    efe_controls: tuple[str, ...]


GDPR_MAPPING: tuple[GdprArticle, ...] = (
    GdprArticle("5", "Principles", ("pii", "consent", "lifecycle")),
    GdprArticle("6", "Lawful basis", ("consent",)),
    GdprArticle("15", "Right of access", ("dsar",)),
    GdprArticle("16", "Right to rectification", ("dsar",)),
    GdprArticle("17", "Right to erasure", ("dsar", "lifecycle")),
    GdprArticle("25", "Privacy by design", ("principles",)),
    GdprArticle("30", "Records of processing", ("audit",)),
    GdprArticle("32", "Security", ("secrets", "audit")),
    GdprArticle("33", "Breach notification", ("incident",)),
    GdprArticle("35", "DPIA", ("threat_model",)),
)

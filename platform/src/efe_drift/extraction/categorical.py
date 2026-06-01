"""Categorical extractors.

Includes :

* :class:`LanguageDetectExtractor` — uses a pluggable detector
  callable (``text -> lang_code``) injected via the constructor. This
  keeps fasttext / langdetect out of the module's import graph (they
  are optional runtime deps). A tiny built-in heuristic is provided
  for tests.

* :class:`HasCodeExtractor` — detects fenced code blocks or obvious
  keywords (``def``, ``class``, ``function``, ``SELECT``). Returns a
  binary value.

* :class:`CountryExtractor` — reads ``event.covariates["country"]``
  if present, uppercased. No default inference.

* :class:`ModelVariantExtractor` — mirrors ``event.model_id``.
"""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass
from typing import ClassVar

from efe_drift.extraction.base import (
    binary,
    categorical,
)
from efe_drift.records import FeatureKind, FeatureValue, InferenceEvent

# ---------------------------------------------------------------------------
# Built-in fallback language detector — purely heuristic, meant for
# tests + development. In prod the caller wires a real detector.
# ---------------------------------------------------------------------------


_ASCII_LETTERS_RE = re.compile(r"[A-Za-z]")
_LATIN_ACCENTED_RE = re.compile(r"[àâäçéèêëîïôöùûüÿæœÀÂÄÇÉÈÊËÎÏÔÖÙÛÜŸÆŒ]")
_SPANISH_RE = re.compile(r"[ñÑ¿¡]")
_GERMAN_RE = re.compile(r"[äöüßÄÖÜ]")
_CJK_RE = re.compile(r"[\u3400-\u9fff]")
_ARABIC_RE = re.compile(r"[\u0600-\u06FF]")
_CYRILLIC_RE = re.compile(r"[\u0400-\u04FF]")


def simple_heuristic_language(text: str, *, min_chars: int = 8) -> str | None:
    """Tiny fallback language detector — good enough for unit tests.

    Returns ``None`` when the text is too short. Real deployments
    should use fasttext 176-language / LangDetect / cld3.
    """
    stripped = text.strip()
    if len(stripped) < min_chars:
        return None
    if _CJK_RE.search(stripped):
        return "zh"
    if _ARABIC_RE.search(stripped):
        return "ar"
    if _CYRILLIC_RE.search(stripped):
        return "ru"
    if _SPANISH_RE.search(stripped):
        return "es"
    if _GERMAN_RE.search(stripped):
        return "de"
    if _LATIN_ACCENTED_RE.search(stripped):
        return "fr"
    if _ASCII_LETTERS_RE.search(stripped):
        return "en"
    return None


@dataclass(frozen=True, slots=True)
class LanguageDetectExtractor:
    """Per-event language detection.

    The ``detector`` callable is injected. In tests we inject
    :func:`simple_heuristic_language`; in prod we inject a fasttext
    classifier wrapper.
    """

    min_chars: int = 8
    detector: Callable[[str, int], str | None] | None = None
    name: str = "lang_detect"
    kind: FeatureKind = "categorical"
    feature_name: str = "language"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        text = event.prompt_text
        if not text:
            return None
        fn = self.detector or self._default_detector
        lang = fn(text, self.min_chars)
        return categorical(self.feature_name, lang)

    @staticmethod
    def _default_detector(text: str, min_chars: int) -> str | None:
        return simple_heuristic_language(text, min_chars=min_chars)


# ---------------------------------------------------------------------------
# has_code — binary heuristic
# ---------------------------------------------------------------------------


_CODE_FENCE_RE = re.compile(r"```")
_INDENT_CODE_RE = re.compile(r"^(?: {4,}|\t)\S", re.MULTILINE)
_CODE_KEYWORDS_RE = re.compile(
    r"\b(def|class|function|return|import|SELECT|INSERT|UPDATE|"
    r"DELETE|CREATE|public class|if __name__|console\.log)\b",
    re.IGNORECASE,
)


@dataclass(frozen=True, slots=True)
class HasCodeExtractor:
    """Binary : ``1`` if the prompt looks like it contains code."""

    name: str = "has_code_heuristic"
    kind: FeatureKind = "binary"
    feature_name: str = "has_code"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        text = event.prompt_text
        if not text:
            return None
        has_fence = bool(_CODE_FENCE_RE.search(text))
        has_indent = bool(_INDENT_CODE_RE.search(text))
        has_keyword = bool(_CODE_KEYWORDS_RE.search(text))
        flag = has_fence or has_indent or has_keyword
        return binary(self.feature_name, flag)


# ---------------------------------------------------------------------------
# Country / model variant — reads from structured fields, no NLP
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class CountryExtractor:
    """Pulls ``country`` from ``event.covariates`` (two-letter code)."""

    name: str = "country"
    kind: FeatureKind = "categorical"
    feature_name: str = "country"
    KEY: ClassVar[str] = "country"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        raw = event.covariates.get(self.KEY) if event.covariates else None
        if not isinstance(raw, str):
            return None
        normalised = raw.strip().upper()
        if len(normalised) != 2 or not normalised.isalpha():
            return None
        return categorical(self.feature_name, normalised)


@dataclass(frozen=True, slots=True)
class ModelVariantExtractor:
    name: str = "model_variant"
    kind: FeatureKind = "categorical"
    feature_name: str = "model_variant"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        return categorical(self.feature_name, event.model_id)


__all__ = [
    "CountryExtractor",
    "HasCodeExtractor",
    "LanguageDetectExtractor",
    "ModelVariantExtractor",
    "simple_heuristic_language",
]

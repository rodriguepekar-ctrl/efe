"""MistralBackend — OpenAI-compatible with a few quirks.

Mistral's hosted API mostly mirrors OpenAI chat completions, so we
inherit :class:`OpenAiBackend`'s payload shape and override only the
URL + tool choice mapping.
"""

from __future__ import annotations

from dataclasses import dataclass

from efe_inference.backends.openai import OpenAiBackend
from efe_inference.config import BackendKind


@dataclass
class MistralBackend(OpenAiBackend):
    kind: BackendKind = BackendKind.MISTRAL
    base_url: str = "https://api.mistral.ai/v1"


__all__ = ["MistralBackend"]

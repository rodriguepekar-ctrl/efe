"""Prompt templating — turn canonical ChatRequest into family-specific prompts."""

from __future__ import annotations

from efe_inference.templating.families import (
    BUILTIN_TEMPLATES,
    ModelFamily,
    family_for_backend,
)
from efe_inference.templating.templater import (
    PromptTemplater,
    RenderedPrompt,
)

__all__ = [
    "BUILTIN_TEMPLATES",
    "ModelFamily",
    "PromptTemplater",
    "RenderedPrompt",
    "family_for_backend",
]

"""ModelFamily enum + built-in templates.

The architecture document lists 5 families (``claude``, ``gpt``,
``llama3``, ``mistral``, ``gemini``); the built-in templates below
cover the three that are not delivered by an SDK (``llama3``,
``mistral`` via ChatML, ``chatml`` alias). The SDK-delivered families
(``claude``, ``gpt``, ``gemini``) pass the canonical message array
straight to the backend SDK and do not need a textual template.
"""

from __future__ import annotations

from enum import Enum
from typing import Final

from efe_inference.config import BackendKind


class ModelFamily(str, Enum):
    LLAMA3 = "llama3"
    MISTRAL = "mistral"
    CHATML = "chatml"
    CLAUDE = "claude"
    GPT = "gpt"
    GEMINI = "gemini"


# ---------------------------------------------------------------------------
# Built-in Jinja2 templates
# ---------------------------------------------------------------------------


_LLAMA3_TEMPLATE: Final[str] = (
    "<|begin_of_text|>"
    "{%- for m in messages -%}"
    "<|start_header_id|>{{ m.role }}<|end_header_id|>\n\n"
    "{{ m.text }}<|eot_id|>"
    "{%- endfor -%}"
    "<|start_header_id|>assistant<|end_header_id|>\n\n"
)

_CHATML_TEMPLATE: Final[str] = (
    "{%- for m in messages -%}"
    "<|im_start|>{{ m.role }}\n{{ m.text }}<|im_end|>\n"
    "{%- endfor -%}"
    "<|im_start|>assistant\n"
)

_MISTRAL_V3_TEMPLATE: Final[str] = (
    "<s>"
    "{%- for m in messages -%}"
    "{%- if m.role == 'system' -%}"
    "[INST] {{ m.text }} [/INST]\n"
    "{%- elif m.role == 'user' -%}"
    "[INST] {{ m.text }} [/INST]"
    "{%- elif m.role == 'assistant' -%}"
    " {{ m.text }} </s>"
    "{%- endif -%}"
    "{%- endfor -%}"
)


BUILTIN_TEMPLATES: Final[dict[tuple[ModelFamily, str], str]] = {
    (ModelFamily.LLAMA3, "1.0"): _LLAMA3_TEMPLATE,
    (ModelFamily.CHATML, "1.0"): _CHATML_TEMPLATE,
    (ModelFamily.MISTRAL, "1.0"): _MISTRAL_V3_TEMPLATE,
}


# ---------------------------------------------------------------------------
# Backend-kind to family mapping
# ---------------------------------------------------------------------------


_KIND_TO_FAMILY: Final[dict[BackendKind, ModelFamily]] = {
    BackendKind.VLLM_LOCAL: ModelFamily.LLAMA3,
    BackendKind.PEFT_LOCAL: ModelFamily.LLAMA3,
    BackendKind.ANTHROPIC: ModelFamily.CLAUDE,
    BackendKind.OPENAI: ModelFamily.GPT,
    BackendKind.MISTRAL: ModelFamily.MISTRAL,
    BackendKind.MOCK: ModelFamily.GPT,
}


def family_for_backend(
    kind: BackendKind, *, override: str | None = None,
) -> ModelFamily:
    """Return the canonical family for a given backend kind."""
    if override:
        try:
            return ModelFamily(override)
        except ValueError:
            pass
    return _KIND_TO_FAMILY.get(kind, ModelFamily.GPT)


__all__ = [
    "BUILTIN_TEMPLATES",
    "ModelFamily",
    "family_for_backend",
]

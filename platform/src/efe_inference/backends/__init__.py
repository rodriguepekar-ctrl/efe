"""Backend adapters — vLLM / Claude / GPT / Mistral / PEFT / Mock."""

from __future__ import annotations

from efe_inference.backends.claude import ClaudeBackend
from efe_inference.backends.mistral import MistralBackend
from efe_inference.backends.mock import (
    MockBackend,
    ScriptedBackend,
    ScriptedResponse,
)
from efe_inference.backends.openai import OpenAiBackend
from efe_inference.backends.peft import PeftBackend
from efe_inference.backends.protocol import (
    BackendAdapter,
    BackendRequest,
    BackendResponse,
    HealthStatus,
)
from efe_inference.backends.registry import BackendRegistry
from efe_inference.backends.selector import (
    BackendSelector,
    FallbackChain,
    FallbackEntry,
)
from efe_inference.backends.vllm import VllmBackend

__all__ = [
    "BackendAdapter",
    "BackendRegistry",
    "BackendRequest",
    "BackendResponse",
    "BackendSelector",
    "ClaudeBackend",
    "FallbackChain",
    "FallbackEntry",
    "HealthStatus",
    "MistralBackend",
    "MockBackend",
    "OpenAiBackend",
    "PeftBackend",
    "ScriptedBackend",
    "ScriptedResponse",
    "VllmBackend",
]

"""EFE Inference Adapter — polyglot LLM façade.

Public surface::

    from efe_inference import InferenceAdapter, ChatRequest, ChatMessage

    adapter = InferenceAdapter.from_settings(settings)
    response = await adapter.generate(ChatRequest(...))

Responsibilities (see M07_ARCHITECTURE.md for full detail):

* Normalize OpenAI / Anthropic / native request formats.
* Dispatch to vLLM-local (via M06) / Claude / GPT / Mistral / PEFT-local.
* Apply PII scrub + content moderation + prompt-injection detection.
* Cache (verbatim + semantic, strict tenant partition).
* Resilience (retry + circuit-breaker + fallback chain).
* Exact token accounting + cost tracking.
* Signed safety audit trail.

This package is deliberately small at the surface — all heavy logic
lives in sub-packages (``backends``, ``safety``, ``cache``,
``resilience`` …).
"""

from __future__ import annotations

from efe_inference.adapter import InferenceAdapter
from efe_inference.config import (
    AccountingSettings,
    BackendConfig,
    BackendKind,
    CacheSettings,
    DatabaseSettings,
    InferenceSettings,
    NatsSettings,
    ResilienceSettings,
    SafetySettings,
)
from efe_inference.messages.request import (
    ChatMessage,
    ChatRequest,
    ContentBlock,
    ImageBlock,
    SamplingParams,
    TextBlock,
    ToolDef,
    ToolResultBlock,
    ToolUseBlock,
)
from efe_inference.messages.response import (
    ChatChoice,
    ChatChunk,
    ChatResponse,
    FinishReason,
    Usage,
)
from efe_inference.templating.families import ModelFamily
from efe_inference.version import __version__

__all__ = [
    "AccountingSettings",
    "BackendConfig",
    "BackendKind",
    "CacheSettings",
    "ChatChoice",
    "ChatChunk",
    "ChatMessage",
    "ChatRequest",
    "ChatResponse",
    "ContentBlock",
    "DatabaseSettings",
    "FinishReason",
    "ImageBlock",
    "InferenceAdapter",
    "InferenceSettings",
    "ModelFamily",
    "NatsSettings",
    "ResilienceSettings",
    "SafetySettings",
    "SamplingParams",
    "TextBlock",
    "ToolDef",
    "ToolResultBlock",
    "ToolUseBlock",
    "Usage",
    "__version__",
]

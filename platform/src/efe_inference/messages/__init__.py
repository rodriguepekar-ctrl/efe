"""Canonical request / response types + normalisation."""

from __future__ import annotations

from efe_inference.messages.normalizer import (
    AnthropicNormalizer,
    NativeNormalizer,
    OpenAINormalizer,
    RequestNormalizer,
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
    ToolCall,
    Usage,
)

__all__ = [
    "AnthropicNormalizer",
    "ChatChoice",
    "ChatChunk",
    "ChatMessage",
    "ChatRequest",
    "ChatResponse",
    "ContentBlock",
    "FinishReason",
    "ImageBlock",
    "NativeNormalizer",
    "OpenAINormalizer",
    "RequestNormalizer",
    "SamplingParams",
    "TextBlock",
    "ToolCall",
    "ToolDef",
    "ToolResultBlock",
    "ToolUseBlock",
    "Usage",
]

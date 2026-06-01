"""Provider abstraction layer for efe-byo-agent (M20).

Public exports :
  - LLMProvider     : Protocol all providers must implement
  - FallbackChain   : multi-provider failover
  - CircuitBreaker  : per-provider state machine

Provider classes are lazily importable to avoid hard dependencies on
provider SDKs that the user did not install :

    from efe_byo_agent.llm import AnthropicProvider  # requires [anthropic] extra

If the corresponding extra is not installed, the import raises a
helpful ImportError with the install command.
"""
from __future__ import annotations

from typing import TYPE_CHECKING

from .base import LLMProvider
from .circuit_breaker import CircuitBreaker, CircuitState
from .fallback import FallbackChain

# Lazy provider imports : importing this package must not fail when
# optional extras are missing. Each provider module performs its own
# `import` inside the constructor.

if TYPE_CHECKING:  # pragma: no cover
    from .anthropic import AnthropicProvider
    from .azure_openai import AzureOpenAIProvider
    from .bedrock import BedrockProvider
    from .llama_cpp import LlamaCppProvider
    from .mlx import MLXProvider
    from .openai import OpenAIProvider
    from .vertex import VertexProvider
    from .vllm import VLLMProvider


def __getattr__(name: str):
    """Lazy attribute access — raises a clear ImportError on missing extras."""
    if name == "AnthropicProvider":
        from .anthropic import AnthropicProvider
        return AnthropicProvider
    if name == "OpenAIProvider":
        from .openai import OpenAIProvider
        return OpenAIProvider
    if name == "BedrockProvider":
        from .bedrock import BedrockProvider
        return BedrockProvider
    if name == "AzureOpenAIProvider":
        from .azure_openai import AzureOpenAIProvider
        return AzureOpenAIProvider
    if name == "VertexProvider":
        from .vertex import VertexProvider
        return VertexProvider
    if name == "VLLMProvider":
        from .vllm import VLLMProvider
        return VLLMProvider
    if name == "LlamaCppProvider":
        from .llama_cpp import LlamaCppProvider
        return LlamaCppProvider
    if name == "MLXProvider":
        from .mlx import MLXProvider
        return MLXProvider
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")


__all__ = [
    "LLMProvider",
    "CircuitBreaker",
    "CircuitState",
    "FallbackChain",
    # Lazy-loaded providers
    "AnthropicProvider",
    "OpenAIProvider",
    "BedrockProvider",
    "AzureOpenAIProvider",
    "VertexProvider",
    "VLLMProvider",
    "LlamaCppProvider",
    "MLXProvider",
]

"""efe-byo-agent — Bring-Your-Own-LLM agent SDK for EFE (M20).

Public surface :
  - Agent             : high-level facade
  - LLMProvider       : Protocol for provider implementations
  - FallbackChain     : multi-provider failover with circuit breaker
  - exceptions        : ProviderError, AllProvidersFailedError, etc.

Provider implementations (opt-in via extras) :
  - AnthropicProvider     (`pip install efe-agent[anthropic]`)
  - OpenAIProvider        (`pip install efe-agent[openai]`)
  - BedrockProvider       (`pip install efe-agent[bedrock]`)
  - AzureOpenAIProvider   (`pip install efe-agent[azure-openai]`)
  - VertexProvider        (`pip install efe-agent[vertex]`)
  - VLLMProvider          (no extra dep, uses OpenAI-compat)
  - LlamaCppProvider      (no extra dep, uses OpenAI-compat)
  - MLXProvider           (`pip install efe-agent[mlx]`, Mac only)

For the operator-facing helpers (summarize_classification,
compare_bundles_quality, etc.), see :
  - efe_byo_agent.helpers
  - efe_byo_agent.cli  (`efe-agent helper <name> ...`)

Default state : NOT installed in the mcp-server image. Operators do
`pip install efe-agent` separately on their workstations / CI / batch
systems. Cf. M20 architecture § 6.1.
"""
from __future__ import annotations

from .__version__ import __version__
from .agent import Agent
from .exceptions import (
    AllProvidersFailedError,
    EfeAgentError,
    MCPClientError,
    ProviderError,
    ValidationFailedError,
)
from .llm.base import LLMProvider
from .llm.fallback import FallbackChain

__all__ = [
    "__version__",
    "Agent",
    "LLMProvider",
    "FallbackChain",
    "EfeAgentError",
    "ProviderError",
    "AllProvidersFailedError",
    "MCPClientError",
    "ValidationFailedError",
]

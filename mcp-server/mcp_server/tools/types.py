"""Shared types for tool handlers (M16 + M17 extensions)."""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Awaitable, Callable, Optional

from ..auth import AuthContext
from ..backend_client import BackendClient


# Forward-declare to avoid circular import (sampling, streaming, state
# import from analysis which imports from tools indirectly).
class _SamplingClientLike:
    """Type-only stub ; replaced at runtime by mcp_server.sampling.SamplingClient."""

    available: bool


class _ProgressEmitterLike:
    """Type-only stub ; replaced at runtime by mcp_server.streaming.ProgressEmitter."""

    streaming_enabled: bool


@dataclass
class ToolContext:
    """What every tool handler receives.

    M16 carry-over : `auth` + `backend`.
    M17 additions : sampling, progress, embedding_cache, tenant_id.
    M18 additions : bundle_cache, oci_factory (lazy, per-call).
    """

    auth: AuthContext
    backend: BackendClient
    sampling: Any | None = None          # Optional[SamplingClient]
    progress: Any | None = None          # Optional[ProgressEmitter]
    embedding_cache: Any | None = None   # Optional[EmbeddingCache]
    tenant_id: str = ""
    compute_executor: Any | None = None  # Optional[ThreadPoolExecutor]
    bundle_cache: Any | None = None      # Optional[BundleCache] (M18)
    oci_factory: Any | None = None       # Optional[Callable[..., OCIRegistryClient]] (M18)

    def has_sampling(self) -> bool:
        return self.sampling is not None and getattr(self.sampling, "available", False)

    def has_streaming(self) -> bool:
        return (
            self.progress is not None
            and getattr(self.progress, "streaming_enabled", False)
        )


@dataclass
class Tool:
    """An MCP tool exposed by EFE.

    M20 addition : `feature_flag` — optional env-var name. When set,
    the tool is registered ONLY when the env-var is `true`. Tools with
    a non-null `feature_flag` are filtered out of `list_tools` AND
    rejected at `tool/call` if the flag evaluates to false. This is
    the mechanism for opt-in OFF-by-default tools (cf. roadmap § F.7).
    """

    name: str
    description: str
    input_schema: dict[str, Any]
    handler: Callable[[dict[str, Any], ToolContext], Awaitable[dict[str, Any]]]
    is_write: bool = False
    requires_sampling: bool = False
    streams_progress: bool = False
    profile_tier: str = "core"   # "core" | "advanced" | "agent-native"
    feature_flag: Optional[str] = None  # M20 : opt-in env-var gate

    def to_mcp_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
        }


class ToolError(Exception):
    """Raised by handlers when the call should fail with an MCP isError."""

    def __init__(
        self,
        message: str,
        *,
        code: str = "tool_error",
        status_code: Optional[int] = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.status_code = status_code

    def to_dict(self) -> dict[str, Any]:
        return {
            "error": str(self),
            "code": self.code,
            "status_code": self.status_code,
        }

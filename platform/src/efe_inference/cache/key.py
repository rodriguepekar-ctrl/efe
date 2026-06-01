"""CacheKeyBuilder — deterministic hash of (tenant, task, prompt, sampling)."""

from __future__ import annotations

from dataclasses import dataclass

from efe_inference.hashing import sha256_hex
from efe_inference.messages.request import ChatRequest


@dataclass(frozen=True, slots=True)
class CacheKey:
    tenant_id: str
    task_id: str
    prompt_hash_hex: str

    def redis_key(self) -> str:
        return (
            f"inf:cache:v:{self.tenant_id}:{self.task_id}:"
            f"{self.prompt_hash_hex[:16]}"
        )


class CacheKeyBuilder:
    """Build a stable cache key from canonical request fields.

    Fields included: tenant + task + sampling (temp / top_p / top_k /
    max_tokens / seed / stop) + canonical representation of messages.
    Model hint is included only when ``include_model_hint=True`` (off
    by default so the cache survives a backend fallback on otherwise
    identical prompts).
    """

    def __init__(self, *, include_model_hint: bool = False) -> None:
        self._include_model_hint = include_model_hint

    def build(self, req: ChatRequest) -> CacheKey:
        """Build a cache key that distinguishes semantically-different
        requests (H9 + C3 fixes).

        The prompt signature now serialises:

        * role + text for every :class:`TextBlock`;
        * ``[image:<mime>:<url-prefix>]`` for every :class:`ImageBlock`;
        * ``[tool_use:<id>:<name>:<canonical-args>]`` for tool_use blocks;
        * ``[tool_result:<id>:<content>:<is_error>]`` for tool_result
          blocks.

        ``tool_choice`` is included in the signature so a cached
        ``tool_choice=none`` response cannot be served when
        ``tool_choice=required`` is requested.
        """
        from efe_inference.messages.request import (
            ImageBlock,
            TextBlock,
            ToolResultBlock,
            ToolUseBlock,
        )

        parts: list[str] = []
        for m in req.messages:
            block_parts: list[str] = []
            for b in m.content:
                if isinstance(b, TextBlock):
                    block_parts.append(f"text:{b.text}")
                elif isinstance(b, ImageBlock):
                    block_parts.append(f"image:{b.mime}:{b.image_url[:64]}")
                elif isinstance(b, ToolUseBlock):
                    args = sorted(b.tool_args.items())
                    block_parts.append(
                        f"tool_use:{b.tool_call_id}:{b.tool_name}:{args}",
                    )
                elif isinstance(b, ToolResultBlock):
                    block_parts.append(
                        f"tool_result:{b.tool_call_id}:{b.content}:"
                        f"{b.is_error}",
                    )
            parts.append(f"[{m.role.value}]" + "|".join(block_parts))
        prompt_signature = "\n---\n".join(parts)
        sampling_signature = (
            f"t={req.sampling.temperature:.4f};"
            f"tp={req.sampling.top_p:.4f};"
            f"mk={req.sampling.max_tokens};"
            f"sk={req.sampling.seed};"
            f"stop={'|'.join(req.sampling.stop)}"
        )
        tools_signature = (
            "tools:" + ",".join(t.name for t in req.tools)
            + "|choice:" + req.tool_choice
        )
        model_signature = (
            f"model:{req.model_hint or 'auto'}"
            if self._include_model_hint else "model:*"
        )
        combined = "\n@@\n".join([
            prompt_signature,
            sampling_signature,
            tools_signature,
            model_signature,
        ])
        return CacheKey(
            tenant_id=req.tenant_id,
            task_id=req.task_id,
            prompt_hash_hex=sha256_hex(combined),
        )


__all__ = ["CacheKey", "CacheKeyBuilder"]

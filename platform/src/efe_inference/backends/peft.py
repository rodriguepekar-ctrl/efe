"""PeftBackend — local runtime for small tenants / fallback safe-net.

Lazy-imports ``peft`` + ``transformers`` so the outer package stays
importable on CPU-only CI. In production, this backend runs a LoRA
adapter on top of a base model with modest throughput (5-10 req/s per
A100); used as the last-resort entry in a fallback chain.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from typing import Any

from efe_inference.backends.protocol import (
    BackendRequest,
    BackendResponse,
    HealthStatus,
)
from efe_inference.config import BackendKind
from efe_inference.errors import BackendUnavailable
from efe_inference.messages.response import (
    ChatChunk,
    FinishReason,
    Usage,
)


@dataclass
class PeftBackend:
    name: str
    base_model: str
    adapter_path: str
    kind: BackendKind = BackendKind.PEFT_LOCAL
    _pipeline: Any = field(default=None, repr=False)

    async def generate(self, req: BackendRequest) -> BackendResponse:
        pipe = self._get_pipeline()
        prompt = req.textual or _msgs_to_text(req.messages or ())
        try:
            out = pipe(
                prompt,
                max_new_tokens=req.sampling.max_tokens,
                temperature=req.sampling.temperature,
                top_p=req.sampling.top_p,
                do_sample=req.sampling.temperature > 0,
                return_full_text=False,
            )
        except Exception as exc:  # pragma: no cover
            raise BackendUnavailable(f"peft inference failed: {exc}") from exc
        text = (out[0]["generated_text"] if out else "")
        pt = len(prompt.split())
        ct = len(text.split())
        return BackendResponse(
            text=text,
            finish_reason=FinishReason.STOP,
            usage=Usage(
                prompt_tokens=pt, completion_tokens=ct,
                total_tokens=pt + ct,
            ),
            raw={"peft": True},
        )

    async def stream(
        self, req: BackendRequest,
    ) -> AsyncIterator[ChatChunk]:
        resp = await self.generate(req)
        yield ChatChunk(
            delta_text=resp.text,
            finish_reason=resp.finish_reason,
            usage=resp.usage,
        )

    async def health(self) -> HealthStatus:
        try:
            self._get_pipeline()
            return HealthStatus.HEALTHY
        except Exception:
            return HealthStatus.UNHEALTHY

    async def close(self) -> None:
        self._pipeline = None

    def _get_pipeline(self) -> Any:
        if self._pipeline is not None:
            return self._pipeline
        try:
            from peft import PeftModel
            from transformers import (
                AutoModelForCausalLM,
                AutoTokenizer,
                pipeline,
            )
        except ImportError as exc:  # pragma: no cover
            raise BackendUnavailable(
                "peft/transformers not installed for PeftBackend",
            ) from exc
        try:
            tok = AutoTokenizer.from_pretrained(self.base_model)
            model = AutoModelForCausalLM.from_pretrained(self.base_model)
            from peft import PeftModel

            wrapped = PeftModel.from_pretrained(model, self.adapter_path)
            self._pipeline = pipeline(
                "text-generation", model=wrapped, tokenizer=tok,
            )
        except Exception as exc:  # pragma: no cover
            raise BackendUnavailable(
                f"peft pipeline init failed: {exc}",
            ) from exc
        return self._pipeline


def _msgs_to_text(msgs: tuple[dict[str, Any], ...]) -> str:
    parts: list[str] = []
    for m in msgs:
        role = m.get("role")
        content = m.get("content")
        if isinstance(content, str):
            parts.append(f"{role}: {content}")
        elif isinstance(content, list):
            for b in content:
                if b.get("type") == "text":
                    parts.append(f"{role}: {b.get('text','')}")
    return "\n".join(parts)


__all__ = ["PeftBackend"]

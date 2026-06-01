"""vLLM-backed serving adapter.

Lazy-imports ``vllm`` to stay test-friendly: environments without a
GPU / CUDA stack can still import :mod:`efe_registry.serving` and use
:class:`MockServingAdapter`.

L3 verification (pre-load): if a :class:`SignatureVerifier` is
supplied at construction time, :meth:`load` calls ``verify_blob`` on
``model.safetensors`` before instantiating the vLLM engine. A
verification failure raises :class:`ArtifactVerificationFailed` so a
tampered artifact can never reach GPU memory.
"""

from __future__ import annotations

import time
from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from efe_registry.config import VllmPoolSpec
from efe_registry.core.artifact import Artifact
from efe_registry.errors import ArtifactVerificationFailed, ModelLoadFailed
from efe_registry.serving.adapter import (
    CompletionRequest,
    CompletionResponse,
)
from efe_registry.signing.models import (
    VerificationInput,
    VerificationLayer,
)
from efe_registry.signing.verifier import SignatureVerifier


@dataclass
class VllmAdapter:
    """Real adapter — wraps :class:`vllm.AsyncLLMEngine`.

    The vLLM package is intentionally imported inside :meth:`load` to
    keep the outer package importable on CPU-only CI. If the import
    fails we raise :class:`ModelLoadFailed` with a clear message.
    """

    artifact: Artifact
    pool: VllmPoolSpec
    local_root: Path
    verifier: SignatureVerifier | None = None
    engine: Any | None = None  # vllm.AsyncLLMEngine
    lora_registry: dict[str, Any] = field(default_factory=dict)
    _loaded_at_monotonic: float | None = None

    @property
    def artifact_id(self) -> str:
        return self.artifact.artifact_id

    async def _verify_or_die(self) -> None:
        """Call the cosign L3 verify on the weights blob, or raise."""
        if self.verifier is None:
            return
        blob = self.local_root / "model.safetensors"
        sig = self.local_root / "model.safetensors.sig"
        if not blob.exists():
            raise ArtifactVerificationFailed(
                f"weights blob missing at {blob!s}",
            )
        if not sig.exists():
            raise ArtifactVerificationFailed(
                f"signature file missing at {sig!s}",
            )
        cert = self.local_root / "model.safetensors.cert"
        result = await self.verifier.verify(VerificationInput(
            artifact_id=self.artifact.artifact_id,
            layer=VerificationLayer.L3_PRE_LOAD,
            blob_path=blob,
            signature_path=sig,
            certificate_path=cert if cert.exists() else None,
        ))
        if not result.ok:
            raise ArtifactVerificationFailed(
                f"L3 pre-load verification failed for "
                f"{self.artifact.artifact_id}: {result.details}",
            )

    async def load(self) -> None:
        # L3 — verify signature BEFORE instantiating the vLLM engine
        # (which would mmap and place weights on GPU).
        await self._verify_or_die()
        try:
            from vllm import AsyncEngineArgs, AsyncLLMEngine
        except ImportError as exc:  # pragma: no cover
            raise ModelLoadFailed(
                "vllm is not installed; cannot use VllmAdapter "
                "(install platform extra 'serving')",
            ) from exc
        try:
            args = AsyncEngineArgs(
                model=str(self.local_root),
                tensor_parallel_size=self.pool.tensor_parallel,
                max_model_len=self.pool.max_model_len,
                dtype=self.pool.dtype,
                gpu_memory_utilization=self.pool.gpu_memory_utilization,
                enable_lora=self.pool.enable_lora,
                max_loras=self.pool.max_loras,
                max_lora_rank=self.pool.max_lora_rank,
            )
            self.engine = AsyncLLMEngine.from_engine_args(args)
        except Exception as exc:  # pragma: no cover
            raise ModelLoadFailed(
                f"vLLM engine initialization failed: {exc}",
            ) from exc
        self._loaded_at_monotonic = time.monotonic()

    async def ready(self) -> bool:
        return self.engine is not None

    async def generate(
        self, req: CompletionRequest,
    ) -> CompletionResponse:
        if self.engine is None:
            raise ModelLoadFailed("engine not loaded")
        try:
            from vllm import SamplingParams
        except ImportError as exc:  # pragma: no cover
            raise ModelLoadFailed("vllm not available at generate time") from exc

        params = SamplingParams(
            max_tokens=req.max_tokens,
            temperature=req.temperature,
            top_p=req.top_p,
            top_k=req.top_k,
            stop=list(req.stop) if req.stop else None,
        )
        lora = None
        if req.lora_id and req.lora_id in self.lora_registry:
            lora = self.lora_registry[req.lora_id]

        start = time.monotonic()
        final_text = ""
        prompt_tokens = 0
        completion_tokens = 0
        finish = "stop"
        # vLLM generate is an async iterator; we collect the final.
        async for out in self.engine.generate(
            prompt=req.prompt,
            sampling_params=params,
            request_id=req.request_id,
            lora_request=lora,
        ):
            if out.outputs:
                final_text = out.outputs[0].text
                completion_tokens = len(out.outputs[0].token_ids)
                finish = out.outputs[0].finish_reason or finish
            if getattr(out, "prompt_token_ids", None):
                prompt_tokens = len(out.prompt_token_ids)
        elapsed_ms = (time.monotonic() - start) * 1000.0
        return CompletionResponse(
            request_id=req.request_id,
            text=final_text,
            prompt_tokens=prompt_tokens,
            completion_tokens=completion_tokens,
            finish_reason=finish,
            artifact_id=self.artifact.artifact_id,
            content_hash_head=self.artifact.content_hash_hex[:16],
            latency_ms=elapsed_ms,
        )

    async def stream(
        self, req: CompletionRequest,
    ) -> AsyncIterator[str]:
        # In streaming mode we yield deltas; simplified here.
        resp = await self.generate(req)
        for tok in resp.text.split():
            yield tok + " "

    async def register_lora(self, lora_id: str, local_path: str) -> None:
        try:
            from vllm.lora.request import LoRARequest
        except ImportError as exc:  # pragma: no cover
            raise ModelLoadFailed("vllm not available") from exc
        self.lora_registry[lora_id] = LoRARequest(
            lora_name=lora_id,
            lora_int_id=(hash(lora_id) & 0xFFFF) or 1,
            lora_path=local_path,
        )

    async def close(self) -> None:
        if self.engine is not None:
            shutdown = getattr(self.engine, "shutdown", None)
            if shutdown is not None:
                await shutdown()
        self.engine = None


__all__ = ["VllmAdapter"]

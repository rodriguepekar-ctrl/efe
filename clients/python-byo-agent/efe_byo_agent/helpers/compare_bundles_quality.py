"""CompareBundlesQualityHelper — direct mode (M20).

NOTE : this helper requires an MCP client (`agent.mcp`) to fetch
classify results from the EFE backend. If only an LLMProvider is
configured (no MCP), this helper raises.
"""
from __future__ import annotations

import json

from ..exceptions import EfeAgentError
from ._base import BaseHelper


_MAX_DIVERGENCES_IN_PROMPT = 50


class CompareBundlesQualityHelper(BaseHelper):
    name = "efe_compare_bundles_quality"
    helper_id = "compare_bundles_quality"
    template_id = "compare_bundles_quality_v1"

    intelligence_priority = 0.9
    speed_priority = 0.0
    cost_priority = 0.1
    max_tokens = 1500
    temperature = 0.0

    async def run(
        self,
        *,
        catalog: str,
        version_a: str,
        version_b: str,
        test_sentences: list[str] | None = None,
        sample_size: int = 100,
    ) -> dict:
        if version_a == version_b:
            raise EfeAgentError(
                f"version_a and version_b must differ, both = {version_a!r}"
            )
        if self._agent.mcp is None:
            raise EfeAgentError(
                "compare_bundles_quality requires an MCP client to fetch "
                "classify results — connect via Agent(mcp_endpoint=...) "
                "or use the mcp-server tool directly via Agent.mcp_tool()."
            )

        # 1. Resolve sentences
        sentences = list(test_sentences) if test_sentences else \
            await self._fetch_gold_sentences(catalog, sample_size)
        if not sentences:
            raise EfeAgentError(
                f"no sentences available for catalog={catalog!r}"
            )

        # 2. Classify on both versions via mcp-server (efe_classify or
        # efe_bulk_classify if available).
        results_a = await self._classify_batch(catalog, version_a, sentences)
        results_b = await self._classify_batch(catalog, version_b, sentences)

        # 3. Compute divergences
        divergences: list[dict] = []
        for s, a, b in zip(sentences, results_a, results_b):
            if a["label"] != b["label"] or abs(
                float(a["confidence"]) - float(b["confidence"])
            ) >= 0.10:
                divergences.append({
                    "sentence": s,
                    "label_a": a["label"],
                    "confidence_a": round(float(a["confidence"]), 3),
                    "label_b": b["label"],
                    "confidence_b": round(float(b["confidence"]), 3),
                })
        divergences.sort(
            key=lambda d: max(d["confidence_a"], d["confidence_b"]),
        )
        prompt_div = divergences[:_MAX_DIVERGENCES_IN_PROMPT]

        # 4. Render + LLM
        sys_p, usr_p = self._render(
            catalog=catalog,
            version_a=version_a,
            version_b=version_b,
            n_total=len(sentences),
            n_divergent=len(divergences),
            divergences_json=json.dumps(prompt_div, ensure_ascii=False),
        )
        resp = await self._call_llm(sys_p, usr_p)
        validated = await self._validate(resp.text, sys_p, usr_p)

        return {
            "n_sentences_compared": len(sentences),
            "n_divergent": len(divergences),
            "accuracy_delta_pp": validated["accuracy_delta_pp"],
            "divergence_examples": validated["divergence_examples"],
            "summary": validated["summary"],
            "recommendation": validated["recommendation"],
            "metadata": {
                "version_a": version_a,
                "version_b": version_b,
                "tokens_in": resp.tokens_in,
                "tokens_out": resp.tokens_out,
                "cost_usd": resp.cost_usd,
                "model_used": resp.model_name,
                "prompt_template": self.template_id,
            },
        }

    async def _fetch_gold_sentences(self, catalog: str, n: int) -> list[str]:
        # Best-effort : delegate to mcp-server admin endpoint if exposed.
        # If unavailable, return empty list.
        try:
            result = await self._agent.mcp.call_tool(
                "efe_diagnose_catalog",
                {"catalog": catalog, "include_sample_sentences": True},
            )
            samples = result.get("sample_sentences") or []
            return list(samples[:n])
        except Exception:  # noqa: BLE001
            return []

    async def _classify_batch(
        self, catalog: str, version: str, sentences: list[str],
    ) -> list[dict]:
        # Try efe_bulk_classify ; fallback to per-call efe_classify.
        try:
            result = await self._agent.mcp.call_tool(
                "efe_bulk_classify",
                {
                    "catalog": catalog,
                    "version": version,
                    "texts": sentences,
                },
            )
            return list(result.get("results") or [])
        except Exception:  # noqa: BLE001
            out: list[dict] = []
            for s in sentences:
                r = await self._agent.mcp.call_tool(
                    "efe_classify",
                    {"catalog": catalog, "version": version, "text": s},
                )
                out.append({
                    "label": r.get("label"),
                    "confidence": r.get("confidence", 0.0),
                })
            return out


__all__ = ["CompareBundlesQualityHelper"]

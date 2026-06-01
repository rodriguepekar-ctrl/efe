"""SummarizeClassificationHelper — direct mode (M20).

Mirrors the mcp-server tool `efe_summarize_classification` exactly :
same prompt template, same validation, same output schema. Use this
when running outside an MCP client (CI batch, scripts).
"""
from __future__ import annotations

import json

from ._base import BaseHelper


_LANGUAGES = {
    "en": "English",
    "fr": "français",
    "de": "Deutsch",
    "es": "español",
    "pt": "português",
    "it": "italiano",
}


class SummarizeClassificationHelper(BaseHelper):
    name = "efe_summarize_classification"
    helper_id = "summarize_classification"
    template_id = "summarize_classification_v1"

    intelligence_priority = 0.7
    speed_priority = 0.3
    cost_priority = 0.0
    max_tokens = 600
    temperature = 0.0

    async def run(
        self,
        *,
        catalog: str,
        sentence: str,
        classification_result: dict,
        version: str | None = None,
        language_hint: str = "en",
        class_definitions: dict | None = None,
    ) -> dict:
        sys_prompt, usr_prompt = self._render(
            catalog=catalog,
            sentence=sentence,
            classification_result_json=json.dumps(
                classification_result, ensure_ascii=False,
            ),
            class_definitions_json=json.dumps(
                class_definitions or {}, ensure_ascii=False,
            ),
            language=_LANGUAGES.get(language_hint, "English"),
        )
        resp = await self._call_llm(sys_prompt, usr_prompt)
        validated = await self._validate(resp.text, sys_prompt, usr_prompt)
        return {
            "explanation": validated["explanation"],
            "key_factors": validated["key_factors"],
            "confidence_assessment": validated["confidence_assessment"],
            "suggested_alternatives": validated["suggested_alternatives"],
            "metadata": {
                "tokens_in": resp.tokens_in,
                "tokens_out": resp.tokens_out,
                "cost_usd": resp.cost_usd,
                "model_used": resp.model_name,
                "provider": resp.provider_name,
                "language": language_hint,
                "prompt_template": self.template_id,
            },
        }


__all__ = ["SummarizeClassificationHelper"]

"""GenerateTestSentencesHelper — direct mode (M20)."""
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


class GenerateTestSentencesHelper(BaseHelper):
    name = "efe_generate_test_sentences"
    helper_id = "generate_test_sentences"
    template_id = "generate_test_sentences_v1"

    intelligence_priority = 0.6
    speed_priority = 0.4
    cost_priority = 0.0
    max_tokens = 2000
    temperature = 0.7

    async def run(
        self,
        *,
        catalog: str,
        target_class: str,
        version: str | None = None,
        count: int = 16,
        difficulty: str = "mixed",
        language: str = "en",
        avoid_redundancy_with_existing: bool = True,
        existing_sentences: list[str] | None = None,
    ) -> dict:
        existing = list(existing_sentences) if existing_sentences else []
        if not existing and avoid_redundancy_with_existing and self._agent.mcp:
            existing = await self._fetch_existing_sentences(
                catalog, version, target_class,
            )
        existing_for_prompt = existing[:30]

        sys_p, usr_p = self._render(
            catalog=catalog,
            target_class=target_class,
            count=count,
            difficulty=difficulty,
            language=_LANGUAGES.get(language, "English"),
            existing_sentences_json=json.dumps(
                existing_for_prompt, ensure_ascii=False,
            ),
        )
        resp = await self._call_llm(sys_p, usr_p)
        validated = await self._validate(
            resp.text, sys_p, usr_p, expected_count=count,
        )
        return {
            "sentences": validated["sentences"],
            "difficulty_self_assessment": validated["difficulty_self_assessment"],
            "rationale": validated["rationale"],
            "metadata": {
                "catalog": catalog,
                "target_class": target_class,
                "count_requested": count,
                "count_returned": len(validated["sentences"]),
                "difficulty": difficulty,
                "language": language,
                "n_existing_referenced": len(existing_for_prompt),
                "tokens_in": resp.tokens_in,
                "tokens_out": resp.tokens_out,
                "cost_usd": resp.cost_usd,
                "model_used": resp.model_name,
                "prompt_template": self.template_id,
            },
        }

    async def _fetch_existing_sentences(
        self,
        catalog: str,
        version: str | None,
        class_name: str,
    ) -> list[str]:
        try:
            args: dict = {
                "catalog": catalog,
                "class_name": class_name,
                "limit": 30,
            }
            if version:
                args["version"] = version
            result = await self._agent.mcp.call_tool(
                "efe_find_similar_examples", args,
            )
            return [
                ex["text"] for ex in (result.get("examples") or [])
                if isinstance(ex, dict) and "text" in ex
            ]
        except Exception:  # noqa: BLE001
            return []


__all__ = ["GenerateTestSentencesHelper"]

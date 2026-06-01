"""AnalyzeConfusionMatrixHelper — direct mode (M20)."""
from __future__ import annotations

import json
from typing import Any

from ._base import BaseHelper


class AnalyzeConfusionMatrixHelper(BaseHelper):
    name = "efe_analyze_confusion_matrix"
    helper_id = "analyze_confusion_matrix"
    template_id = "analyze_confusion_matrix_v1"

    intelligence_priority = 0.9
    speed_priority = 0.1
    cost_priority = 0.0
    max_tokens = 1500
    temperature = 0.0

    async def run(
        self,
        *,
        catalog: str,
        confusion_matrix: dict[str, dict[str, int]],
        version: str | None = None,
        min_confusion_rate: float = 0.05,
        max_pairs: int = 5,
    ) -> dict:
        if not (0.0 <= min_confusion_rate <= 1.0):
            raise ValueError("min_confusion_rate must be in [0, 1]")

        pairs = self._extract_top_pairs(
            confusion_matrix, min_rate=min_confusion_rate, max_pairs=max_pairs,
        )
        if not pairs:
            return {
                "top_confusion_pairs": [],
                "proposed_actions": [],
                "summary": (
                    f"No confusion pairs above min_confusion_rate="
                    f"{min_confusion_rate} in catalog {catalog!r}."
                ),
                "metadata": {
                    "catalog": catalog,
                    "n_classes": len(confusion_matrix),
                    "n_pairs_above_threshold": 0,
                    "prompt_template": self.template_id,
                },
            }

        sys_p, usr_p = self._render(
            catalog=catalog,
            version=version or "(unspecified)",
            confusion_matrix_json=json.dumps(pairs, ensure_ascii=False),
            min_confusion_rate=min_confusion_rate,
            max_pairs=max_pairs,
        )
        resp = await self._call_llm(sys_p, usr_p)
        validated = await self._validate(resp.text, sys_p, usr_p)

        return {
            "top_confusion_pairs": validated["top_confusion_pairs"],
            "proposed_actions": validated["proposed_actions"],
            "summary": validated["summary"],
            "metadata": {
                "catalog": catalog,
                "version": version,
                "n_classes": len(confusion_matrix),
                "n_pairs_above_threshold": len(pairs),
                "min_confusion_rate": min_confusion_rate,
                "max_pairs": max_pairs,
                "tokens_in": resp.tokens_in,
                "tokens_out": resp.tokens_out,
                "cost_usd": resp.cost_usd,
                "model_used": resp.model_name,
                "prompt_template": self.template_id,
            },
        }

    @staticmethod
    def _extract_top_pairs(
        matrix: dict[str, dict[str, int]],
        *,
        min_rate: float,
        max_pairs: int,
    ) -> list[dict[str, Any]]:
        pairs: list[dict[str, Any]] = []
        for true_class, row in matrix.items():
            if not isinstance(row, dict):
                continue
            total = sum(int(v) for v in row.values() if isinstance(v, int))
            if total == 0:
                continue
            for pred, count in row.items():
                if pred == true_class:
                    continue
                rate = float(count) / float(total)
                if rate >= min_rate:
                    pairs.append({
                        "true_class": true_class,
                        "predicted_class": pred,
                        "count": int(count),
                        "row_total": total,
                        "rate": round(rate, 4),
                    })
        pairs.sort(key=lambda p: p["rate"], reverse=True)
        return pairs[:max_pairs]


__all__ = ["AnalyzeConfusionMatrixHelper"]

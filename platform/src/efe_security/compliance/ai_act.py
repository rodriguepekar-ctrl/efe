"""EU AI Act mapping."""

from __future__ import annotations

from dataclasses import dataclass

__all__ = ["AI_ACT_MAPPING", "AiActSystem"]


@dataclass(frozen=True, slots=True)
class AiActSystem:
    number: str
    title: str
    efe_controls: tuple[str, ...]


AI_ACT_MAPPING: tuple[AiActSystem, ...] = (
    AiActSystem("9", "Risk management", ("threat_model", "incident")),
    AiActSystem("12", "Record keeping", ("audit",)),
    AiActSystem("14", "Human oversight", ("authz_obligations",)),
    AiActSystem("15", "Accuracy and robustness", ("drift", "retraining")),
)

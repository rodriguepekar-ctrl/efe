"""Content moderation — pluggable providers.

Production chains :class:`LocalBlocklistModerator` (fast) with
``OpenAiModerationModerator`` (async HTTP). The chain short-circuits
on the first provider reporting a category above threshold.

This module ships the local blocklist + a null provider for tests;
the OpenAI-backed provider is an adapter left to M07.L4 (implemented
as part of integration code in prod).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Protocol

from efe_inference.config import ModerationSettings


@dataclass(frozen=True, slots=True)
class ModerationResult:
    flagged: bool
    categories: dict[str, float] = field(default_factory=dict)
    provider: str = ""

    def flagged_categories(self, thresholds: dict[str, float]) -> list[str]:
        return [
            cat for cat, score in self.categories.items()
            if score >= thresholds.get(cat, 0.5)
        ]


class ContentModerator(Protocol):
    name: str

    async def moderate(
        self, text: str,
    ) -> ModerationResult:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# Null moderator (test helper)
# ---------------------------------------------------------------------------


@dataclass
class NullModerator:
    name: str = "null"

    async def moderate(self, text: str) -> ModerationResult:
        return ModerationResult(flagged=False, provider=self.name)


# ---------------------------------------------------------------------------
# Local blocklist
# ---------------------------------------------------------------------------


_DEFAULT_BLOCKLIST: dict[str, tuple[str, ...]] = {
    "hate": (
        "genocide", "ethnic cleansing",
    ),
    "violence": (
        "how to make a bomb", "build an explosive",
    ),
    "self_harm": (
        "how to end my life",
    ),
    "sexual": (
        # deliberately generic — real deployments tune this per-locale
    ),
    "harassment": (
        "kill yourself",
    ),
}


@dataclass
class LocalBlocklistModerator:
    name: str = "local_blocklist"
    blocklist: dict[str, tuple[str, ...]] = field(
        default_factory=lambda: _DEFAULT_BLOCKLIST,
    )

    async def moderate(self, text: str) -> ModerationResult:
        lower = text.lower()
        scores: dict[str, float] = {}
        for cat, needles in self.blocklist.items():
            for needle in needles:
                if needle and needle in lower:
                    # binary scoring: 1.0 on any hit.
                    scores[cat] = 1.0
                    break
        return ModerationResult(
            flagged=bool(scores),
            categories=scores,
            provider=self.name,
        )


# ---------------------------------------------------------------------------
# Chain (first hit wins)
# ---------------------------------------------------------------------------


@dataclass
class ModerationChain:
    providers: tuple[ContentModerator, ...]
    settings: ModerationSettings

    async def moderate(self, text: str) -> ModerationResult:
        """Run providers in order; return first flagged result.

        Fix H3 — reuse the last non-flagged result from the single
        pass instead of re-calling ``providers[-1]``. Doubled latency
        and doubled API cost on every clean request was the bug.
        """
        last_result: ModerationResult | None = None
        for provider in self.providers:
            result = await provider.moderate(text)
            if result.flagged_categories(self.settings.thresholds):
                return result
            last_result = result
        return last_result or ModerationResult(flagged=False)


def default_chain(settings: ModerationSettings) -> ModerationChain:
    providers: list[ContentModerator] = []
    if "local_blocklist" in settings.providers:
        providers.append(LocalBlocklistModerator())
    # Slot for openai_moderations / azure providers — not included in
    # MVP tests (they require live credentials).
    return ModerationChain(
        providers=tuple(providers), settings=settings,
    )


__all__ = [
    "ContentModerator",
    "LocalBlocklistModerator",
    "ModerationChain",
    "ModerationResult",
    "NullModerator",
    "default_chain",
]

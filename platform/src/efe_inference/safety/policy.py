"""SafetyDecider — orchestrates scrubber / moderator / injection detector.

For each request, a :class:`SafetyContext` is built once per stage
(preflight on the input, postflight on the output). The decider
consults PII flags, moderation categories, and injection score, and
returns a single :class:`SafetyVerdict` with the final decision and
the underlying signals for audit.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Literal


class SafetyStage(str, Enum):
    PREFLIGHT_PII = "preflight_pii"
    PREFLIGHT_MODERATION = "preflight_moderation"
    PREFLIGHT_INJECTION = "preflight_injection"
    POSTFLIGHT_PII = "postflight_pii"
    POSTFLIGHT_MODERATION = "postflight_moderation"
    POSTFLIGHT_POLICY = "postflight_policy"


Decision = Literal["allow", "review", "block"]


@dataclass(frozen=True, slots=True)
class SafetyVerdict:
    decision: Decision
    stage: SafetyStage
    categories: tuple[str, ...] = ()
    reason: str = ""
    scrubbed_text: str | None = None


@dataclass
class SafetyContext:
    """Bundled references to all safety components + per-tenant policy.

    Intentionally tiny façade — all heavy logic lives in the component
    classes.
    """

    scrubber: PiiScrubberProto
    moderator: ModeratorProto
    injector: InjectorProto
    output_filter: OutputFilterProto | None = None
    injection_threshold: int = 80
    moderation_thresholds: dict[str, float] = field(default_factory=dict)


@dataclass
class SafetyDecider:
    context: SafetyContext

    async def preflight(self, text: str) -> SafetyVerdict:
        scrub = self.context.scrubber.scrub(text)
        pii_cats = tuple(f.category for f in scrub.flags)
        # PII -> redact in place; do NOT block unless explicitly
        # configured at scrubber level (out of scope here).
        mod = await self.context.moderator.moderate(scrub.scrubbed_text)
        mod_flagged = mod.flagged_categories(self.context.moderation_thresholds)
        if mod_flagged:
            return SafetyVerdict(
                decision="block",
                stage=SafetyStage.PREFLIGHT_MODERATION,
                categories=tuple(mod_flagged),
                reason="moderation_flagged",
                scrubbed_text=scrub.scrubbed_text,
            )
        inj = self.context.injector.score(scrub.scrubbed_text)
        if inj.score >= self.context.injection_threshold:
            return SafetyVerdict(
                decision="block",
                stage=SafetyStage.PREFLIGHT_INJECTION,
                categories=("injection", *tuple(inj.phrases)),
                reason=f"injection_score={inj.score}",
                scrubbed_text=scrub.scrubbed_text,
            )
        return SafetyVerdict(
            decision="allow",
            stage=SafetyStage.PREFLIGHT_PII,
            categories=pii_cats,
            scrubbed_text=scrub.scrubbed_text,
        )

    async def postflight(self, text: str) -> SafetyVerdict:
        scrub = self.context.scrubber.scrub(text)
        mod = await self.context.moderator.moderate(scrub.scrubbed_text)
        mod_flagged = mod.flagged_categories(self.context.moderation_thresholds)
        if mod_flagged:
            return SafetyVerdict(
                decision="block",
                stage=SafetyStage.POSTFLIGHT_MODERATION,
                categories=tuple(mod_flagged),
                reason="moderation_flagged_output",
                scrubbed_text=scrub.scrubbed_text,
            )
        if self.context.output_filter is not None:
            verdict = self.context.output_filter.evaluate(scrub.scrubbed_text)
            if verdict.decision != "allow":
                return SafetyVerdict(
                    decision=(
                        "block" if verdict.decision == "block" else "allow"
                    ),
                    stage=SafetyStage.POSTFLIGHT_POLICY,
                    categories=("policy",),
                    reason=verdict.reason,
                    scrubbed_text=verdict.redacted_text or scrub.scrubbed_text,
                )
        return SafetyVerdict(
            decision="allow",
            stage=SafetyStage.POSTFLIGHT_PII,
            categories=tuple(f.category for f in scrub.flags),
            scrubbed_text=scrub.scrubbed_text,
        )


# ---------------------------------------------------------------------------
# Structural Protocols (kept local to avoid circular imports)
# ---------------------------------------------------------------------------


from typing import Any, Protocol  # noqa: E402


class PiiScrubberProto(Protocol):
    def scrub(self, text: str) -> Any:  # pragma: no cover
        ...


class ModeratorProto(Protocol):
    async def moderate(self, text: str) -> Any:  # pragma: no cover
        ...


class InjectorProto(Protocol):
    def score(self, text: str) -> Any:  # pragma: no cover
        ...


class OutputFilterProto(Protocol):
    def evaluate(self, text: str) -> Any:  # pragma: no cover
        ...


__all__ = [
    "Decision",
    "SafetyContext",
    "SafetyDecider",
    "SafetyStage",
    "SafetyVerdict",
]

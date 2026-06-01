"""Output policy filter — deny-lists, allow-lists, per-tenant profiles."""

from __future__ import annotations

import re
from collections.abc import Iterable
from dataclasses import dataclass
from typing import Literal

PolicyDecision = Literal["allow", "redact", "block"]


@dataclass(frozen=True, slots=True)
class OutputPolicyVerdict:
    decision: PolicyDecision
    reason: str = ""
    redacted_text: str | None = None


@dataclass
class OutputPolicyFilter:
    """Deny/allow lists and profile-based filtering.

    Profile defaults :

    - ``pg13`` : regex blocks on explicit violence/sexual keywords.
    - ``professional`` : redacts informal markers (excess emojis,
      all-caps words).
    - ``custom`` : tenant-provided regex list.
    """

    deny_regex: tuple[re.Pattern[str], ...] = ()
    allow_regex: tuple[re.Pattern[str], ...] = ()
    redact_regex: tuple[re.Pattern[str], ...] = ()
    replacement: str = "[REDACTED]"

    @classmethod
    def compile(
        cls,
        deny_list: Iterable[str] = (),
        allow_list: Iterable[str] = (),
        redact_list: Iterable[str] = (),
    ) -> OutputPolicyFilter:
        return cls(
            deny_regex=tuple(re.compile(p) for p in deny_list),
            allow_regex=tuple(re.compile(p) for p in allow_list),
            redact_regex=tuple(re.compile(p) for p in redact_list),
        )

    def evaluate(self, text: str) -> OutputPolicyVerdict:
        for pattern in self.deny_regex:
            if pattern.search(text):
                return OutputPolicyVerdict(
                    decision="block",
                    reason=f"deny_regex:{pattern.pattern}",
                )
        if self.allow_regex and not any(
            p.search(text) for p in self.allow_regex
        ):
            return OutputPolicyVerdict(
                decision="block",
                reason="no_allow_regex_matched",
            )
        if self.redact_regex:
            redacted = text
            applied = 0
            for p in self.redact_regex:
                redacted, n = p.subn(self.replacement, redacted)
                applied += n
            if applied:
                return OutputPolicyVerdict(
                    decision="redact",
                    reason=f"redacted_{applied}_matches",
                    redacted_text=redacted,
                )
        return OutputPolicyVerdict(decision="allow")


__all__ = [
    "OutputPolicyFilter",
    "OutputPolicyVerdict",
    "PolicyDecision",
]

"""Slack channel — webhook-based.

Production wires a real ``httpx.AsyncClient`` via the ``http_client``
constructor arg. Tests inject an :class:`InMemoryHttpClient` shim.

Message body follows the Slack Blocks format :

* header       : the alert title with severity emoji
* section      : a mrkdwn summary of tenant / task / feature / score
* context      : links to the dashboard + the acking CLI command
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from typing import Any

from efe_drift.alerting.channels.base import (
    DispatchResult,
    IncidentPayload,
)
from efe_drift.clock import Clock, SystemClock

SEVERITY_EMOJI: dict[str, str] = {
    "warning":  ":warning:",
    "major":    ":rotating_light:",
    "critical": ":fire:",
}


HttpPoster = Callable[[str, dict[str, Any]], Awaitable[tuple[int, str]]]


@dataclass(frozen=True, slots=True)
class SlackChannel:
    """Slack webhook channel — sends a Blocks-formatted message."""

    webhook_url: str
    http_poster: HttpPoster
    mention: str | None = None
    clock: Clock = field(default_factory=SystemClock)
    _name: str = "slack"

    @property
    def name(self) -> str:
        return self._name

    async def dispatch(self, payload: IncidentPayload) -> DispatchResult:
        body = self._render(payload)
        try:
            status, text = await self.http_poster(self.webhook_url, body)
        except Exception as exc:
            return DispatchResult(
                ok=False, channel=self._name,
                error=f"{type(exc).__name__}: {exc}",
            )
        if status != 200:
            return DispatchResult(
                ok=False, channel=self._name,
                error=f"status={status}, body={text[:200]!r}",
            )
        return DispatchResult(
            ok=True, channel=self._name,
            delivered_at=self.clock.now(),
        )

    # ---- internals -----------------------------------------------------

    def _render(self, p: IncidentPayload) -> dict[str, Any]:
        emoji = SEVERITY_EMOJI.get(p.severity, ":bell:")
        mention = f"{self.mention} " if self.mention else ""
        header = f"{emoji} Drift Alert — {p.severity.upper()}"
        score_line = f"*Score:* {p.score:.4f}"
        if p.p_value is not None:
            score_line += f"  (*p-value:* {p.p_value:.2e})"
        mrkdwn = (
            f"{mention}*Tenant:* `{p.tenant_id}`  *Task:* `{p.task_id}`  "
            f"*Feature:* `{p.feature_name}`\n"
            f"*Method:* `{p.method}`  *Window:* `{p.window_kind}`\n"
            f"{score_line}\n"
            f"*Baseline:* `{p.baseline_id[:8]}…`\n"
        )
        return {
            "text": header,
            "blocks": [
                {
                    "type": "header",
                    "text": {"type": "plain_text", "text": header},
                },
                {
                    "type": "section",
                    "text": {"type": "mrkdwn", "text": mrkdwn},
                },
                {
                    "type": "context",
                    "elements": [
                        {
                            "type": "mrkdwn",
                            "text": (
                                f"incident_id=`{p.incident_id}`  "
                                f"fired_at=`{p.fired_at}`"
                            ),
                        },
                    ],
                },
            ],
        }


__all__ = ["SEVERITY_EMOJI", "HttpPoster", "SlackChannel"]

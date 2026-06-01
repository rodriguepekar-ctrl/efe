"""Automatable runbooks for incident categories."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Awaitable, Callable

from efe_security.incident.triage import IncidentCategory, IncidentRecord
from efe_security.errors import PlaybookFailed

__all__ = ["Runbook", "RunbookRegistry", "run_playbook"]


PlaybookFn = Callable[[IncidentRecord], Awaitable[dict]]


@dataclass(frozen=True, slots=True)
class Runbook:
    category: IncidentCategory
    playbook: PlaybookFn
    description: str = ""


@dataclass(slots=True)
class RunbookRegistry:
    _entries: dict[IncidentCategory, Runbook] = field(default_factory=dict)

    def register(self, rb: Runbook) -> None:
        self._entries[rb.category] = rb

    def get(self, category: IncidentCategory) -> Runbook | None:
        return self._entries.get(category)

    def list(self) -> list[Runbook]:
        return list(self._entries.values())


async def run_playbook(
    registry: RunbookRegistry, incident: IncidentRecord
) -> dict:
    rb = registry.get(incident.category)
    if rb is None:
        raise PlaybookFailed(
            f"no runbook for category={incident.category.value}"
        )
    try:
        return await rb.playbook(incident)
    except Exception as exc:  # noqa: BLE001
        raise PlaybookFailed(str(exc)) from exc

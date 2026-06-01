"""Dead-letter management."""

from __future__ import annotations

import logging
from dataclasses import dataclass
from datetime import datetime
from typing import TYPE_CHECKING

from efe_scheduling.types import JobState

if TYPE_CHECKING:  # pragma: no cover
    from efe_scheduling.storage.protocols import DeadLetterStore, JobStore

__all__ = ["DeadLetterManager"]

_LOG = logging.getLogger("efe_scheduling.dead_letter")


@dataclass(slots=True)
class DeadLetterManager:
    """Move permanently-failed jobs to the dead-letter table."""

    job_store: "JobStore"
    dead_letter_store: "DeadLetterStore"

    async def send(
        self,
        *,
        job_id: str,
        last_run_id: str | None,
        reason: str,
        now: datetime,
    ) -> None:
        version = await self.job_store.current_version(job_id)
        current_state = await self.job_store.current_state(job_id)
        if current_state != JobState.failed_permanent:
            # Transition must be failed_permanent first.
            await self.job_store.transition(
                job_id=job_id,
                to_state=JobState.failed_permanent,
                expected_version=version,
                fields={"release_lease": True},
            )
            version += 1
        await self.job_store.transition(
            job_id=job_id,
            to_state=JobState.dead_letter,
            expected_version=version,
            fields={"release_lease": True},
        )
        await self.dead_letter_store.move(
            job_id=job_id, last_run_id=last_run_id, reason=reason, moved_at=now
        )
        _LOG.warning("jobs.dead_letter.moved job_id=%s reason=%s", job_id, reason)

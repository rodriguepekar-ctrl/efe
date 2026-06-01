"""Key rotation scheduler and plan.

Rotation is expressed declaratively via :class:`RotationPlan`. The
:class:`RotationScheduler` compares plan cadence with the registry to
decide when to rotate.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import TYPE_CHECKING

from efe_security.errors import RotationFailed
from efe_security.secrets.key_registry import KeyEntry, KeyPurpose, KeyState

if TYPE_CHECKING:  # pragma: no cover
    from efe_security.secrets.key_registry import KeyRegistry
    from efe_security.secrets.kms_client import KmsClient

_LOG = logging.getLogger("efe_security.secrets.rotation")

__all__ = ["RotationOutcome", "RotationPlan", "RotationScheduler"]


@dataclass(frozen=True, slots=True)
class RotationPlan:
    purpose: KeyPurpose
    max_age_days: int
    grace_days: int = 7

    def __post_init__(self) -> None:
        if self.max_age_days < 1:
            raise ValueError("max_age_days must be >= 1")
        if self.grace_days < 0:
            raise ValueError("grace_days must be >= 0")


@dataclass(frozen=True, slots=True)
class RotationOutcome:
    rotated: list[str] = field(default_factory=list)
    retired: list[str] = field(default_factory=list)
    skipped: list[str] = field(default_factory=list)


@dataclass(slots=True)
class RotationScheduler:
    registry: "KeyRegistry"
    kms: "KmsClient"
    plans: tuple[RotationPlan, ...]

    async def run(self, *, now: datetime) -> RotationOutcome:
        if now.tzinfo is None:
            raise ValueError("now must be tz-aware")
        out = RotationOutcome()
        for plan in self.plans:
            actives = await self.registry.list_active(purpose=plan.purpose)
            for entry in actives:
                age = now - entry.created_at
                if age > timedelta(days=plan.max_age_days):
                    # Rotate : mark current retiring, create successor.
                    try:
                        await self.registry.transition(
                            entry.key_id,
                            KeyState.retiring,
                            expected_version=entry.state_version,
                            now=now,
                        )
                        new_version = await self.kms.rotate(key_id=entry.key_id)
                        out.rotated.append(f"{entry.key_id}@v{new_version}")
                        _LOG.info(
                            "rotation.scheduled key_id=%s purpose=%s age_d=%.1f",
                            entry.key_id, plan.purpose.value, age.days,
                        )
                    except Exception as exc:
                        raise RotationFailed(str(exc)) from exc
                else:
                    out.skipped.append(entry.key_id)
            # Retire old retiring keys past their grace window.
            all_rows = await self.registry.list_all()
            for entry in all_rows:
                if entry.state != KeyState.retiring:
                    continue
                retire_age = now - (entry.retired_at or entry.created_at)
                if retire_age > timedelta(days=plan.grace_days):
                    await self.registry.transition(
                        entry.key_id,
                        KeyState.retired,
                        expected_version=entry.state_version,
                        now=now,
                    )
                    out.retired.append(entry.key_id)
        return out

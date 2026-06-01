"""Composite triggers : AllOf / AnyOf / Not."""

from __future__ import annotations

from typing import Sequence

from efe_scheduling.triggers.base import Decision, Fire, Hold, Reset, Trigger, TriggerContext
from efe_scheduling.types import JobKind

__all__ = ["AllOfTrigger", "AnyOfTrigger", "NotTrigger"]


class AllOfTrigger:
    __slots__ = ("_children", "enabled", "job_kind", "task_id", "tenant_id", "trigger_id")

    def __init__(
        self,
        *,
        trigger_id: str,
        tenant_id: str,
        task_id: str | None,
        job_kind: JobKind,
        children: Sequence[Trigger],
        enabled: bool = True,
    ) -> None:
        if not children:
            raise ValueError("AllOfTrigger needs at least one child")
        self.trigger_id = trigger_id
        self.tenant_id = tenant_id
        self.task_id = task_id
        self.job_kind = job_kind
        self.enabled = enabled
        self._children = tuple(children)

    def should_fire(self, ctx: TriggerContext) -> Decision:
        if not self.enabled:
            return Hold(reason="trigger disabled")
        merged: dict[str, object] = {}
        any_reset = False
        for c in self._children:
            d = c.should_fire(ctx)
            if isinstance(d, Hold):
                return Hold(reason=f"child {c.trigger_id} held: {d.reason}")
            if isinstance(d, Reset):
                any_reset = True
                continue
            assert isinstance(d, Fire)
            merged.update(d.payload)
        if any_reset:
            return Hold(reason="some child reset")
        merged["composite"] = "allof"
        return Fire(payload=dict(merged), reason="all children fired")


class AnyOfTrigger:
    __slots__ = ("_children", "enabled", "job_kind", "task_id", "tenant_id", "trigger_id")

    def __init__(
        self,
        *,
        trigger_id: str,
        tenant_id: str,
        task_id: str | None,
        job_kind: JobKind,
        children: Sequence[Trigger],
        enabled: bool = True,
    ) -> None:
        if not children:
            raise ValueError("AnyOfTrigger needs at least one child")
        self.trigger_id = trigger_id
        self.tenant_id = tenant_id
        self.task_id = task_id
        self.job_kind = job_kind
        self.enabled = enabled
        self._children = tuple(children)

    def should_fire(self, ctx: TriggerContext) -> Decision:
        if not self.enabled:
            return Hold(reason="trigger disabled")
        for c in self._children:
            d = c.should_fire(ctx)
            if isinstance(d, Fire):
                return Fire(payload={**d.payload, "composite": "anyof"}, reason=d.reason)
        return Hold(reason="no child fired")


class NotTrigger:
    __slots__ = ("_child", "enabled", "job_kind", "task_id", "tenant_id", "trigger_id")

    def __init__(
        self,
        *,
        trigger_id: str,
        tenant_id: str,
        task_id: str | None,
        job_kind: JobKind,
        child: Trigger,
        enabled: bool = True,
    ) -> None:
        self.trigger_id = trigger_id
        self.tenant_id = tenant_id
        self.task_id = task_id
        self.job_kind = job_kind
        self.enabled = enabled
        self._child = child

    def should_fire(self, ctx: TriggerContext) -> Decision:
        if not self.enabled:
            return Hold(reason="trigger disabled")
        d = self._child.should_fire(ctx)
        if isinstance(d, Fire):
            return Hold(reason="child fired (NOT inverts)")
        return Fire(payload={"composite": "not"}, reason="child did not fire")

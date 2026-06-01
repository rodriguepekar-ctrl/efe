"""AlertRule + AlertRuleStore."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_drift.alerting import (
    AlertRule,
    ChannelRef,
    InMemoryAlertRuleStore,
)
from efe_drift.clock import FakeClock
from efe_drift.detection import DEFAULTS
from efe_drift.detection.base import DriftResult
from efe_drift.errors import NotFound, ValidationError
from efe_drift.ids import new_uuid


def _result(
    *, tenant: str = "acme", task: str = "summarize",
    feature: str = "f1", method: str = "psi",
    window: str | None = None,
) -> DriftResult:
    return DriftResult(
        evaluation_id=new_uuid(),
        tenant_id=tenant, task_id=task,
        feature_id=feature, feature_name=feature,
        baseline_id="b1",
        window_start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 10, 11, 0, tzinfo=timezone.utc),
        method=method,  # type: ignore[arg-type]
        score=0.3, severity="major",
        computed_at=datetime(2026, 4, 10, 11, 0, tzinfo=timezone.utc),
        multi_scale_window=window,  # type: ignore[arg-type]
    )


def test_rule_rejects_missing_channels():
    with pytest.raises(ValidationError):
        AlertRule(
            rule_id=new_uuid(),
            tenant_id="acme",
            method="psi",
            thresholds=DEFAULTS["psi"],
            channels=(),
        )


def test_rule_matches_wildcard():
    rule = AlertRule(
        rule_id=new_uuid(),
        tenant_id="acme",
        method="psi",
        thresholds=DEFAULTS["psi"],
        channels=(ChannelRef(channel="slack"),),
    )
    assert rule.matches(_result(tenant="acme"))
    assert not rule.matches(_result(tenant="other"))


def test_rule_matches_specific_feature_and_window():
    rule = AlertRule(
        rule_id=new_uuid(),
        tenant_id="acme",
        method="psi",
        feature_id="f1",
        window_kind="1h",
        thresholds=DEFAULTS["psi"],
        channels=(ChannelRef(channel="slack"),),
    )
    assert rule.matches(_result(feature="f1", window="1h"))
    assert not rule.matches(_result(feature="f2", window="1h"))
    assert not rule.matches(_result(feature="f1", window="1d"))


def test_rule_specificity_order():
    wildcard = AlertRule(
        rule_id="w", tenant_id="acme", method="psi",
        thresholds=DEFAULTS["psi"],
        channels=(ChannelRef(channel="slack"),),
    )
    specific = AlertRule(
        rule_id="s", tenant_id="acme", method="psi",
        task_id="summarize", feature_id="f1", window_kind="1h",
        thresholds=DEFAULTS["psi"],
        channels=(ChannelRef(channel="slack"),),
    )
    assert specific.specificity > wildcard.specificity


@pytest.mark.asyncio
async def test_in_memory_store_create_and_list():
    clock = FakeClock()
    store = InMemoryAlertRuleStore(clock=clock)
    rule = await store.create(
        tenant_id="acme",
        method="psi",
        thresholds=DEFAULTS["psi"],
        channels=(ChannelRef(channel="slack"),),
    )
    assert rule.created_at is not None
    again = await store.get(rule.rule_id)
    assert again.rule_id == rule.rule_id
    assert len(await store.list(tenant_id="acme")) == 1
    assert len(await store.list(tenant_id="other")) == 0


@pytest.mark.asyncio
async def test_in_memory_store_matching_sorted_by_specificity():
    store = InMemoryAlertRuleStore()
    wild = await store.create(
        tenant_id="acme", method="psi",
        thresholds=DEFAULTS["psi"],
        channels=(ChannelRef(channel="slack"),),
    )
    specific = await store.create(
        tenant_id="acme", method="psi",
        feature_id="f1",
        thresholds=DEFAULTS["psi"],
        channels=(ChannelRef(channel="pagerduty"),),
    )
    rules = await store.matching_rules(_result(feature="f1"))
    assert len(rules) == 2
    # Most specific first.
    assert rules[0].rule_id == specific.rule_id
    assert rules[1].rule_id == wild.rule_id


@pytest.mark.asyncio
async def test_in_memory_store_delete():
    store = InMemoryAlertRuleStore()
    rule = await store.create(
        tenant_id="acme", method="psi",
        thresholds=DEFAULTS["psi"],
        channels=(ChannelRef(channel="slack"),),
    )
    await store.delete(rule.rule_id)
    with pytest.raises(NotFound):
        await store.get(rule.rule_id)


def test_channel_ref_validates_severity():
    with pytest.raises(ValidationError):
        ChannelRef(channel="slack", min_severity="none")

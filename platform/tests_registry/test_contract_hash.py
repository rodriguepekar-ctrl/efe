"""Canonicalisation + contract_hash stability."""

from __future__ import annotations


import pytest

from efe_registry.contracts.canonicalizer import canonicalize_schema
from efe_registry.contracts.hasher import contract_hash_from_schemas
from efe_registry.contracts.registry import ContractRegistry
from efe_registry.contracts.sunset import (
    DeprecationSweeper,
    InMemoryAlertSink,
)
from efe_registry.errors import ContractSchemaInvalid


def test_canonicalize_sorts_keys():
    a = {"b": 1, "a": 2, "c": 3}
    b = {"c": 3, "a": 2, "b": 1}
    assert canonicalize_schema(a) == canonicalize_schema(b)


def test_canonicalize_strips_comments():
    src = {"$comment": "ignore", "type": "object"}
    canon = canonicalize_schema(src)
    assert "$comment" not in canon
    assert canon == {"type": "object"}


def test_canonicalize_rejects_nan():
    with pytest.raises(ContractSchemaInvalid):
        canonicalize_schema({"bad": float("nan")})


def test_canonicalize_rejects_inf():
    with pytest.raises(ContractSchemaInvalid):
        canonicalize_schema({"bad": float("inf")})


def test_canonicalize_rejects_null_in_array():
    with pytest.raises(ContractSchemaInvalid):
        canonicalize_schema({"items": [1, None, 3]})


def test_contract_hash_stable_under_reorder():
    a = {"type": "object", "properties": {"x": {"type": "integer"}}}
    b = {"properties": {"x": {"type": "integer"}}, "type": "object"}
    output = {"type": "string"}
    assert (
        contract_hash_from_schemas(a, output)
        == contract_hash_from_schemas(b, output)
    )


def test_contract_hash_changes_on_value_change():
    a = {"type": "object", "properties": {"x": {"type": "integer"}}}
    b = {"type": "object", "properties": {"x": {"type": "number"}}}
    output = {"type": "string"}
    assert (
        contract_hash_from_schemas(a, output)
        != contract_hash_from_schemas(b, output)
    )


@pytest.mark.asyncio
async def test_contract_registry_register_and_resolve(
    session_factory, settings, fake_clock,
):
    reg = ContractRegistry(
        session_factory=session_factory,
        settings=settings.contracts,
        clock=fake_clock,
    )
    h = await reg.register(
        input_schema={"type": "object"},
        output_schema={"type": "string"},
        description="test",
    )
    # Idempotent second call returns same hash.
    h2 = await reg.register(
        input_schema={"type": "object"},
        output_schema={"type": "string"},
    )
    assert h == h2
    rec = await reg.resolve(h)
    assert rec.contract_hash == h
    assert rec.deprecated_at is None


@pytest.mark.asyncio
async def test_deprecation_sweeper_alerts_at_thresholds(
    session_factory, settings, fake_clock,
):
    reg = ContractRegistry(
        session_factory=session_factory,
        settings=settings.contracts,
        clock=fake_clock,
    )
    h = await reg.register(
        input_schema={"type": "object"},
        output_schema={"type": "string"},
    )
    rec = await reg.deprecate(h)
    assert rec.sunset_at is not None

    sink = InMemoryAlertSink()
    sweeper = DeprecationSweeper(
        session_factory=session_factory,
        settings=settings.contracts,
        sink=sink,
        clock=fake_clock,
    )
    # Advance to 30 days before sunset.
    fake_clock.advance(
        ((settings.contracts.deprecation_grace_days - 30) * 86_400),
    )
    alerts = await sweeper.tick()
    assert len(alerts) == 1
    assert alerts[0].days_remaining in settings.contracts.alert_days_before_sunset

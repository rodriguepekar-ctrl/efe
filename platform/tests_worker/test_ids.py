"""Tests for :mod:`efe_worker.ids`."""

from __future__ import annotations

from efe_worker.ids import (
    SequentialIdGen,
    SystemIdGen,
    new_uuid,
    reset_id_gen,
    set_id_gen,
)


def test_system_id_gen_is_uuid_shaped() -> None:
    reset_id_gen()
    u = new_uuid()
    assert len(u) == 36
    assert u.count("-") == 4


def test_system_id_gen_direct() -> None:
    g = SystemIdGen()
    a, b = g.uuid4(), g.uuid4()
    assert a != b


def test_sequential_id_gen() -> None:
    set_id_gen(SequentialIdGen(start=0))
    try:
        assert new_uuid() == "00000000-0000-0000-0000-000000000001"
        assert new_uuid() == "00000000-0000-0000-0000-000000000002"
    finally:
        reset_id_gen()


def test_reset() -> None:
    set_id_gen(SequentialIdGen())
    reset_id_gen()
    u = new_uuid()
    # System UUIDs have non-zero high bits.
    assert u != "00000000-0000-0000-0000-000000000000"

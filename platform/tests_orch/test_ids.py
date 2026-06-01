"""Tests for the ID generator abstraction."""

from __future__ import annotations

from efe_orch.ids import (
    SequentialIdGen,
    new_hb_id,
    new_run_id,
    new_worker_id,
    reset_id_gen,
    set_id_gen,
)


def test_system_ids_are_uuid_shaped() -> None:
    reset_id_gen()
    rid = new_run_id()
    assert len(rid) == 36
    assert rid.count("-") == 4


def test_sequential_id_gen_deterministic() -> None:
    gen = SequentialIdGen()
    a = gen.uuid4()
    b = gen.uuid4()
    assert a != b
    assert a == "00000000-0000-0000-0000-000000000001"
    assert b == "00000000-0000-0000-0000-000000000002"


def test_module_helpers_use_registered_gen() -> None:
    set_id_gen(SequentialIdGen(start=100))
    try:
        assert new_run_id() == "00000000-0000-0000-0000-000000000101"
        assert new_worker_id() == "00000000-0000-0000-0000-000000000102"
        assert new_hb_id() == "00000000-0000-0000-0000-000000000103"
    finally:
        reset_id_gen()

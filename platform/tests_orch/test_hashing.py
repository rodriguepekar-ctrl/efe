"""Tests for :mod:`efe_orch.hashing`."""

from __future__ import annotations

from efe_orch.hashing import content_hash, sha256_bytes, sha256_hex


def test_sha256_bytes_length() -> None:
    assert len(sha256_bytes(b"hello")) == 32


def test_sha256_hex_accepts_str_or_bytes() -> None:
    assert sha256_hex("x") == sha256_hex(b"x")
    assert len(sha256_hex("x")) == 64


def test_content_hash_key_order_invariant() -> None:
    a = content_hash({"a": 1, "b": 2})
    b = content_hash({"b": 2, "a": 1})
    assert a == b


def test_content_hash_handles_non_json_default() -> None:
    # bytes → hex via _default_json
    h = content_hash({"k": b"\xff\x00"})
    assert isinstance(h, bytes)
    assert len(h) == 32

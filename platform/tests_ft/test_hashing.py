"""Tests for the hashing helpers."""

from __future__ import annotations

from efe_ft.hashing import (
    content_hash,
    content_hash_iter,
    normalise_text,
    sha256_text,
)


def test_normalise_nfkc() -> None:
    assert normalise_text("café") == normalise_text("cafe\u0301")


def test_sha256_text_stable_across_equivalent_inputs() -> None:
    a = sha256_text("Hello  world\r\n")
    b = sha256_text("Hello  world\n")
    assert a == b


def test_content_hash_key_order_insensitive() -> None:
    a = content_hash({"x": 1, "y": 2})
    b = content_hash({"y": 2, "x": 1})
    assert a == b


def test_content_hash_iter_order_insensitive() -> None:
    a = content_hash_iter(["b", "c", "a"])
    b = content_hash_iter(["a", "b", "c"])
    assert a == b


def test_content_hash_differs_on_change() -> None:
    a = content_hash({"x": 1})
    b = content_hash({"x": 2})
    assert a != b

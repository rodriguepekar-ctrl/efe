"""Tests for :mod:`efe_worker.hashing`."""

from __future__ import annotations

from pathlib import Path

import pytest

from efe_worker.hashing import (
    StreamHasher,
    coerce_hex,
    sha256_bytes,
    sha256_file,
    sha256_hex,
    verify_hex,
)


def test_sha256_bytes_length() -> None:
    assert len(sha256_bytes(b"hello")) == 32


def test_sha256_hex_accepts_both() -> None:
    a = sha256_hex("hello")
    b = sha256_hex(b"hello")
    assert a == b
    assert len(a) == 64


def test_sha256_file(tmp_path: Path) -> None:
    p = tmp_path / "a.bin"
    p.write_bytes(b"abc" * 100)
    digest = sha256_file(str(p))
    assert len(digest) == 64


def test_stream_hasher() -> None:
    h = StreamHasher()
    h.update(b"hel")
    h.update(b"lo")
    assert h.hexdigest() == sha256_hex(b"hello")
    assert h.bytes_seen == 5
    assert len(h.digest()) == 32


def test_verify_hex() -> None:
    a = sha256_hex("x")
    assert verify_hex(a, a) is True
    assert verify_hex(a, a.upper()) is True
    assert verify_hex(a, a[:-1] + "0") is False
    assert verify_hex(a, a + "00") is False


def test_coerce_hex_str() -> None:
    assert coerce_hex("A" * 64) == "a" * 64


def test_coerce_hex_bytes() -> None:
    raw = b"\x00" * 32
    assert coerce_hex(raw) == "00" * 32


def test_coerce_hex_rejects_wrong_length() -> None:
    with pytest.raises(ValueError):
        coerce_hex("too_short")
    with pytest.raises(ValueError):
        coerce_hex(b"too_short")


def test_coerce_hex_rejects_wrong_type() -> None:
    with pytest.raises(ValueError):
        coerce_hex(123)  # type: ignore[arg-type]

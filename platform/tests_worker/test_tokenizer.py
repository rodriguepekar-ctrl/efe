"""Tests for :mod:`efe_worker.data.tokenizer`."""

from __future__ import annotations

import pytest

from efe_worker.data.tokenizer import FakeTokenizer, TokenizedBatch
from efe_worker.errors import TokenizationError


def test_fake_tokenizer_encode_single() -> None:
    tk = FakeTokenizer()
    ids = tk.encode("hello")
    assert ids[0] == tk.BOS_ID
    assert ids[-1] == tk.EOS_ID
    assert len(ids) <= tk.max_seq_len


def test_fake_tokenizer_batch_padded() -> None:
    tk = FakeTokenizer(max_seq_len=12)
    batch = tk.encode_batch(["a", "hello world"])
    assert isinstance(batch, TokenizedBatch)
    assert batch.batch_size == 2
    assert len(batch.input_ids[0]) == len(batch.input_ids[1])
    assert sum(batch.attention_mask[0]) < sum(batch.attention_mask[1])


def test_fake_tokenizer_rejects_empty_batch() -> None:
    tk = FakeTokenizer()
    with pytest.raises(TokenizationError):
        tk.encode_batch([])


def test_fake_tokenizer_min_vocab() -> None:
    with pytest.raises(TokenizationError):
        FakeTokenizer(vocab_size=4)


def test_fake_tokenizer_decode_roundtrip() -> None:
    tk = FakeTokenizer(vocab_size=512, max_seq_len=32)
    ids = tk.encode("abc")
    out = tk.decode(ids)
    # Lossy but should include a/b/c or their mapped offset chars.
    assert len(out) >= 1


def test_fake_tokenizer_truncation() -> None:
    tk = FakeTokenizer(max_seq_len=8)
    ids = tk.encode("x" * 100)
    assert len(ids) <= 8

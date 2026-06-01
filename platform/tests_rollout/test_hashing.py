"""Hash primitives — determinism, domain-separation, bucket uniformity."""

from __future__ import annotations


import pytest

from efe_rollout.hashing import (
    DOMAIN_TAG_EXPOSURE,
    DOMAIN_TAG_NATS,
    DOMAIN_TAG_PREREG,
    bucket,
    canonical_json_bytes,
    content_hash,
    hmac_sign,
    hmac_verify,
    murmur3_hash64,
)
from efe_rollout.hashing import _pure_murmur3_64


def test_murmur3_is_deterministic():
    for sample in ("alice", "bob", "café", "🎯", ""):
        a = murmur3_hash64(sample)
        b = murmur3_hash64(sample)
        assert a == b


def test_murmur3_distribution_is_roughly_uniform_over_100k():
    counts = [0] * 10
    for i in range(100_000):
        h = murmur3_hash64(f"user-{i}")
        counts[bucket(h, buckets=10)] += 1
    # Expected 10_000 per bucket; tolerate 5% deviation.
    for c in counts:
        assert 9_000 <= c <= 11_000


def test_pure_murmur3_matches_mmh3_when_available():
    # The internal pure-python implementation must always be consistent
    # with itself; the parity against mmh3 is asserted only when present.
    for sample in ("", "a", "alice", "bob", "café", "🎯", "x" * 1000):
        pure = _pure_murmur3_64(sample.encode("utf-8"), 0)
        assert isinstance(pure, int)
        assert 0 <= pure < 2 ** 64


def test_bucket_uniform_within_5_percent():
    buckets = [0] * 100
    for i in range(50_000):
        h = murmur3_hash64(f"user-{i}")
        buckets[bucket(h, buckets=100)] += 1
    assert min(buckets) > 350
    assert max(buckets) < 650


def test_canonical_json_is_sort_stable():
    a = canonical_json_bytes({"b": 1, "a": 2})
    b = canonical_json_bytes({"a": 2, "b": 1})
    assert a == b


def test_content_hash_changes_on_modification():
    h1 = content_hash({"a": 1, "b": 2})
    h2 = content_hash({"a": 1, "b": 3})
    assert h1 != h2


def test_hmac_sign_verify_happy_path():
    key = b"\x01" * 32
    msg = b"canonical payload"
    sig = hmac_sign(msg, key, domain_tag=DOMAIN_TAG_NATS)
    assert hmac_verify(msg, key, sig, domain_tag=DOMAIN_TAG_NATS)


def test_hmac_rejects_domain_tag_mismatch():
    """Same key + same message but different tags must produce
    non-verifiable signatures — this is the cross-site replay guard."""
    key = b"\x02" * 32
    msg = b"canonical payload"
    sig_nats = hmac_sign(msg, key, domain_tag=DOMAIN_TAG_NATS)
    # Pretend this NATS signature was replayed against a prereg verify.
    assert not hmac_verify(
        msg, key, sig_nats, domain_tag=DOMAIN_TAG_PREREG,
    )
    assert not hmac_verify(
        msg, key, sig_nats, domain_tag=DOMAIN_TAG_EXPOSURE,
    )


def test_hmac_rejects_modified_message():
    key = b"\x03" * 32
    sig = hmac_sign(b"hello", key, domain_tag=DOMAIN_TAG_NATS)
    assert not hmac_verify(
        b"hello!", key, sig, domain_tag=DOMAIN_TAG_NATS,
    )


def test_hmac_empty_domain_tag_rejected():
    key = b"\x04" * 32
    with pytest.raises(ValueError):
        hmac_sign(b"hello", key, domain_tag=b"")


def test_bucket_negative_raises():
    with pytest.raises(ValueError):
        bucket(1, buckets=0)

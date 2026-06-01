"""Hashing primitives — determinism, domain-separation, tamper guards."""

from __future__ import annotations

import pytest

from efe_drift.hashing import (
    DOMAIN_TAG_BASELINE,
    DOMAIN_TAG_NATS,
    DOMAIN_TAG_USER_ID,
    canonical_json_bytes,
    content_hash,
    hash_user_id,
    hmac_sign,
    hmac_verify,
)


def test_canonical_json_sorts_keys():
    a = canonical_json_bytes({"b": 1, "a": 2})
    b = canonical_json_bytes({"a": 2, "b": 1})
    assert a == b


def test_content_hash_changes_when_value_changes():
    h1 = content_hash({"a": 1, "b": 2})
    h2 = content_hash({"a": 1, "b": 3})
    assert h1 != h2


def test_hmac_sign_verify_happy_path():
    key = b"k" * 32
    msg = b"hello world"
    sig = hmac_sign(msg, key, domain_tag=DOMAIN_TAG_BASELINE)
    assert hmac_verify(msg, key, sig, domain_tag=DOMAIN_TAG_BASELINE)


def test_hmac_domain_tag_replay_rejected():
    """Same key + message under NATS tag must NOT verify under the
    baseline tag."""
    key = b"k" * 32
    msg = b"hello"
    sig_nats = hmac_sign(msg, key, domain_tag=DOMAIN_TAG_NATS)
    assert not hmac_verify(
        msg, key, sig_nats, domain_tag=DOMAIN_TAG_BASELINE,
    )
    assert not hmac_verify(
        msg, key, sig_nats, domain_tag=DOMAIN_TAG_USER_ID,
    )


def test_hmac_empty_domain_tag_rejected():
    with pytest.raises(ValueError):
        hmac_sign(b"x", b"k" * 32, domain_tag=b"")


def test_hash_user_id_deterministic():
    salt = b"salt" * 8
    assert hash_user_id("alice", salt=salt) == hash_user_id("alice", salt=salt)


def test_hash_user_id_different_salts_diverge():
    h1 = hash_user_id("alice", salt=b"salt-one" * 4)
    h2 = hash_user_id("alice", salt=b"salt-two" * 4)
    assert h1 != h2


def test_hash_user_id_empty_salt_rejected():
    with pytest.raises(ValueError):
        hash_user_id("alice", salt=b"")

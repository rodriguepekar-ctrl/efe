"""Tests for the error classifier + retry policy."""

from __future__ import annotations

import random

from efe_orch.config import RetrySettings
from efe_orch.retry import (
    BackoffPolicy,
    ErrorClass,
    RetryPolicy,
    classify_error,
)
from efe_orch.retry.policy import NON_RETRYABLE


def test_classify_known_codes() -> None:
    assert classify_error("cuda_oom", "") is ErrorClass.OOM
    assert classify_error("pod_evicted", "") is ErrorClass.TRANSIENT
    assert classify_error("snapshot_missing", "") is ErrorClass.DATA


def test_classify_keyword_fallback() -> None:
    assert classify_error(None, "process killed: OOMKilled") is ErrorClass.OOM
    assert classify_error("unknown_code", "deadline exceeded") is ErrorClass.TIMEOUT
    assert classify_error(None, "PVC not bound to volume") is ErrorClass.CAPACITY


def test_classify_fallback_fatal() -> None:
    assert classify_error(None, None) is ErrorClass.FATAL
    assert classify_error("zzz", "???") is ErrorClass.FATAL


def test_classify_unknown_sentinel() -> None:
    assert classify_error("unknown", "") is ErrorClass.FATAL


def test_backoff_monotonic() -> None:
    pol = BackoffPolicy(RetrySettings(base_backoff_s=10, max_backoff_s=1000,
                                     jitter_ratio=0))
    d1 = pol.delay(attempt=1)
    d2 = pol.delay(attempt=2)
    d3 = pol.delay(attempt=3)
    assert d1 < d2 < d3
    # Exponential (no jitter): exactly 2^(n-1) * base
    assert d1 == 10
    assert d2 == 20
    assert d3 == 40


def test_backoff_bounded_by_max() -> None:
    pol = BackoffPolicy(RetrySettings(base_backoff_s=10, max_backoff_s=25,
                                     jitter_ratio=0))
    assert pol.delay(attempt=8) == 25  # would be 1280 without cap


def test_backoff_deterministic_with_seed() -> None:
    pol = BackoffPolicy(RetrySettings(base_backoff_s=10, jitter_ratio=0.5))
    a = pol.delay(attempt=3, rng=random.Random(42))
    b = pol.delay(attempt=3, rng=random.Random(42))
    assert a == b


def test_retry_policy_non_retryable_classes_never_retry() -> None:
    pol = RetryPolicy(RetrySettings(max_attempts=10))
    for err in NON_RETRYABLE:
        dec = pol.should_retry(
            error_class=err, attempt=1, current_pool="cpu-small",
        )
        assert dec.should_retry is False


def test_retry_policy_budget() -> None:
    pol = RetryPolicy(RetrySettings(max_attempts=3, base_backoff_s=1,
                                    max_backoff_s=1, jitter_ratio=0))
    d1 = pol.should_retry(
        error_class=ErrorClass.TRANSIENT, attempt=1, current_pool="cpu-small",
    )
    d2 = pol.should_retry(
        error_class=ErrorClass.TRANSIENT, attempt=3, current_pool="cpu-small",
    )
    assert d1.should_retry is True
    assert d1.attempt_next == 2
    assert d2.should_retry is False  # budget exhausted


def test_retry_policy_oom_upgrade() -> None:
    pol = RetryPolicy(RetrySettings(max_attempts=4))
    dec = pol.should_retry(
        error_class=ErrorClass.OOM, attempt=1, current_pool="cpu-small",
    )
    assert dec.should_retry
    assert dec.next_pool_hint == "cpu-large"


def test_retry_policy_oom_no_upgrade_past_top() -> None:
    pol = RetryPolicy(RetrySettings(max_attempts=4))
    dec = pol.should_retry(
        error_class=ErrorClass.OOM, attempt=1, current_pool="gpu-a100",
    )
    # Top of ladder → no hint.
    assert dec.next_pool_hint is None


def test_retry_policy_oom_unknown_pool() -> None:
    pol = RetryPolicy(RetrySettings(max_attempts=4))
    dec = pol.should_retry(
        error_class=ErrorClass.OOM, attempt=1, current_pool="mystery",
    )
    assert dec.next_pool_hint is None


def test_retry_policy_oom_none_pool_starts_at_cpu_large() -> None:
    pol = RetryPolicy(RetrySettings(max_attempts=4))
    dec = pol.should_retry(
        error_class=ErrorClass.OOM, attempt=1, current_pool=None,
    )
    assert dec.next_pool_hint == "cpu-large"

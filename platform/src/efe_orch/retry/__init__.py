"""Retry & recovery — error classifier + backoff policy.

Public surface:

    ErrorClass         — closed enum of error categories
    classify_error     — map a raw (code, message) pair to an ErrorClass
    RetryDecision      — outcome of ``should_retry``
    BackoffPolicy      — compute next retry delay (exp + jitter)
    RetryPolicy        — decide whether (and when) to retry
"""

from __future__ import annotations

from efe_orch.retry.classifier import (
    ERROR_CLASS_BY_CODE,
    ErrorClass,
    classify_error,
)
from efe_orch.retry.policy import (
    BackoffPolicy,
    RetryDecision,
    RetryPolicy,
)

__all__ = [
    "ERROR_CLASS_BY_CODE",
    "BackoffPolicy",
    "ErrorClass",
    "RetryDecision",
    "RetryPolicy",
    "classify_error",
]

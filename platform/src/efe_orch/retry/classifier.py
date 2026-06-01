"""Error classifier — mapping ``(code, message)`` to an :class:`ErrorClass`.

We maintain a *closed* enum of classes because the rest of the
orchestrator (backoff policy, dashboards, alerts) branches on them.
Adding a new class is an intentional act that must update every
consumer.

Classification rule: exact-match on ``code`` first (fast + stable);
fall back to keyword heuristics on ``message`` when the code is
unknown (e.g. early prototypes, third-party errors). A lowercased
``keyword in message`` check is deliberate — robust against case and
string interpolation.
"""

from __future__ import annotations

from enum import Enum


class ErrorClass(str, Enum):
    """Coarse-grained error taxonomy used by the retry policy."""

    TRANSIENT = "transient"
    """Infra-level glitch (network, DB deadlock, pod eviction).

    Retryable immediately with exponential backoff.
    """

    OOM = "oom"
    """Out-of-memory on the worker or Python side.

    Retryable *with pool upgrade* — we bump to a bigger pool rather
    than retrying at the same capacity.
    """

    TIMEOUT = "timeout"
    """Soft timeout (``ActiveDeadlineSeconds`` or user-set deadline).

    Retryable a few times but count against the max-attempts budget.
    """

    DATA = "data"
    """Dataset / snapshot is malformed or missing labels.

    Not retryable — human intervention (M02) required.
    """

    CONFIG = "config"
    """Recipe / hyperparameter config invalid (e.g. batch size 0).

    Not retryable; submitter must fix the request.
    """

    CAPACITY = "capacity"
    """Cluster cannot place the Job (e.g. PVC unbound, no GPU free).

    Retryable with backoff; likely to resolve as the cluster drains.
    """

    FATAL = "fatal"
    """Catch-all for everything we decided we don't know how to recover.

    Not retryable — the run is marked FAILED.
    """


ERROR_CLASS_BY_CODE: dict[str, ErrorClass] = {
    # Transient
    "pod_evicted": ErrorClass.TRANSIENT,
    "network_partition": ErrorClass.TRANSIENT,
    "db_serialization_failure": ErrorClass.TRANSIENT,
    "worker_disconnect": ErrorClass.TRANSIENT,
    "heartbeat_timeout": ErrorClass.TRANSIENT,

    # OOM family
    "cuda_oom": ErrorClass.OOM,
    "worker_oom": ErrorClass.OOM,
    "memory_limit_exceeded": ErrorClass.OOM,

    # Timeout
    "active_deadline": ErrorClass.TIMEOUT,
    "user_deadline": ErrorClass.TIMEOUT,
    "eval_timeout": ErrorClass.TIMEOUT,

    # Data
    "snapshot_missing": ErrorClass.DATA,
    "label_imbalance": ErrorClass.DATA,
    "corrupt_dataset": ErrorClass.DATA,
    "empty_snapshot": ErrorClass.DATA,

    # Config
    "invalid_hyperparameters": ErrorClass.CONFIG,
    "unknown_recipe": ErrorClass.CONFIG,
    "invalid_base_model": ErrorClass.CONFIG,

    # Capacity
    "pool_saturated": ErrorClass.CAPACITY,
    "gpu_shortage": ErrorClass.CAPACITY,
    "pvc_unbound": ErrorClass.CAPACITY,

    # Fatal
    "training_diverged": ErrorClass.FATAL,
    "result_upload_failed": ErrorClass.FATAL,
    "unknown": ErrorClass.FATAL,
}


_KEYWORDS_TO_CLASS: tuple[tuple[str, ErrorClass], ...] = (
    ("oom", ErrorClass.OOM),
    ("out of memory", ErrorClass.OOM),
    ("cuda", ErrorClass.OOM),
    ("timeout", ErrorClass.TIMEOUT),
    ("deadline", ErrorClass.TIMEOUT),
    ("snapshot", ErrorClass.DATA),
    ("label", ErrorClass.DATA),
    ("hyperparameter", ErrorClass.CONFIG),
    ("recipe", ErrorClass.CONFIG),
    ("evict", ErrorClass.TRANSIENT),
    ("network", ErrorClass.TRANSIENT),
    ("deadlock", ErrorClass.TRANSIENT),
    ("saturated", ErrorClass.CAPACITY),
    ("gpu shortage", ErrorClass.CAPACITY),
    ("pvc", ErrorClass.CAPACITY),
)


def classify_error(code: str | None, message: str | None) -> ErrorClass:
    """Return an :class:`ErrorClass` for the given ``(code, message)``.

    Lookup precedence:
        1. exact match on ``code`` in :data:`ERROR_CLASS_BY_CODE`,
        2. keyword heuristic on lowercased ``message``,
        3. fallback: :attr:`ErrorClass.FATAL`.
    """
    if code:
        cls = ERROR_CLASS_BY_CODE.get(code)
        if cls is not None:
            return cls
    if message:
        low = message.lower()
        for keyword, cls in _KEYWORDS_TO_CLASS:
            if keyword in low:
                return cls
    return ErrorClass.FATAL


__all__ = ["ERROR_CLASS_BY_CODE", "ErrorClass", "classify_error"]

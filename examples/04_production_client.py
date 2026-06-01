"""Sprint D10 — example 4: production-grade client.

Goal: production wiring template — auth, retries, idempotency,
structured JSON logging, OpenTelemetry trace propagation, graceful
shutdown. The "what you would deploy in prod" canonical client.

Demonstrates:

    * JWT token from environment variable (never inlined)
    * Exponential backoff with full jitter on retryable errors
    * Per-request idempotency key (UUIDv4)
    * OpenTelemetry trace context propagation (optional dep)
    * Structured logging (JSON one-line)
    * Graceful shutdown on SIGTERM/SIGINT
    * Typed error handling (EfeAuthError vs EfeRateLimitError vs ...)

Pre-requisites:
    * efe-server reachable on EFE_HOST (default 127.0.0.1:50051)
      with auth ENABLED (otherwise EFE_AUTH_ENABLED=false override
      bypasses the JWT check)
    * EFE_TOKEN env var set
    * pip install 'efe-tool[otel]==0.1.10' (otel optional)

Run:
    EFE_TOKEN=$(cat /tmp/jwt) python examples/04_production_client.py

Exit codes:
    0 — clean shutdown after processing the input batch
    2 — operational error (missing token, etc.)
    3 — auth failure (token expired/invalid)
"""

from __future__ import annotations

import json
import logging
import os
import random
import signal
import sys
import time
import uuid
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Iterator

from efe_tool import (
    AnchorCatalog,
    EfeAuthError,
    EfeClient,
    EfeError,
    EfeInvalidArgumentError,
    EfeRateLimitError,
    EfeUnavailableError,
    LocalSTEmbedder,
)


# ─────────────────────────────────────────────────────────────────────
# Optional OpenTelemetry — degrade gracefully when not installed
# ─────────────────────────────────────────────────────────────────────

try:
    from opentelemetry import trace
    from opentelemetry.sdk.trace import TracerProvider
    from opentelemetry.sdk.trace.export import (
        BatchSpanProcessor,
        ConsoleSpanExporter,
    )
    trace.set_tracer_provider(TracerProvider())
    trace.get_tracer_provider().add_span_processor(
        BatchSpanProcessor(ConsoleSpanExporter())
    )
    _tracer = trace.get_tracer("efe_tool.examples.04_production_client")
except ImportError:
    _tracer = None


@contextmanager
def maybe_span(name: str) -> Iterator[Any]:
    if _tracer is None:
        yield None
        return
    with _tracer.start_as_current_span(name) as span:
        yield span


# ─────────────────────────────────────────────────────────────────────
# Structured JSON logging
# ─────────────────────────────────────────────────────────────────────

class _JsonFormatter(logging.Formatter):
    """One JSON object per log line, suitable for log aggregators."""

    def format(self, record: logging.LogRecord) -> str:
        payload = {
            "ts": self.formatTime(record, "%Y-%m-%dT%H:%M:%S%z"),
            "level": record.levelname,
            "msg": record.getMessage(),
            "logger": record.name,
        }
        # Promote arbitrary `extra` kwargs to the top level.
        for key, value in record.__dict__.items():
            if key in ("args", "msg", "name", "levelname", "levelno",
                       "pathname", "filename", "module", "exc_info",
                       "exc_text", "stack_info", "lineno", "funcName",
                       "created", "msecs", "relativeCreated", "thread",
                       "threadName", "processName", "process", "taskName"):
                continue
            payload[key] = value
        if record.exc_info:
            payload["exc"] = self.formatException(record.exc_info)
        return json.dumps(payload, ensure_ascii=False)


def setup_logging() -> logging.Logger:
    handler = logging.StreamHandler()
    handler.setFormatter(_JsonFormatter())
    log = logging.getLogger("efe.production_example")
    log.setLevel(logging.INFO)
    # Replace any existing handlers to avoid duplicate output when
    # this script is imported under a test runner.
    log.handlers = [handler]
    log.propagate = False
    return log


log = setup_logging()


# ─────────────────────────────────────────────────────────────────────
# Retry with exponential backoff + full jitter
# ─────────────────────────────────────────────────────────────────────

def classify_with_retries(
    client: EfeClient,
    catalog: AnchorCatalog,
    text: str,
    *,
    max_attempts: int = 3,
    base_backoff_s: float = 0.2,
    max_backoff_s: float = 5.0,
):
    """Classify with exponential backoff + full jitter on retryable errors.

    Stops retrying on:
        * ``EfeAuthError`` — re-raises; caller refreshes token.
        * ``EfeInvalidArgumentError`` — re-raises; bug in caller.
        * Any non-``EfeError`` exception — re-raises (don't retry the
          unknown).
    """
    request_id = str(uuid.uuid4())
    last_exc: BaseException | None = None

    for attempt in range(1, max_attempts + 1):
        with maybe_span(f"classify.attempt.{attempt}") as span:
            try:
                return client.classify(text, catalog, request_id=request_id)
            except EfeAuthError:
                raise  # never retry auth failures
            except EfeInvalidArgumentError:
                raise  # request shape is wrong — retry won't help
            except (EfeRateLimitError, EfeUnavailableError) as exc:
                last_exc = exc
                if attempt == max_attempts:
                    raise
                # Full jitter: sleep ∈ [0, base × 2^(attempt-1)]
                cap = min(base_backoff_s * 2 ** (attempt - 1), max_backoff_s)
                sleep = random.uniform(0.0, cap)
                log.warning(
                    "retryable error — backing off",
                    extra={
                        "attempt": attempt,
                        "max_attempts": max_attempts,
                        "sleep_s": round(sleep, 3),
                        "grpc_code": exc.grpc_code,
                        "request_id": request_id,
                    },
                )
                time.sleep(sleep)
            except EfeError:
                # All other Efe* errors are non-retryable.
                raise
    # Defensive — should be unreachable.
    raise last_exc if last_exc else RuntimeError("retry loop fell through")


# ─────────────────────────────────────────────────────────────────────
# Graceful shutdown
# ─────────────────────────────────────────────────────────────────────

_shutdown = False


def _on_signal(*_args: object) -> None:
    global _shutdown
    _shutdown = True


# ─────────────────────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────────────────────

CATALOG_PATH = (
    Path(__file__).resolve().parent.parent / "catalogs" / "support_triage.yaml"
)

CASES = [
    "Someone charged my card without my consent.",
    "Where is my refund?",
    "L'application plante quand j'ouvre le menu.",
]


def main() -> int:
    token = os.environ.get("EFE_TOKEN", "").strip()
    if not token:
        log.error("EFE_TOKEN env var required (server has auth enabled)")
        return 2

    host = os.environ.get("EFE_HOST", "127.0.0.1:50051")
    insecure = os.environ.get("EFE_INSECURE", "true").lower() == "true"

    signal.signal(signal.SIGTERM, _on_signal)
    signal.signal(signal.SIGINT, _on_signal)

    embedder = LocalSTEmbedder.from_env()
    catalog = AnchorCatalog.load(CATALOG_PATH, embedder=embedder)

    log.info("starting production client", extra={
        "host": host, "insecure": insecure,
        "catalog": catalog.metadata.name,
        "version": catalog.metadata.version,
    })

    with EfeClient(
        endpoint=host, embedder=embedder,
        jwt=token, insecure=insecure, timeout_s=10.0,
    ) as client:
        # One pass over CASES — production code would loop forever
        # on a queue/Kafka/whatever.
        for text in CASES:
            if _shutdown:
                log.info("shutdown requested — exiting loop")
                break
            try:
                with maybe_span("classify"):
                    r = classify_with_retries(client, catalog, text)
            except EfeAuthError as exc:
                log.error("auth failed — token expired or revoked",
                          extra={"grpc_code": exc.grpc_code})
                return 3
            except Exception as exc:  # pragma: no cover — defensive
                log.exception("classification failed: %s", exc)
                continue

            log.info(
                "classified",
                extra={
                    "label": r.label,
                    "confidence": round(r.confidence, 3),
                    "reject": r.reject,
                    "rounds": getattr(r, "rounds", None),
                    "text": text[:80],   # truncated to avoid PII leak in logs
                },
            )

    log.info("graceful shutdown")
    return 0


if __name__ == "__main__":
    sys.exit(main())

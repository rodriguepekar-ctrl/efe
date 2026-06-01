"""Sprint D10 — gRPC Mode-1 stub server for example smoke tests.

This is a MINIMAL `efe.v1.SwarmService` implementation that returns
canned responses based on a deterministic hash of the request's
``raw_prompt``. It is **not** a model — it exists only so the four
``examples/0X_*.py`` scripts can run against a known-good endpoint
in CI without booting the real Rust binary (which requires Cargo +
build, ~3 min).

The mapping is intentionally simple:

    * "fraud" / "card" / "consent" / "hacked"           → fraud
    * "invoice" / "billing" / "charged" / "subscription" → billing
    * "?" suffix or "how" / "where" / "what"             → question
    * "crash" / "plante" / "doesn't work"                → bug
    * (anything else)                                    → fraud  (default)

Confidence is a fixed 0.92 and the swarm reports 7 rounds. The
response shape matches what `clients/python/efe_tool/_proto/efe/v1`
expects (proto3 JSON serialisation); we use the SDK's own pb2
modules so the wire contract stays in sync with the canonical
``.proto`` files.

Usage::

    python tests/stub_server.py [--port 50051]

The server prints "stub-server bound on PORT" to stdout when
ready, then idles until SIGTERM/SIGINT.
"""

from __future__ import annotations

import argparse
import sys
import time
from concurrent import futures
from pathlib import Path

# Make sure the SDK's proto stubs are importable. We resolve them
# relative to this file so the script works whether invoked from
# the repo root or from ``examples/tests/``.
_REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_REPO / "clients" / "python"))

import grpc

from efe_tool._proto.efe.v1 import efe_pb2, efe_pb2_grpc
from efe_tool._proto.efe.v1 import types_pb2


# ─────────────────────────────────────────────────────────────────────
# Canned classification logic
#
# The stub server CANNOT see the user's raw prompt — `EfeClient.classify`
# defaults to `include_raw_prompt=False` (PII concern, D8 audit).
# Instead we route by hashing the deterministic `request_id` (which the
# SDK derives from the catalog version + sha256(text)). This produces:
#
#   * deterministic per-text labels (good for assertion stability), and
#   * a balanced rotation across the 4 labels (so smoke tests can
#     assert ALL FOUR labels appear in the example's stdout).
#
# The keyword path is kept as a fallback for diagnostic runs where
# the operator manually invokes the stub with `--data-trace` or sets
# `include_raw_prompt=True` on the client.
# ─────────────────────────────────────────────────────────────────────

import hashlib

_LABELS = ("fraud", "billing", "question", "bug")
_CONFIDENCE = (0.92, 0.93, 0.88, 0.91)

_FRAUD_KEYWORDS = ("fraud", "card", "consent", "charged", "hacked", "stolen")
_BILLING_KEYWORDS = ("invoice", "billing", "subscription", "refund",
                     "facturation", "facturé", "abonnement")
_BUG_KEYWORDS = ("crash", "plante", "bug", "doesn't work", "ne fonctionne",
                 "broken", "error", "freeze", "menu paramètres")
_QUESTION_KEYWORDS = ("how", "where", "what", "comment", "où", "quand",
                      "quel", "?")


def _label_for(text: str) -> tuple[str, float]:
    """Return ``(label, confidence)`` for a deterministic stub response.

    Two paths:
      1. Keyword match if `text` is a real prompt (fallback for
         operator runs with `include_raw_prompt=True`).
      2. Otherwise hash `text` (which will be a request_id like
         "5fa8…d3e2") and pick a label by `hash mod 4`. This
         guarantees a balanced label distribution over many requests
         even though the stub is "blind" to the actual prompt.
    """
    lower = text.lower()
    # Path 1 — keyword detection (real prompt visible).
    if any(k in lower for k in _BUG_KEYWORDS):
        return "bug", 0.91
    if any(k in lower for k in _FRAUD_KEYWORDS):
        return "fraud", 0.92
    if any(k in lower for k in _BILLING_KEYWORDS):
        return "billing", 0.93
    if any(k in lower for k in _QUESTION_KEYWORDS):
        return "question", 0.88

    # Path 2 — hash-rotate over the 4 labels for a deterministic
    # balanced distribution. Use sha256 (not built-in hash()) so the
    # mapping stays stable across Python interpreter restarts —
    # essential for reproducible smoke tests.
    h = int.from_bytes(
        hashlib.sha256(text.encode("utf-8", errors="replace")).digest()[:4],
        "big",
    )
    idx = h % len(_LABELS)
    return _LABELS[idx], _CONFIDENCE[idx]


# ─────────────────────────────────────────────────────────────────────
# Servicer
# ─────────────────────────────────────────────────────────────────────

class StubSwarmServicer(efe_pb2_grpc.SwarmServiceServicer):
    """Returns one ExecuteResponse per Execute call, no streaming."""

    def Execute(self, request, context):
        # The SDK's classify() flow embeds locally then sends an
        # ExecuteRequest with a pre-embedded SwarmSpec. We don't
        # actually look at the spec — only at the raw_prompt
        # (which the SDK now sends with `include_raw_prompt=False`
        # by default, BUT for examples/02_custom_catalog.py the
        # text leaks through `request.raw_prompt`. Falls back to
        # the spec's anchor count as a fingerprint).
        #
        # For deterministic stub responses we hash the request_id
        # to pick a label distribution — this lets the smoke tests
        # assert specific labels appear.
        text = request.raw_prompt or request.request_id or ""
        label, confidence = _label_for(text)
        return efe_pb2.ExecuteResponse(
            swarm_id=f"stub-swarm-{hash(text) & 0xffff:04x}",
            request_id=request.request_id or f"stub-req-{int(time.time())}",
            output=types_pb2.DecodedOutput(text=label),
            confidence=confidence,
            verdict=types_pb2.VERDICT_CONVERGED,
            rounds_executed=7,
            total_ms=12.0,
            alignment_delta=0.05,
        )

    def ExecuteStreaming(self, request, context):
        """Single-final-chunk pseudo-stream."""
        final = self.Execute(request, context)
        yield efe_pb2.StreamChunk(final=final)

    def DescribeSpec(self, request, context):
        return efe_pb2.DescribeSpecResponse(
            valid=True,
            errors=[],
            hbm_bytes_estimate=0,
            rounds_expected=7,
        )


# ─────────────────────────────────────────────────────────────────────
# Entry-point
# ─────────────────────────────────────────────────────────────────────

def serve(port: int = 50051) -> grpc.Server:
    server = grpc.server(
        futures.ThreadPoolExecutor(max_workers=8),
        options=[
            ("grpc.so_reuseport", 1),
            ("grpc.max_send_message_length", 8 * 1024 * 1024),
            ("grpc.max_receive_message_length", 8 * 1024 * 1024),
        ],
    )
    efe_pb2_grpc.add_SwarmServiceServicer_to_server(
        StubSwarmServicer(), server,
    )
    bind_addr = f"127.0.0.1:{port}"
    actual_port = server.add_insecure_port(bind_addr)
    server.start()
    print(f"stub-server bound on {actual_port}", flush=True)
    return server


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    p.add_argument("--port", type=int, default=50051)
    args = p.parse_args()
    server = serve(args.port)
    try:
        server.wait_for_termination()
    except KeyboardInterrupt:
        server.stop(grace=2.0)
    return 0


if __name__ == "__main__":
    sys.exit(main())

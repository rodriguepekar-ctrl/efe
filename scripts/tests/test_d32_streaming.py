"""Sprint **D32** (M4 § 12) — pin tests for `Mode1Service.ExecuteStreaming`
activation.

Verifies :
  * `StreamChunk` proto has 4 oneof variants (RoundUpdate / Final / event / error).
  * Server-side handler emits a `RoundUpdate` chunk THEN a `Final` chunk.
  * Channel buffer is `mpsc::channel(64)` per M4 § 12.5.
  * Errors are wrapped in `StreamError` chunks (not raw transport errors).
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
EFE_PROTO = REPO_ROOT / "protos" / "efe" / "v1" / "efe.proto"
SWARM_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "swarm.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def test_proto_stream_chunk_has_four_oneof_variants() -> None:
    proto = _read(EFE_PROTO)
    # The oneof block must contain all four payload types.
    for variant in (
        "RoundUpdate round_update",
        "FinalVerdict final",
        "ServerEvent event",
        "StreamError error",
    ):
        assert variant in proto, f"oneof variant `{variant}` missing"


def test_proto_round_update_carries_all_fields() -> None:
    proto = _read(EFE_PROTO)
    # M4 § 12.2 contract : round, variance, confidence_so_far,
    # active_agents, elapsed_ms.
    body = proto[proto.find("message RoundUpdate") :]
    body = body[: body.find("}") + 1]
    for f in ("round", "variance", "confidence_so_far", "active_agents", "elapsed_ms"):
        assert f in body, f"RoundUpdate.{f} missing"


def test_proto_stream_error_present() -> None:
    proto = _read(EFE_PROTO)
    body = proto[proto.find("message StreamError") :]
    body = body[: body.find("}") + 1]
    for f in ("code", "message", "details"):
        assert f in body, f"StreamError.{f} missing"


def test_handler_uses_mpsc_channel_64() -> None:
    rs = _read(SWARM_RS)
    body = rs[rs.find("async fn execute_streaming(") :]
    # M4 § 12.5 — buffer size 64 (was 32 pre-D32).
    assert "mpsc::channel::<Result<StreamChunk, Status>>(64)" in body, (
        "execute_streaming must use mpsc::channel(64) per S-A4 audit fix"
    )


def test_handler_emits_round_update_chunk() -> None:
    rs = _read(SWARM_RS)
    body = rs[rs.find("async fn execute_streaming(") :]
    assert "stream_chunk::Payload::RoundUpdate" in body, (
        "execute_streaming must emit at least one RoundUpdate chunk"
    )


def test_handler_emits_final_verdict_chunk() -> None:
    rs = _read(SWARM_RS)
    body = rs[rs.find("async fn execute_streaming(") :]
    assert "stream_chunk::Payload::Final" in body


def test_handler_wraps_errors_in_stream_error_chunk() -> None:
    rs = _read(SWARM_RS)
    body = rs[rs.find("async fn execute_streaming(") :]
    assert "stream_chunk::Payload::Error" in body, (
        "errors must be wrapped in StreamChunk.Error per M4 § 12.2"
    )


def test_handler_detects_client_cancel_via_is_closed() -> None:
    rs = _read(SWARM_RS)
    body = rs[rs.find("async fn execute_streaming(") :]
    # M4 § 12.4 SLO S-10 — client cancel detected via tx.is_closed.
    assert "tx.is_closed()" in body or "is_closed()" in body


def test_streaming_path_required_scope_swarm_execute() -> None:
    """Sprint D22 — the streaming RPC must enforce both granular
    `swarm:execute` AND broad `data` scope (defence in depth)."""
    rs = _read(SWARM_RS)
    body = rs[rs.find("async fn execute_streaming(") :]
    assert "Scope::SWARM_EXECUTE" in body
    assert "BroadScope::DATA" in body


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))

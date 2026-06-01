"""Deserialise wire-level dicts back into Python dataclasses."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any

from efe_swarm_integration.errors import SchemaViolation
from efe_swarm_integration.types import (
    BreachReport,
    DecodedOutput,
    DescribeSpecReply,
    ExecuteResponse,
    ServerEvent,
    StreamChunk,
    TextDelta,
    VerdictEnum,
)

__all__ = [
    "parse_describe_spec_reply",
    "parse_execute_response",
    "parse_stream_chunk",
]


def _proto_ts_to_datetime(ts: dict[str, Any]) -> datetime:
    seconds = int(ts.get("seconds", 0))
    nanos = int(ts.get("nanos", 0))
    total = seconds + nanos / 1_000_000_000
    return datetime.fromtimestamp(total, tz=timezone.utc)


def _verdict_from_wire(v: str | int) -> VerdictEnum:
    # protobuf enums serialize as uppercase strings or ints.
    if isinstance(v, int):
        mapping_int = {
            0: VerdictEnum.UNSPECIFIED,
            1: VerdictEnum.CONTINUE,
            2: VerdictEnum.CONVERGED,
            3: VerdictEnum.DIVERGENCE,
            4: VerdictEnum.MODE_COLLAPSE,
        }
        return mapping_int.get(v, VerdictEnum.UNSPECIFIED)
    norm = str(v).lower().replace("verdict_", "")
    return {
        "unspecified": VerdictEnum.UNSPECIFIED,
        "continue": VerdictEnum.CONTINUE,
        "converged": VerdictEnum.CONVERGED,
        "divergence": VerdictEnum.DIVERGENCE,
        "mode_collapse": VerdictEnum.MODE_COLLAPSE,
    }.get(norm, VerdictEnum.UNSPECIFIED)


def _parse_decoded_output(d: dict[str, Any]) -> DecodedOutput:
    if "text" in d and d["text"] is not None:
        text = d["text"]
        if isinstance(text, dict):  # oneof wrapper form
            text = text.get("text", "")
        return DecodedOutput(kind="text", text=str(text))
    if "raw" in d and d["raw"] is not None:
        raw = d["raw"]
        vals = raw.get("values") if isinstance(raw, dict) else raw
        return DecodedOutput(
            kind="raw_centroid",
            raw_centroid=tuple(float(x) for x in (vals or ())),
        )
    raise SchemaViolation("DecodedOutput missing payload", detail={"got": list(d)})


def parse_execute_response(wire: dict[str, Any]) -> ExecuteResponse:
    """Convert a dict form of ``efe.v1.ExecuteResponse`` into Python."""
    try:
        output = _parse_decoded_output(wire.get("output") or {})
        verdict = _verdict_from_wire(wire.get("verdict", 0))
        return ExecuteResponse(
            swarm_id=str(wire.get("swarm_id", "")),
            request_id=str(wire.get("request_id", "")),
            output=output,
            confidence=float(wire.get("confidence", 0.0)),
            verdict=verdict,
            rounds_executed=int(wire.get("rounds_executed", 0)),
            total_ms=float(wire.get("total_ms", 0.0)),
            alignment_delta=float(wire.get("alignment_delta", 0.0)),
        )
    except SchemaViolation:
        raise
    except Exception as exc:
        raise SchemaViolation(
            f"failed to parse ExecuteResponse: {exc}",
            detail={"wire_keys": list(wire)},
        ) from exc


def parse_stream_chunk(wire: dict[str, Any]) -> StreamChunk:
    """Parse a :class:`efe.v1.StreamChunk` variant."""
    if "text_delta" in wire and wire["text_delta"] is not None:
        return StreamChunk(
            payload_kind="text_delta",
            text_delta=TextDelta(text=str(wire["text_delta"])),
        )
    if "final" in wire and wire["final"] is not None:
        return StreamChunk(
            payload_kind="final",
            final=parse_execute_response(wire["final"]),
        )
    if "event" in wire and wire["event"] is not None:
        ev = wire["event"]
        labels = tuple(
            (k, str(v)) for k, v in sorted((ev.get("labels") or {}).items())
        )
        return StreamChunk(
            payload_kind="server_event",
            server_event=ServerEvent(
                kind=str(ev.get("kind", "")),
                message=str(ev.get("message", "")),
                labels=labels,
            ),
        )
    raise SchemaViolation("StreamChunk missing payload")


def parse_describe_spec_reply(wire: dict[str, Any]) -> DescribeSpecReply:
    return DescribeSpecReply(
        valid=bool(wire.get("valid", False)),
        errors=tuple(str(e) for e in wire.get("errors", [])),
        hbm_bytes_estimate=int(wire.get("hbm_bytes_estimate", 0)),
        rounds_expected=int(wire.get("rounds_expected", 0)),
    )


def parse_breach_report(wire: dict[str, Any]) -> BreachReport:
    ts_raw = wire.get("timestamp")
    if isinstance(ts_raw, dict):
        ts = _proto_ts_to_datetime(ts_raw)
    elif isinstance(ts_raw, str):
        ts = datetime.fromisoformat(ts_raw.replace("Z", "+00:00"))
    else:
        ts = datetime.now(tz=timezone.utc)
    return BreachReport(
        cone_key=str(wire.get("cone_key", "")),
        swarm_id=str(wire.get("swarm_id", "")),
        observed_delta=float(wire.get("observed_delta", 0.0)),
        cone_radius=float(wire.get("cone_radius", 0.0)),
        timestamp=ts,
        context=str(wire.get("context", "")),
    )

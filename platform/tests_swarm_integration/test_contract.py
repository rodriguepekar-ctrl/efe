"""Contract adapter tests : validation + round-trip + snapshot."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_swarm_integration.config import MAX_ANCHORS, MAX_POPULATION, MAX_ROUNDS
from efe_swarm_integration.contract import (
    CONTRACT_SNAPSHOT,
    assert_snapshot_stable,
    parse_describe_spec_reply,
    parse_execute_response,
    parse_stream_chunk,
    serialise_execute_request,
    serialise_swarm_spec,
    validate_swarm_spec,
)
from efe_swarm_integration.contract.adapter_admin import (
    admin_evolve_op_to_dict,
    admin_register_cone_to_dict,
    parse_cone_handle,
)
from efe_swarm_integration.errors import ContractSnapshotMismatch, SchemaViolation
from efe_swarm_integration.types import (
    AlignmentCone,
    Anchor,
    ConeEvolutionOp,
    ConeRef,
    ConeSpec,
    DecodeMode,
    ExecuteRequest,
    PopulationEntry,
    PopulationMix,
    ResourceBudget,
    SwarmSpec,
    TemplateCode,
    VerdictEnum,
)


def _base_spec() -> SwarmSpec:
    return SwarmSpec(
        dim_latent=4,
        problem_embedding=(0.1, 0.2, 0.3, 0.4),
        population=PopulationMix(
            entries=(PopulationEntry(TemplateCode.NaSem, 16),)
        ),
        alignment_cone=None,
        alignment_cone_ref=ConeRef("acme", "k", 1),
    )


def test_serialise_swarm_spec_validates() -> None:
    spec = _base_spec()
    out = serialise_swarm_spec(spec)
    assert out["dim_latent"] == 4
    assert out["alignment_cone_ref"]["tenant"] == "acme"


def test_serialise_swarm_spec_rejects_over_population() -> None:
    spec = SwarmSpec(
        dim_latent=1,
        problem_embedding=(0.0,),
        population=PopulationMix(
            entries=(PopulationEntry(TemplateCode.NaSem, MAX_POPULATION + 1),)
        ),
        alignment_cone=None,
        alignment_cone_ref=ConeRef("t", "k", 1),
    )
    with pytest.raises(SchemaViolation):
        serialise_swarm_spec(spec)


def test_serialise_swarm_spec_rejects_big_max_rounds() -> None:
    spec = SwarmSpec(
        dim_latent=1,
        problem_embedding=(0.0,),
        population=PopulationMix(
            entries=(PopulationEntry(TemplateCode.NaSem, 1),)
        ),
        alignment_cone=None,
        alignment_cone_ref=ConeRef("t", "k", 1),
        max_rounds=MAX_ROUNDS + 1,
    )
    with pytest.raises(SchemaViolation):
        serialise_swarm_spec(spec)


def test_execute_request_includes_deadline() -> None:
    spec = _base_spec()
    dt = datetime(2026, 4, 23, tzinfo=timezone.utc)
    req = ExecuteRequest(spec=spec, deadline=dt, decode_mode=DecodeMode("pass_through"))
    out = serialise_execute_request(req)
    assert "deadline" in out


def test_parse_execute_response_roundtrip() -> None:
    wire = {
        "swarm_id": "swarm_123",
        "request_id": "req_456",
        "output": {"raw": {"values": [0.1, 0.2, 0.3]}},
        "confidence": 0.83,
        "verdict": 2,  # VERDICT_CONVERGED
        "rounds_executed": 4,
        "total_ms": 123.5,
        "alignment_delta": 0.12,
    }
    resp = parse_execute_response(wire)
    assert resp.swarm_id == "swarm_123"
    assert resp.verdict == VerdictEnum.CONVERGED
    assert resp.output.raw_centroid == (0.1, 0.2, 0.3)


def test_parse_execute_response_text_output() -> None:
    wire = {
        "swarm_id": "s",
        "request_id": "r",
        "output": {"text": "fraud"},
        "confidence": 0.9,
        "verdict": "VERDICT_CONVERGED",
        "rounds_executed": 3,
        "total_ms": 42.0,
        "alignment_delta": 0.05,
    }
    resp = parse_execute_response(wire)
    assert resp.output.text == "fraud"
    assert resp.verdict == VerdictEnum.CONVERGED


def test_parse_stream_chunk_final() -> None:
    chunk = parse_stream_chunk(
        {
            "final": {
                "swarm_id": "s",
                "request_id": "r",
                "output": {"text": "x"},
                "confidence": 0.5,
                "verdict": 2,
                "rounds_executed": 1,
                "total_ms": 1.0,
                "alignment_delta": 0.1,
            }
        }
    )
    assert chunk.payload_kind == "final"


def test_parse_stream_chunk_text_delta() -> None:
    chunk = parse_stream_chunk({"text_delta": "hello"})
    assert chunk.payload_kind == "text_delta"
    assert chunk.text_delta is not None


def test_parse_stream_chunk_server_event() -> None:
    chunk = parse_stream_chunk(
        {"event": {"kind": "round.completed", "labels": {"n": "1"}}}
    )
    assert chunk.payload_kind == "server_event"
    assert chunk.server_event is not None and chunk.server_event.kind == "round.completed"


def test_parse_describe_spec_reply() -> None:
    reply = parse_describe_spec_reply(
        {"valid": True, "errors": [], "hbm_bytes_estimate": 1024, "rounds_expected": 3}
    )
    assert reply.valid
    assert reply.hbm_bytes_estimate == 1024


def test_admin_register_cone_payload() -> None:
    spec = ConeSpec(
        tenant="acme",
        task="triage",
        version=1,
        cone=AlignmentCone(
            anchors=(Anchor(values=(0.0, 1.0)),), max_projection_distance=0.5
        ),
        created_by="alice",
        description="d",
        labels=(("k", "v"),),
    )
    out = admin_register_cone_to_dict(spec)
    assert out["tenant"] == "acme"
    assert out["labels"] == {"k": "v"}
    assert out["cone"]["anchors"][0]["values"] == [0.0, 1.0]


def test_admin_evolve_op_variants() -> None:
    a = admin_evolve_op_to_dict(
        ConeEvolutionOp(kind="update_radius", update_radius=0.4)
    )
    assert a == {"update_radius": 0.4}
    b = admin_evolve_op_to_dict(
        ConeEvolutionOp(kind="remove_anchor_idx", remove_anchor_idx=3)
    )
    assert b == {"remove_anchor_idx": 3}
    c = admin_evolve_op_to_dict(
        ConeEvolutionOp(
            kind="replace_anchor",
            replace_anchor=(1, Anchor(values=(0.1, 0.2))),
        )
    )
    assert c["replace_anchor"]["idx"] == 1


def test_parse_cone_handle() -> None:
    wire = {
        "key": "acme/triage@v1",
        "cone": {
            "cone": {"anchors": [{"values": [0.0, 1.0]}], "max_projection_distance": 0.3},
            "metadata": {
                "created_at": {"seconds": 1745417600, "nanos": 0},
                "created_by": "alice",
                "description": "",
                "labels": {"x": "y"},
            },
        },
    }
    handle = parse_cone_handle(wire)
    assert handle.key == "acme/triage@v1"
    assert handle.canonical.metadata.labels == (("x", "y"),)


def test_snapshot_stable_against_itself() -> None:
    # Self-check ; the snapshot is self-consistent.
    assert_snapshot_stable(CONTRACT_SNAPSHOT)


def test_snapshot_detects_drift() -> None:
    bad = {
        "messages": {
            "SwarmSpec": {
                "fields": [
                    {"name": "problem_embedding", "number": 99, "type": "TYPE_FLOAT"},
                ]
            }
        }
    }
    with pytest.raises(ContractSnapshotMismatch):
        assert_snapshot_stable(bad)

"""Frozen snapshot of the ``efe.v1`` wire layout.

This file is the contract we agree with the Rust server. Any change
to a field number/name/type here is a **breaking change** and requires
an ADR + a migration across both sides.
"""

from __future__ import annotations

from typing import Any

__all__ = ["CONTRACT_SNAPSHOT", "assert_snapshot_stable"]


CONTRACT_SNAPSHOT: dict[str, Any] = {
    "schema_version": "efe.v1",
    "messages": {
        "SwarmSpec": {
            "fields": [
                {"name": "swarm_id", "number": 1, "type": "TYPE_STRING"},
                {"name": "problem_embedding", "number": 2, "type": "TYPE_FLOAT",
                 "label": "LABEL_REPEATED"},
                {"name": "dim_latent", "number": 3, "type": "TYPE_UINT32"},
                {"name": "population", "number": 4, "type": "TYPE_MESSAGE",
                 "label": "LABEL_REPEATED"},
                {"name": "max_rounds", "number": 5, "type": "TYPE_UINT32"},
                {"name": "convergence", "number": 6, "type": "TYPE_MESSAGE"},
                {"name": "alignment_cone", "number": 7, "type": "TYPE_MESSAGE"},
                {"name": "alignment_cone_ref", "number": 8, "type": "TYPE_MESSAGE"},
                {"name": "budget", "number": 9, "type": "TYPE_MESSAGE"},
            ],
        },
        "ExecuteRequest": {
            "fields": [
                {"name": "request_id", "number": 1, "type": "TYPE_STRING"},
                {"name": "raw_prompt", "number": 2, "type": "TYPE_STRING"},
                {"name": "spec", "number": 3, "type": "TYPE_MESSAGE"},
                {"name": "decode_mode", "number": 4, "type": "TYPE_MESSAGE"},
                {"name": "deadline", "number": 5, "type": "TYPE_MESSAGE"},
            ],
        },
        "ExecuteResponse": {
            "fields": [
                {"name": "swarm_id", "number": 1, "type": "TYPE_STRING"},
                {"name": "request_id", "number": 2, "type": "TYPE_STRING"},
                {"name": "output", "number": 3, "type": "TYPE_MESSAGE"},
                {"name": "confidence", "number": 4, "type": "TYPE_FLOAT"},
                {"name": "verdict", "number": 5, "type": "TYPE_ENUM"},
                {"name": "rounds_executed", "number": 6, "type": "TYPE_UINT32"},
                {"name": "total_ms", "number": 7, "type": "TYPE_FLOAT"},
                {"name": "alignment_delta", "number": 8, "type": "TYPE_FLOAT"},
            ],
        },
        "Verdict": {
            "enum_values": [
                {"name": "VERDICT_UNSPECIFIED", "number": 0},
                {"name": "VERDICT_CONTINUE", "number": 1},
                {"name": "VERDICT_CONVERGED", "number": 2},
                {"name": "VERDICT_DIVERGENCE", "number": 3},
                {"name": "VERDICT_MODE_COLLAPSE", "number": 4},
            ],
        },
    },
}


def assert_snapshot_stable(current_snapshot: dict[str, Any]) -> None:
    """Compare a runtime snapshot against the frozen snapshot.

    Used by the contract CI test : if a :mod:`efe.v1` message changes
    a field number (or adds/removes a field), this raises with a
    precise diff.
    """
    expected_messages = CONTRACT_SNAPSHOT["messages"]
    actual_messages = current_snapshot.get("messages", {})
    errors: list[str] = []
    for name, expected in expected_messages.items():
        actual = actual_messages.get(name)
        if actual is None:
            errors.append(f"missing message: {name}")
            continue
        # Compare fields by (name, number, type).
        expected_fields = {f["name"]: f for f in expected.get("fields", [])}
        actual_fields = {f["name"]: f for f in actual.get("fields", [])}
        for fname, ef in expected_fields.items():
            af = actual_fields.get(fname)
            if af is None:
                errors.append(f"{name}.{fname}: missing")
                continue
            if af.get("number") != ef.get("number"):
                errors.append(
                    f"{name}.{fname}: number mismatch "
                    f"expected={ef['number']} actual={af.get('number')}"
                )
            if af.get("type") != ef.get("type"):
                errors.append(
                    f"{name}.{fname}: type mismatch "
                    f"expected={ef['type']} actual={af.get('type')}"
                )
    if errors:
        from efe_swarm_integration.errors import ContractSnapshotMismatch

        raise ContractSnapshotMismatch(
            "contract snapshot drift detected",
            detail={"errors": errors},
        )

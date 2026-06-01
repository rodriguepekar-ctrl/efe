"""Smoke tests for the gRPC transport adapter.

The real transport can only be exercised against a running efe-server,
which is out of scope for unit tests. We instead verify:

* the module imports cleanly when ``grpcio`` + generated stubs are
  available,
* the outbound converters produce structurally valid proto messages
  from neutral dataclasses.

If the stubs are not importable in the dev environment, the whole
module is skipped.
"""

from __future__ import annotations

import pytest

grpc = pytest.importorskip("grpc")

try:
    from efe_platform.swarm import _grpc_transport as gt  # noqa: F401
except ImportError as e:
    pytest.skip(f"gRPC transport unavailable: {e}", allow_module_level=True)


from efe_platform.swarm.protocol import (
    AnchorRef,
    ConeRef,
    ConvergenceThresholds,
    DecodeMode,
    ExecuteRequest,
    PopulationEntry,
    ResourceBudget,
    SwarmSpec,
)


def _spec_with_ref() -> SwarmSpec:
    return SwarmSpec(
        problem_embedding=tuple(0.1 * i for i in range(16)),
        dim_latent=16,
        population=(
            PopulationEntry("na-sem", 512),
            PopulationEntry("na-log", 384),
            PopulationEntry("na-crit", 128),
        ),
        max_rounds=8,
        convergence=ConvergenceThresholds(),
        budget=ResourceBudget(),
        alignment_cone=None,
        alignment_cone_ref=ConeRef("acme", "routing", 1),
    )


def _spec_with_anchors() -> SwarmSpec:
    return SwarmSpec(
        problem_embedding=tuple(0.1 * i for i in range(16)),
        dim_latent=16,
        population=(PopulationEntry("na-sem", 64),),
        max_rounds=4,
        convergence=ConvergenceThresholds(),
        budget=ResourceBudget(),
        alignment_cone=(
            AnchorRef(tuple(0.1 * i for i in range(16))),
            AnchorRef(tuple(0.2 * i for i in range(16))),
        ),
        alignment_cone_ref=None,
        max_projection_distance=1.5,
    )


def test_module_importable() -> None:
    from efe_platform.swarm._grpc_transport import GrpcSwarmTransport

    assert GrpcSwarmTransport is not None


def test_to_proto_spec_with_cone_ref() -> None:
    from efe_platform.swarm._grpc_transport import _to_proto_swarm_spec

    proto = _to_proto_swarm_spec(_spec_with_ref())
    assert proto.dim_latent == 16
    # oneof "cone_source" → alignment_cone_ref populated.
    assert proto.WhichOneof("cone_source") == "alignment_cone_ref"
    assert proto.alignment_cone_ref.tenant == "acme"
    assert proto.alignment_cone_ref.task == "routing"
    assert proto.alignment_cone_ref.version == 1


def test_to_proto_spec_with_inline_cone() -> None:
    from efe_platform.swarm._grpc_transport import _to_proto_swarm_spec

    proto = _to_proto_swarm_spec(_spec_with_anchors())
    assert proto.WhichOneof("cone_source") == "alignment_cone"
    assert len(proto.alignment_cone.anchors) == 2
    assert proto.alignment_cone.max_projection_distance == pytest.approx(1.5)


def test_to_proto_execute_request_sets_request_id() -> None:
    from efe_platform.swarm._grpc_transport import _to_proto_execute_request

    req = ExecuteRequest(
        spec=_spec_with_ref(),
        raw_prompt="hello",
        request_id="rid-42",
        decode_mode=DecodeMode(kind="text", max_tokens=64),
    )
    proto = _to_proto_execute_request(req)
    assert proto.request_id == "rid-42"
    assert proto.raw_prompt == "hello"
    assert proto.decode_mode.WhichOneof("mode") == "text"
    assert proto.decode_mode.text.max_tokens == 64


def test_to_proto_execute_request_passthrough_mode() -> None:
    from efe_platform.swarm._grpc_transport import _to_proto_execute_request

    req = ExecuteRequest(
        spec=_spec_with_ref(),
        raw_prompt="",
        request_id="x",
        decode_mode=DecodeMode(kind="pass_through"),
    )
    proto = _to_proto_execute_request(req)
    assert proto.decode_mode.WhichOneof("mode") == "pass_through"

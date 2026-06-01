"""Real gRPC transport wrapping the ``efe.v1`` generated stubs.

This module is lazy-imported by :func:`efe_platform.swarm.client._lazy_grpc_transport`
— keeping ``grpcio`` and the generated protobuf stubs **optional** so that
consumers using only the in-memory transport (tests, notebooks,
standalone dev mode) do not need to install them.

The generated stubs ship with the ``efe-tool`` package
(``efe_tool._proto.efe.v1``). Installing ``efe-platform[client]``
pulls them in transitively; callers who have the stubs on their
``PYTHONPATH`` from a different source will also work — the import
resolution is lenient.

Conversion strategy
-------------------

* Outbound: :class:`SwarmSpec` / :class:`ExecuteRequest` dataclasses are
  walked and mapped field-by-field into their proto counterparts.
* Inbound: the proto :class:`ExecuteResponse` is decoded into the
  neutral dataclass by reading its accessors — we never leak a proto
  message outside this module.

This is the single seam where the wire shape is allowed to matter.
Every other file in :mod:`efe_platform` stays proto-agnostic.
"""

from __future__ import annotations

import contextlib
from collections.abc import AsyncIterator
from typing import Any

from efe_platform.errors import (
    EfePlatformError,
    SwarmDeadlineExceeded,
    SwarmInvalidSpec,
    SwarmUnavailable,
)
from efe_platform.swarm.client import SwarmEndpoint
from efe_platform.swarm.protocol import (
    DecodedPayload,
    ExecuteRequest,
    ExecuteResponse,
    SwarmSpec,
    Verdict,
)

# ---------------------------------------------------------------------------
#  Lazy import of grpcio + generated stubs
# ---------------------------------------------------------------------------

try:
    import grpc
    import grpc.aio
except ImportError as exc:  # pragma: no cover — optional dep
    raise ImportError(
        "grpcio is required for the gRPC swarm transport. Install with "
        "`pip install efe-platform[client]` or `pip install grpcio`."
    ) from exc

try:
    # Canonical location — shipped with the efe-tool package.
    from efe_tool._proto.efe.v1 import efe_pb2, efe_pb2_grpc, types_pb2
except ImportError:  # pragma: no cover — fall back to a top-level install
    try:
        from efe.v1 import efe_pb2, efe_pb2_grpc, types_pb2
    except ImportError as exc:
        raise ImportError(
            "Generated efe.v1 proto stubs not found. Install the "
            "``efe-tool`` package (which ships them) or add your own "
            "proto output directory to PYTHONPATH."
        ) from exc


# ---------------------------------------------------------------------------
#  Transport
# ---------------------------------------------------------------------------


class GrpcSwarmTransport:
    """Implements :class:`SwarmTransport` against a real efe-server."""

    def __init__(self, endpoint: SwarmEndpoint) -> None:
        self._endpoint = endpoint
        self._channel: Any = self._make_channel(endpoint)
        self._stub = efe_pb2_grpc.SwarmServiceStub(self._channel)
        self._closed = False

    @staticmethod
    def _make_channel(endpoint: SwarmEndpoint) -> Any:
        target = endpoint.target
        options: list[tuple[str, Any]] = [
            # Send keep-alive pings every 20 s to hold long-lived channels.
            ("grpc.keepalive_time_ms", 20_000),
            ("grpc.keepalive_timeout_ms", 5_000),
            ("grpc.keepalive_permit_without_calls", 1),
            ("grpc.http2.max_pings_without_data", 0),
            # Allow reasonably large responses (64 MiB).
            ("grpc.max_receive_message_length", 64 * 1024 * 1024),
        ]
        if endpoint.tls:
            creds_kwargs: dict[str, Any] = {}
            if endpoint.tls_ca_path:
                with open(endpoint.tls_ca_path, "rb") as fh:
                    creds_kwargs["root_certificates"] = fh.read()
            creds = grpc.ssl_channel_credentials(**creds_kwargs)
            return grpc.aio.secure_channel(target, creds, options=options)
        return grpc.aio.insecure_channel(target, options=options)

    # ---------------------------------------------------- Public operations

    async def execute(
        self,
        req: ExecuteRequest,
        *,
        metadata: tuple[tuple[str, str], ...],
        deadline_s: float,
    ) -> ExecuteResponse:
        self._ensure_open()
        proto_req = _to_proto_execute_request(req)
        try:
            resp = await self._stub.Execute(
                proto_req,
                metadata=metadata,
                timeout=deadline_s,
            )
        except grpc.aio.AioRpcError as exc:
            raise _translate_rpc_error(exc) from exc
        return _from_proto_execute_response(resp)

    async def execute_streaming(
        self,
        req: ExecuteRequest,
        *,
        metadata: tuple[tuple[str, str], ...],
        deadline_s: float,
    ) -> AsyncIterator[dict[str, Any]]:
        self._ensure_open()
        proto_req = _to_proto_execute_request(req)
        try:
            async for chunk in self._stub.ExecuteStreaming(
                proto_req,
                metadata=metadata,
                timeout=deadline_s,
            ):
                yield _translate_stream_chunk(chunk)
        except grpc.aio.AioRpcError as exc:
            raise _translate_rpc_error(exc) from exc

    async def describe_spec(
        self,
        spec: SwarmSpec,
        *,
        deadline_s: float,
    ) -> dict[str, Any]:
        self._ensure_open()
        proto_req = efe_pb2.DescribeSpecRequest(spec=_to_proto_swarm_spec(spec))
        try:
            resp = await self._stub.DescribeSpec(proto_req, timeout=deadline_s)
        except grpc.aio.AioRpcError as exc:
            raise _translate_rpc_error(exc) from exc
        return {
            "valid": bool(resp.valid),
            "errors": tuple(resp.errors),
            "hbm_bytes_estimate": int(resp.hbm_bytes_estimate),
            "rounds_expected": int(resp.rounds_expected),
        }

    async def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        with contextlib.suppress(Exception):
            await self._channel.close()

    # ------------------------------------------------------------- helpers

    def _ensure_open(self) -> None:
        if self._closed:
            raise SwarmUnavailable("transport closed")


# ---------------------------------------------------------------------------
#  Outbound converters (neutral → proto)
# ---------------------------------------------------------------------------


def _to_proto_execute_request(req: ExecuteRequest) -> Any:
    kwargs: dict[str, Any] = {
        "spec": _to_proto_swarm_spec(req.spec),
        "raw_prompt": req.raw_prompt,
    }
    if req.request_id:
        kwargs["request_id"] = req.request_id
    kwargs["decode_mode"] = _to_proto_decode_mode(req.decode_mode)
    # Deadline field is a google.protobuf.Timestamp — leave unset and
    # rely on the per-RPC timeout for now. A future enhancement could
    # translate deadline_ms into an absolute Timestamp.
    return efe_pb2.ExecuteRequest(**kwargs)


def _to_proto_swarm_spec(spec: SwarmSpec) -> Any:
    kwargs: dict[str, Any] = {
        "dim_latent": spec.dim_latent,
        "max_rounds": spec.max_rounds,
        "problem_embedding": list(spec.problem_embedding),
        "population": [
            types_pb2.PopulationEntry(
                template_code=entry.template_code, count=entry.count
            )
            for entry in spec.population
        ],
        "convergence": types_pb2.ConvergenceThresholds(
            epsilon_variance=spec.convergence.epsilon_variance,
            epsilon_gradient=spec.convergence.epsilon_gradient,
            epsilon_stability=spec.convergence.epsilon_stability,
            stability_window=spec.convergence.stability_window,
        ),
        "budget": types_pb2.ResourceBudget(
            max_wall_clock_ms=spec.budget.max_wall_clock_ms,
            max_energy_wh=spec.budget.max_energy_wh,
            max_hbm_bytes=spec.budget.max_hbm_bytes,
            max_recursion_depth=spec.budget.max_recursion_depth,
        ),
    }
    if spec.swarm_id:
        kwargs["swarm_id"] = spec.swarm_id
    # Cone source is a oneof: exactly one side populated.
    if spec.alignment_cone is not None:
        kwargs["alignment_cone"] = types_pb2.AlignmentCone(
            anchors=[
                types_pb2.Anchor(values=list(a.values))
                for a in spec.alignment_cone
            ],
            max_projection_distance=spec.max_projection_distance,
        )
    elif spec.alignment_cone_ref is not None:
        kwargs["alignment_cone_ref"] = types_pb2.ConeRef(
            tenant=spec.alignment_cone_ref.tenant,
            task=spec.alignment_cone_ref.task,
            version=spec.alignment_cone_ref.version,
        )
    else:  # pragma: no cover — SpecBuilder guards this
        raise SwarmInvalidSpec("spec has no cone source")
    return types_pb2.SwarmSpec(**kwargs)


def _to_proto_decode_mode(mode: Any) -> Any:
    """Translate a :class:`DecodeMode` into the oneof proto shape."""
    if mode.kind == "text":
        return types_pb2.DecodeMode(
            text=types_pb2.TextMode(max_tokens=mode.max_tokens)
        )
    return types_pb2.DecodeMode(pass_through=types_pb2.PassThroughMode())


# ---------------------------------------------------------------------------
#  Inbound converters (proto → neutral)
# ---------------------------------------------------------------------------


def _from_proto_execute_response(resp: Any) -> ExecuteResponse:
    return ExecuteResponse(
        swarm_id=str(resp.swarm_id),
        request_id=str(resp.request_id),
        payload=_from_proto_decoded_output(resp.output),
        confidence=float(resp.confidence),
        verdict=Verdict.from_proto(int(resp.verdict)),
        rounds_executed=int(resp.rounds_executed),
        total_ms=float(resp.total_ms),
        alignment_delta=float(resp.alignment_delta),
    )


def _from_proto_decoded_output(out: Any) -> DecodedPayload:
    which = out.WhichOneof("payload")
    if which == "text":
        return DecodedPayload(text=str(out.text))
    if which == "raw":
        return DecodedPayload(centroid=tuple(float(v) for v in out.raw.values))
    # If neither set, treat as empty text — keeps downstream honest.
    return DecodedPayload(text="")


def _translate_stream_chunk(chunk: Any) -> dict[str, Any]:
    which = chunk.WhichOneof("payload")
    if which == "text_delta":
        return {"kind": "text_delta", "payload": str(chunk.text_delta)}
    if which == "final":
        return {
            "kind": "final",
            "payload": _from_proto_execute_response(chunk.final),
        }
    if which == "event":
        ev = chunk.event
        return {
            "kind": "event",
            "payload": {
                "kind": str(ev.kind),
                "message": str(ev.message),
                "labels": dict(ev.labels),
            },
        }
    return {"kind": "unknown", "payload": None}


# ---------------------------------------------------------------------------
#  Error translation
# ---------------------------------------------------------------------------


def _translate_rpc_error(exc: Any) -> EfePlatformError:
    code = exc.code()
    detail = exc.details() or str(exc)
    if code == grpc.StatusCode.DEADLINE_EXCEEDED:
        return SwarmDeadlineExceeded(detail)
    if code == grpc.StatusCode.INVALID_ARGUMENT:
        return SwarmInvalidSpec(detail)
    if code == grpc.StatusCode.UNAVAILABLE:
        return SwarmUnavailable(detail)
    # Default: unavailable (retryable) so the client does its best.
    return SwarmUnavailable(f"{code.name}: {detail}")


__all__ = ["GrpcSwarmTransport"]

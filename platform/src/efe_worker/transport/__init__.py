"""Transport — gRPC orchestrator stub, heartbeat loop, S3 blob store.

Public surface:

    OrchestratorClient       — protocol (4 RPCs)
    FakeOrchestratorClient   — in-memory fake for tests
    HttpOrchestratorClient   — thin HTTP-JSON impl (no gRPC codegen needed)
    HeartbeatRequest, CheckpointRequest, ResultRequest, ErrorRequest
    heartbeat_loop           — async loop
    BlobStore                — protocol
    FakeBlobStore            — in-memory fake
    LocalBlobStore           — filesystem-backed, file:// URIs
"""

from __future__ import annotations

from efe_worker.transport.grpc_client import (
    CheckpointRequest,
    ErrorRequest,
    FakeOrchestratorClient,
    HeartbeatRequest,
    HttpOrchestratorClient,
    OrchestratorClient,
    ResultRequest,
)
from efe_worker.transport.heartbeat import heartbeat_loop
from efe_worker.transport.s3 import BlobStore, FakeBlobStore, LocalBlobStore

__all__ = [
    "BlobStore",
    "CheckpointRequest",
    "ErrorRequest",
    "FakeBlobStore",
    "FakeOrchestratorClient",
    "HeartbeatRequest",
    "HttpOrchestratorClient",
    "LocalBlobStore",
    "OrchestratorClient",
    "ResultRequest",
    "heartbeat_loop",
]

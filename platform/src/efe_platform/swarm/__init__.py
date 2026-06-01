"""Async interface to the Rust efe-server (SwarmService / AdminService).

Public names re-exported here are the stable surface callers should
import from — the internal module layout may shift between minor
versions without notice.
"""

from __future__ import annotations

from efe_platform.swarm.client import SwarmClient, SwarmEndpoint
from efe_platform.swarm.cone import (
    AlignmentCone,
    ConeReference,
    ConeRegistryClient,
)
from efe_platform.swarm.dissolution import DecodedOutput, decode_output
from efe_platform.swarm.protocol import (
    ConvergenceThresholds,
    ExecuteRequest,
    ExecuteResponse,
    PopulationEntry,
    ResourceBudget,
    SwarmOutcome,
    SwarmSpec,
)
from efe_platform.swarm.spec_builder import SpecBuilder
from efe_platform.swarm.verdict import Verdict, VerdictReport, analyze_verdict

__all__ = [
    # Client
    "SwarmClient",
    "SwarmEndpoint",
    # Cones
    "AlignmentCone",
    "ConeReference",
    "ConeRegistryClient",
    # Dissolution
    "DecodedOutput",
    "decode_output",
    # Protocol types (neutral shapes)
    "ExecuteRequest",
    "ExecuteResponse",
    "PopulationEntry",
    "ConvergenceThresholds",
    "ResourceBudget",
    "SwarmSpec",
    "SwarmOutcome",
    # Spec builder
    "SpecBuilder",
    # Verdict
    "Verdict",
    "VerdictReport",
    "analyze_verdict",
]

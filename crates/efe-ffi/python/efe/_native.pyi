"""Type stubs for `efe._native` (PyO3 bindings).

Hand-maintained — pin test
``scripts/tests/test_efe_ffi_structure.py::test_stub_pyi_in_sync``
verifies sync with the Rust public API.

Cf. ``docs/EFE_FFI_ARCHITECTURE.md`` § 22.
"""

from __future__ import annotations

from typing import Any, Awaitable, Optional

import numpy as np
import numpy.typing as npt

# ────────────────────────────────────────────────────────────────
# Functions
# ────────────────────────────────────────────────────────────────

def version() -> str: ...

def load_bundle(path: str) -> Bundle: ...

def classify_zero_copy(
    text: str,
    catalog_id: str,
    bundle_path: str,
    catalog_path: str,
    *,
    gpu_enabled: bool = ...,
    seed: Optional[int] = ...,
) -> Verdict: ...

def aclassify_zero_copy(
    text: str,
    catalog_id: str,
    bundle_path: str,
    catalog_path: str,
    *,
    gpu_enabled: bool = ...,
    seed: Optional[int] = ...,
) -> Awaitable[Verdict]: ...

# ────────────────────────────────────────────────────────────────
# Classes
# ────────────────────────────────────────────────────────────────

class Verdict:
    """Result of one classify call.

    Attributes:
        label: Predicted label (e.g. "billing-dispute").
        confidence: float in [0.0, 1.0].
        cone_id: Identifier of the alignment cone used.
        rounds_executed: Number of fusion rounds.
        swarm_id: Unique id of this swarm run.
        catalog_version: Version of the catalog hit.
        sub_scores: List of (label, confidence) pairs for top-N alternatives.
        latency_ms: Wall-clock latency in milliseconds.
    """

    label: str
    confidence: float
    cone_id: str
    rounds_executed: int
    swarm_id: str
    catalog_version: str
    sub_scores: list[tuple[str, float]]
    latency_ms: float

    def __init__(
        self,
        label: str,
        confidence: float,
        cone_id: str,
        rounds_executed: int,
        swarm_id: str,
        catalog_version: str,
        sub_scores: list[tuple[str, float]] = ...,
        latency_ms: float = ...,
    ) -> None: ...

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...
    def to_dict(self) -> dict[str, Any]: ...

class Bundle:
    """Embedder bundle wrapper.

    Loaded via :func:`load_bundle`. Holds a reference-counted
    handle to the underlying Rust bundle so multiple
    :class:`EfeNativeClient` can share it.
    """

    id: str
    version: str
    fingerprint: str
    dim: int

    def __repr__(self) -> str: ...
    def encode(self, text: str) -> npt.NDArray[np.float32]: ...

class EpistemicLatentSpace:
    """View of an Epistemic Latent Space matrix.

    Exposes the latent state of `n_agents` × `latent_dim` as numpy
    arrays. The :func:`as_array` method copies in D25 — DT-F-ELS-1
    (D27) replaces with zero-copy.
    """

    swarm_id: str
    n_agents: int
    latent_dim: int

    def as_array(self) -> npt.NDArray[np.float32]: ...
    def centroid(self) -> npt.NDArray[np.float32]: ...
    def __repr__(self) -> str: ...

class EfeNativeClient:
    """In-process EFE client (PyO3 / no gRPC).

    Drop-in replacement for ``efe_tool.EfeClient`` with a ≥ 5×
    speedup on p99.
    """

    def __init__(
        self,
        bundle: Bundle,
        catalog_path: str,
        *,
        gpu_enabled: bool = ...,
        seed: Optional[int] = ...,
    ) -> None: ...

    def classify(self, text: str) -> Verdict: ...
    def healthcheck(self) -> bool: ...
    def close(self) -> None: ...
    def __enter__(self) -> "EfeNativeClient": ...
    def __exit__(self, *exc: object) -> bool: ...
    def __repr__(self) -> str: ...

class SwarmController:
    """Low-level swarm controller. Most users prefer :class:`EfeNativeClient`."""

    catalog_id: str
    catalog_version: str
    gpu_enabled: bool
    seed: Optional[int]

    def __init__(
        self,
        catalog_path: str,
        *,
        gpu_enabled: bool = ...,
        seed: Optional[int] = ...,
    ) -> None: ...

    def n_labels(self) -> int: ...
    def execute(self, spec: SwarmSpec) -> Verdict: ...
    def __repr__(self) -> str: ...

class SwarmSpec:
    """Swarm execution specification."""

    request_id: str
    catalog_id: str
    seed: Optional[int]
    max_rounds: int

    def __init__(
        self,
        request_id: str,
        catalog_id: str,
        *,
        seed: Optional[int] = ...,
        max_rounds: int = ...,
    ) -> None: ...

    @staticmethod
    def builder() -> SwarmSpecBuilder: ...

    def set_embedding(self, embedding: npt.NDArray[np.float32]) -> None: ...
    def embedding_dim(self) -> int: ...
    def __repr__(self) -> str: ...

class SwarmSpecBuilder:
    """Builder for :class:`SwarmSpec`."""

    def request_id(self, request_id: str) -> SwarmSpecBuilder: ...
    def catalog_id(self, catalog_id: str) -> SwarmSpecBuilder: ...
    def embedding(self, embedding: npt.NDArray[np.float32]) -> SwarmSpecBuilder: ...
    def seed(self, seed: int) -> SwarmSpecBuilder: ...
    def max_rounds(self, max_rounds: int) -> SwarmSpecBuilder: ...
    def build(self) -> SwarmSpec: ...

class AlignmentCone:
    """Alignment cone — set of anchors + max projection distance."""

    id: str
    n_anchors: int
    dim: int
    max_projection_distance: float

    def __init__(
        self,
        id: str,
        anchors: npt.NDArray[np.float32],
        max_projection_distance: float,
    ) -> None: ...

    def __repr__(self) -> str: ...

# ────────────────────────────────────────────────────────────────
# Exception hierarchy
# ────────────────────────────────────────────────────────────────

class EfeError(Exception):
    """Base class for all EFE errors."""

class BundleError(EfeError):
    """Error loading or using a bundle."""

class CatalogError(EfeError):
    """Error loading or resolving a catalog."""

class SwarmError(EfeError):
    """Error during swarm execution."""

class AlignmentBreach(EfeError):
    """The output centroid fell outside the alignment cone."""

class NumericalError(EfeError):
    """NaN/Inf detected during computation."""

class ResourceExhausted(EfeError):
    """OOM, VRAM full, rate-limited, etc."""

class ConfigError(EfeError):
    """Invalid configuration value."""

class TimeoutError(EfeError):
    """Operation timed out."""

class FfiError(EfeError):
    """Internal FFI bindings error."""

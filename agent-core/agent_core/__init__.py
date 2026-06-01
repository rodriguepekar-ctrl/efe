"""EFE M25 agent-core — runs inside the Firecracker microVM.

This package is intentionally small and audited : every module is
reviewed before each release, and every external dependency is pinned
to a digest in pyproject.toml. The image is cosign-signed at CI time.
"""

__version__ = "2.0.0"

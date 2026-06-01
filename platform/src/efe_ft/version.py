"""Single source of truth for the efe-ft package version."""

from __future__ import annotations

VERSION: str = "1.0.0"
__version__: str = VERSION


def version_tuple() -> tuple[int, int, int]:
    parts = VERSION.split(".")
    if len(parts) != 3:
        raise ValueError(f"VERSION must be X.Y.Z semver, got {VERSION!r}")
    return int(parts[0]), int(parts[1]), int(parts[2])

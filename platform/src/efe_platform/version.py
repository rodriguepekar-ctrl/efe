"""Package version — single source of truth.

The value here is read by:
- :mod:`efe_platform.__init__` (public re-export),
- :mod:`efe_platform.api.http_app` (``GET /healthz`` payload),
- :mod:`efe_platform.cli` (``efe-platform --version``),
- tests (snapshot assertions).

Bumps follow semver. Only bump MAJOR if a public type signature in
:mod:`efe_platform` (top-level) changes in a breaking way.
"""

from __future__ import annotations

VERSION: str = "0.1.0"
"""Platform version, exposed to callers and transcripts."""

__version__: str = VERSION
"""PEP 396 alias."""


def version_tuple() -> tuple[int, int, int]:
    """Return ``(major, minor, patch)`` integer triple.

    Raises :class:`ValueError` if :data:`VERSION` is not a
    canonical three-segment semver (pre-release suffix ``-rc.1`` etc.
    is intentionally **not** supported — we only ever ship clean tags).
    """
    parts = VERSION.split(".")
    if len(parts) != 3:
        raise ValueError(f"VERSION must be semver X.Y.Z, got {VERSION!r}")
    return int(parts[0]), int(parts[1]), int(parts[2])

"""Single source of truth for the SDK version.

This file is read by:

* ``efe_tool/__init__.py``  — exposes ``__version__``
* ``pyproject.toml``         — synthesised by setuptools-scm at build time
                                (or hard-pinned for reproducibility)
* ``docs/mkdocs.yml`` mike   — docs version label
* ``release.yml`` workflow   — guard against tag/version drift

Sprint D10 amendment — the SDK and server share the same version
post-v0.1.0. A SDK 0.1.X is **only** compatible with server 0.1.X.

Bumping protocol::

    * PATCH bump (0.1.10 → 0.1.11) : non-breaking additive changes
                                      OR bug fixes.
    * MINOR bump (0.1.X → 0.2.0)  : breaking changes (pre-1.0 era).
    * MAJOR bump (0.X.Y → 1.0.0) : promotion to GA — full SemVer
                                    discipline kicks in.

Pinned by ``scripts/tests/test_d10_public_api_surface_pinned.py``.
"""

from __future__ import annotations

__version__ = "0.1.10"
__version_tuple__ = (0, 1, 10)


def server_compatible_version() -> str:
    """Return the server version this SDK is contract-compatible with.

    Pre-1.0, this is exactly ``__version__``. Post-1.0 we may decouple
    (SDK 1.x.y compatible with server 1.x.*).
    """
    return __version__

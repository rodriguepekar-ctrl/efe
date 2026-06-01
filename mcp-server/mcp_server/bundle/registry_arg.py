"""Shared helper for parsing the `registry` argument used by M18 tools.

Audit fix M18-D47-A4 (LOW) : strip `https://` / `http://` scheme before
splitting host vs repo. Pre-fix, `registry="https://ghcr.io/foo"` was
split into `host="https:"` and `repo="/ghcr.io/foo"` (broken).

Also centralises the `_default_oci_factory` used by all M18 tools so a
single place honours `EFE_OCI_REGISTRY_TOKEN` and `EFE_MCP_INSECURE`
(audit fix M18-D47-A7).
"""
from __future__ import annotations

import os
from typing import Callable
from urllib.parse import urlparse


class InvalidRegistryArg(ValueError):
    """Raised when the registry argument can't be split into host + repo."""


def split_registry(registry: str) -> tuple[str, str, str]:
    """Return (host, repo, full_registry_url_without_scheme).

    Accepts both `https://host/repo` and `host/repo` forms.
    Validates that both host and repo are non-empty.
    """
    if not isinstance(registry, str) or not registry.strip():
        raise InvalidRegistryArg("registry argument is empty")
    raw = registry.strip()
    if "://" in raw:
        parsed = urlparse(raw)
        if not parsed.hostname:
            raise InvalidRegistryArg(
                f"registry {registry!r} has no host"
            )
        port_str = f":{parsed.port}" if parsed.port else ""
        host = f"{parsed.hostname}{port_str}"
        repo = parsed.path.lstrip("/")
    else:
        parts = raw.split("/", 1)
        if len(parts) != 2 or not parts[0] or not parts[1]:
            raise InvalidRegistryArg(
                f"registry {registry!r} must be of the form `host/repo`"
            )
        host, repo = parts
    if not host or not repo:
        raise InvalidRegistryArg(
            f"registry {registry!r} must specify both host and repo"
        )
    return host, repo, f"{host}/{repo}"


def default_oci_factory_factory() -> Callable[[str], "object"]:
    """Audit fix M18-D47-A7 (LOW) : centralised default OCI factory.

    Returns a callable `(registry_url) -> OCIRegistryClient` that honors
    the standard env-vars uniformly across all M18 tools :
      - `EFE_OCI_REGISTRY_TOKEN` : bearer token for private registries
      - `EFE_MCP_INSECURE=true` : disable TLS verification (self-signed)
    """
    # Lazy import to avoid circular `bundle/__init__.py` dance.
    from .oci import OCIRegistryClient, parse_registry_allowlist

    def _factory(registry_url: str):
        token = os.environ.get("EFE_OCI_REGISTRY_TOKEN")
        verify_tls = os.environ.get("EFE_MCP_INSECURE", "false").lower() != "true"
        return OCIRegistryClient(
            registry_url,
            token=token,
            allowlist=parse_registry_allowlist(),
            verify_tls=verify_tls,
        )

    return _factory


# Singleton factory instance — built once, reused by all tools.
default_oci_factory = default_oci_factory_factory()

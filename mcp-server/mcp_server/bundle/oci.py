"""Read-only OCI Distribution Spec v1.1 client (M18 D47).

Pure-Python `httpx` HTTP client. **No subprocess** : no `oras`, no
`docker`, no `cosign`. Anonymous + bearer-token auth supported.

Implements only GET endpoints (read-only by construction) :
  GET /v2/<repo>/manifests/<reference>
  GET /v2/<repo>/blobs/<digest>
  GET /v2/<repo>/referrers/<digest>
  GET /v2/<repo>/tags/list

A strict registry **allowlist** is enforced via env-var
`EFE_OCI_REGISTRY_ALLOWLIST` (TM-M18-D-01 + TM-M18-S-01 mitigation).
Default if unset → no registry access permitted ; the operator must
explicitly opt-in.
"""
from __future__ import annotations

import hashlib
import os
from typing import Any
from urllib.parse import urlparse

import httpx


_OCI_MANIFEST_ACCEPT = (
    "application/vnd.oci.image.manifest.v1+json,"
    "application/vnd.oci.artifact.manifest.v1+json,"
    "application/vnd.docker.distribution.manifest.v2+json"
)


# ─────────────────────────────────────────────────────────────────────
# Errors
# ─────────────────────────────────────────────────────────────────────


class OCIRegistryError(Exception):
    """Base class for OCI registry failures."""

    def __init__(
        self,
        message: str,
        *,
        status_code: int | None = None,
        registry: str | None = None,
    ) -> None:
        super().__init__(message)
        self.status_code = status_code
        self.registry = registry


class OCIRegistryNotInAllowlist(OCIRegistryError):
    """The configured allowlist does not permit this registry URL."""


class OCIAccessForbidden(OCIRegistryError):
    """401 / 403 — auth missing or insufficient."""


class OCIArtifactNotFound(OCIRegistryError):
    """404 — manifest, blob, or referrer not found."""


# ─────────────────────────────────────────────────────────────────────
# Allowlist
# ─────────────────────────────────────────────────────────────────────


def parse_registry_allowlist(value: str | None = None) -> tuple[str, ...]:
    """Parse `EFE_OCI_REGISTRY_ALLOWLIST` into normalised prefixes.

    Empty / unset → returns empty tuple = NO registry access.
    Comma- or semicolon-separated list of `host[/repo]` prefixes.
    Whitespace + trailing slashes stripped.
    """
    raw = value if value is not None else os.environ.get("EFE_OCI_REGISTRY_ALLOWLIST", "")
    out: list[str] = []
    for token in raw.replace(";", ",").split(","):
        s = token.strip().rstrip("/")
        if s:
            out.append(s)
    return tuple(out)


def _normalise_registry_url(url: str) -> str:
    """Strip scheme + trailing slash for allowlist matching."""
    parsed = urlparse(url if "://" in url else f"https://{url}")
    host = parsed.hostname or url
    path = (parsed.path or "").rstrip("/")
    if parsed.port and parsed.port not in (443, 80):
        host = f"{host}:{parsed.port}"
    return f"{host}{path}".lower()


def check_allowlist(registry_url: str, *, allowlist: tuple[str, ...]) -> None:
    """Raise OCIRegistryNotInAllowlist if `registry_url` is not allowed.

    Strict mode : `registry_url` must equal an allowlist entry, OR be
    UNDER an allowlist entry (i.e. allowlist entry is a strict prefix
    of `registry_url`). Used for per-REQUEST checks where we know the
    exact `host/repo` being accessed.
    """
    if not allowlist:
        raise OCIRegistryNotInAllowlist(
            f"OCI registry access denied: EFE_OCI_REGISTRY_ALLOWLIST is empty. "
            f"Set it to a comma-separated list of permitted registries.",
            registry=registry_url,
        )
    norm = _normalise_registry_url(registry_url)
    for prefix in allowlist:
        norm_prefix = _normalise_registry_url(prefix)
        if norm == norm_prefix or norm.startswith(norm_prefix + "/"):
            return
    raise OCIRegistryNotInAllowlist(
        f"OCI registry {registry_url!r} not in EFE_OCI_REGISTRY_ALLOWLIST",
        registry=registry_url,
    )


def check_host_in_allowlist(
    registry_url: str, *, allowlist: tuple[str, ...]
) -> None:
    """Audit fix M18-D47-A1 (HIGH) : looser CONSTRUCTION-time check.

    The OCIRegistryClient is built with a registry URL whose path
    component may be broader than what the allowlist permits. We allow
    construction iff the HOST part of the URL matches the host part
    of any allowlist entry. The strict per-request `check_allowlist`
    still enforces the path-level scope on every actual call.
    """
    if not allowlist:
        raise OCIRegistryNotInAllowlist(
            f"OCI registry access denied: EFE_OCI_REGISTRY_ALLOWLIST is empty. "
            f"Set it to a comma-separated list of permitted registries.",
            registry=registry_url,
        )
    norm = _normalise_registry_url(registry_url)
    norm_host = norm.split("/", 1)[0]
    for prefix in allowlist:
        norm_prefix = _normalise_registry_url(prefix)
        prefix_host = norm_prefix.split("/", 1)[0]
        if norm_host == prefix_host:
            return
    raise OCIRegistryNotInAllowlist(
        f"OCI registry host {norm_host!r} not in EFE_OCI_REGISTRY_ALLOWLIST",
        registry=registry_url,
    )


# ─────────────────────────────────────────────────────────────────────
# Client
# ─────────────────────────────────────────────────────────────────────


class OCIRegistryClient:
    """Anonymous + bearer-token-supporting OCI v2 GET-only client.

    Audit fix M18-D47-A1 (HIGH) : construction now uses
    `check_host_in_allowlist` (loose, host-only). Each request method
    additionally calls `_check_repo_allowed` (strict, full host+repo)
    so a path-prefixed allowlist (e.g. `ghcr.io/efe-project`) actually
    restricts access to repos under that prefix.
    """

    def __init__(
        self,
        registry_url: str,
        *,
        token: str | None = None,
        timeout_seconds: float = 30.0,
        allowlist: tuple[str, ...] | None = None,
        verify_tls: bool = True,
    ) -> None:
        wl = allowlist if allowlist is not None else parse_registry_allowlist()
        # M18-D47-A1 fix : the host part of the registry_url must match
        # SOME allowlist entry's host. Per-request checks then enforce
        # the path scope.
        check_host_in_allowlist(registry_url, allowlist=wl)
        self._allowlist = wl

        # Build httpx base_url from scheme + host:port ONLY. OCI v2
        # paths are absolute from the registry root, so a base_url
        # containing a repo prefix would double-encode the path.
        if "://" not in registry_url:
            full_url = f"https://{registry_url}"
        else:
            full_url = registry_url
        parsed = urlparse(full_url)
        port_str = f":{parsed.port}" if parsed.port else ""
        host_only = f"{parsed.scheme}://{parsed.hostname}{port_str}"
        self._url = host_only
        # Stash the original full URL (without trailing /) for per-
        # request allowlist enforcement.
        self._allowlist_url = full_url.rstrip("/")

        self._headers: dict[str, str] = {
            "User-Agent": "efe-mcp-server/0.3.0",
            "Accept": _OCI_MANIFEST_ACCEPT,
        }
        if token:
            self._headers["Authorization"] = f"Bearer {token}"
        self._client = httpx.AsyncClient(
            base_url=self._url,
            timeout=timeout_seconds,
            verify=verify_tls,
        )

    # ── Per-request allowlist check (M18-D47-A1 fix) ────────────────

    def _check_repo_allowed(self, repo: str) -> None:
        """Strict per-request check : the full <host>/<repo> URL must
        satisfy the allowlist. Catches the case where the construction
        used a host-only URL but the allowlist restricts to a sub-repo.
        """
        full = f"{self._url}/{repo.strip('/')}"
        check_allowlist(full, allowlist=self._allowlist)

    async def aclose(self) -> None:
        await self._client.aclose()

    async def __aenter__(self) -> "OCIRegistryClient":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        await self.aclose()

    # ── Errors mapping ──────────────────────────────────────────────

    def _raise_for_status(self, resp: httpx.Response, *, context: str) -> None:
        if 200 <= resp.status_code < 300:
            return
        if resp.status_code in (401, 403):
            raise OCIAccessForbidden(
                f"{context}: HTTP {resp.status_code}",
                status_code=resp.status_code,
                registry=self._url,
            )
        if resp.status_code == 404:
            raise OCIArtifactNotFound(
                f"{context}: HTTP 404",
                status_code=404,
                registry=self._url,
            )
        raise OCIRegistryError(
            f"{context}: HTTP {resp.status_code}",
            status_code=resp.status_code,
            registry=self._url,
        )

    # ── GET helpers ─────────────────────────────────────────────────

    async def get_manifest(
        self, *, repo: str, reference: str,
    ) -> tuple[bytes, dict[str, Any], str]:
        """Pull an OCI manifest.

        Returns (raw_bytes, parsed_json, manifest_sha256_hex).
        `manifest_sha256` is computed locally over the bytes the
        registry served — this is the value used for signature
        verification (TM-M18-T-01).
        """
        self._check_repo_allowed(repo)
        path = f"/v2/{repo.strip('/')}/manifests/{reference}"
        try:
            resp = await self._client.get(path, headers=self._headers)
        except httpx.HTTPError as exc:
            raise OCIRegistryError(
                f"transport error getting manifest {repo}:{reference}: {exc}",
                registry=self._url,
            ) from exc
        self._raise_for_status(resp, context=f"GET manifest {repo}:{reference}")
        raw = resp.content
        manifest_sha = hashlib.sha256(raw).hexdigest()
        try:
            parsed = resp.json()
        except ValueError:
            parsed = {}
        return raw, parsed, manifest_sha

    async def get_blob(self, *, repo: str, digest: str) -> bytes:
        """Pull a blob by digest (e.g. `sha256:abc...`)."""
        self._check_repo_allowed(repo)
        path = f"/v2/{repo.strip('/')}/blobs/{digest}"
        try:
            resp = await self._client.get(path, headers=self._headers)
        except httpx.HTTPError as exc:
            raise OCIRegistryError(
                f"transport error getting blob {repo}@{digest}: {exc}",
                registry=self._url,
            ) from exc
        self._raise_for_status(resp, context=f"GET blob {repo}@{digest}")
        return resp.content

    async def get_referrers(
        self,
        *,
        repo: str,
        digest: str,
        artifact_type: str | None = None,
    ) -> list[dict[str, Any]]:
        """List referrer manifests pointing to `digest`.

        OCI Distribution v1.1 exposes :
          GET /v2/<repo>/referrers/<digest>[?artifactType=<type>]
        Older registries return 404 — we treat that as "no referrers".
        """
        self._check_repo_allowed(repo)
        path = f"/v2/{repo.strip('/')}/referrers/{digest}"
        params = {"artifactType": artifact_type} if artifact_type else None
        try:
            resp = await self._client.get(
                path, headers=self._headers, params=params,
            )
        except httpx.HTTPError as exc:
            raise OCIRegistryError(
                f"transport error listing referrers for {repo}@{digest}: {exc}",
                registry=self._url,
            ) from exc
        if resp.status_code == 404:
            return []
        self._raise_for_status(resp, context=f"GET referrers {repo}@{digest}")
        try:
            data = resp.json()
        except ValueError:
            return []
        manifests = data.get("manifests") if isinstance(data, dict) else None
        return manifests if isinstance(manifests, list) else []

    async def list_tags(self, *, repo: str, n: int = 100) -> list[str]:
        """List up to `n` tags for the given repository."""
        self._check_repo_allowed(repo)
        path = f"/v2/{repo.strip('/')}/tags/list"
        try:
            resp = await self._client.get(
                path, headers=self._headers, params={"n": n},
            )
        except httpx.HTTPError as exc:
            raise OCIRegistryError(
                f"transport error listing tags for {repo}: {exc}",
                registry=self._url,
            ) from exc
        self._raise_for_status(resp, context=f"GET tags {repo}")
        try:
            data = resp.json()
        except ValueError:
            return []
        tags = data.get("tags") if isinstance(data, dict) else None
        return [t for t in tags if isinstance(t, str)] if isinstance(tags, list) else []

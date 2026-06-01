"""MCP resource handler for `efe://deployments/...` (M18 D49).

Currently implements the `current` sub-resource. `history` is reserved
for D50 (write-mediated through `efe_request_deployment`).
"""
from __future__ import annotations

from typing import Any

from ..auth import AuthContext
from ..backend_client import BackendClient


class DeploymentResourceError(Exception):
    """Raised when a deployment resource cannot be served."""

    def __init__(self, message: str, *, code: str = "not_found") -> None:
        super().__init__(message)
        self.code = code


async def get_current_deployment(
    *,
    auth: AuthContext,
    backend: BackendClient,
) -> dict[str, Any]:
    """Read-only snapshot of the active deployment.

    Backend EFE 1.2 path : `GET /admin/deployments/current`. Falls back
    to `/info` composition for older backends, with a `source` flag.
    """
    try:
        result = await backend.get_deployment_current(auth=auth)
        result.setdefault("source", "backend")
        return result
    except NotImplementedError:
        pass

    info = await backend.get_info(auth=auth)
    if not info:
        raise DeploymentResourceError(
            "backend does not expose /info ; cannot derive deployment status",
            code="backend_unavailable",
        )
    # Audit fix M18-D49-A3 (LOW) : also try the M15-A5 `/admin/bundle-info`
    # endpoint to fill in bundle metadata that `/info.embedder` doesn't
    # expose. Previously the fallback always returned "unknown" even
    # when /admin/bundle-info was available.
    bundle_info: dict[str, Any] = {}
    try:
        bundle_info = await backend.get_admin_bundle_info(auth=auth)
    except NotImplementedError:
        bundle_info = {}
    except Exception:  # noqa: BLE001
        bundle_info = {}
    embedder = info.get("embedder") or {}
    return {
        "source": "fallback_info_compose",
        "deployed_at": None,
        "deployed_by": None,
        "bundle": {
            "slug": (
                bundle_info.get("bundle_slug")
                or embedder.get("bundle_slug")
                or "unknown"
            ),
            "version": (
                bundle_info.get("bundle_id")
                or embedder.get("bundle_id")
                or "unknown"
            ),
            "manifest_sha256": (
                bundle_info.get("manifestSha256")
                or embedder.get("manifestSha256")
            ),
        },
        "helm": {
            "chart_version": info.get("helm_chart_version"),
            "release_name": info.get("release_name"),
            "namespace": info.get("namespace"),
        },
        "replicas": {
            "desired": None, "ready": None, "available": None,
        },
        "health": "unknown",
        "uptime_seconds": None,
        "warnings": [
            "fallback view : deployment fields are partial. Upgrade to "
            "EFE 1.2 for full status.",
        ],
    }

"""FastAPI app factory for efe_registry.

The real import of FastAPI is done lazily so the package remains
importable in environments without it (e.g. test-only). When called,
the factory returns a ready-to-serve app with the routes documented
in M06_ARCHITECTURE.md §16.3.
"""

from __future__ import annotations

from typing import Any

from efe_registry.api.context import ApiContext
from efe_registry.api.schemas import (
    ArtifactView,
    PublishRequestModel,
    RollbackRequestModel,
    RollbackResponseModel,
)
from efe_registry.core.registry import PublishRequest
from efe_registry.errors import RegistryError


def build_app(ctx: ApiContext) -> Any:
    """Return a FastAPI ``app``.

    Exceptions derived from :class:`RegistryError` are translated to
    HTTP responses using each error's ``http_status`` and ``to_dict``.
    """
    try:
        from fastapi import FastAPI, HTTPException, Request
        from fastapi.responses import JSONResponse
    except ImportError as exc:  # pragma: no cover
        raise RuntimeError(
            "FastAPI not installed; cannot build HTTP app",
        ) from exc

    app = FastAPI(
        title="efe-registry",
        version="0.1.0",
        openapi_url="/openapi.json",
    )

    @app.exception_handler(RegistryError)
    async def _registry_error(
        request: Request, exc: RegistryError,
    ) -> Any:
        return JSONResponse(
            status_code=exc.http_status, content=exc.to_dict(),
        )

    @app.get("/healthz")
    async def healthz() -> dict[str, str]:
        return {"status": "ok"}

    # ---- Publish ----
    @app.post("/v1/registry/artifacts", response_model=ArtifactView)
    async def publish(body: PublishRequestModel) -> ArtifactView:
        req = PublishRequest(
            tenant_id=body.tenant_id,
            task_id=body.task_id,
            semver=body.semver,
            content_hash=bytes.fromhex(body.content_hash_hex),
            oci_ref=body.oci_ref,
            s3_url=body.s3_url,
            parent_run_id=body.parent_run_id,
            framework=body.framework,
            size_bytes=body.size_bytes,
            parent_decision_id=body.parent_decision_id,
        )
        art = await ctx.registry.publish(req)
        return ArtifactView(
            artifact_id=art.artifact_id,
            tenant_id=art.tenant_id,
            task_id=art.task_id,
            semver=art.semver,
            content_hash_hex=art.content_hash_hex,
            state=art.state.value,
            state_version=art.state_version,
            created_at=art.created_at,
            promoted_at=art.promoted_at,
            deprecated_at=art.deprecated_at,
        )

    # ---- Get ----
    @app.get(
        "/v1/registry/artifacts/{artifact_id}",
        response_model=ArtifactView,
    )
    async def get_artifact(artifact_id: str) -> ArtifactView:
        art = await ctx.registry.get(artifact_id)
        return ArtifactView(
            artifact_id=art.artifact_id,
            tenant_id=art.tenant_id,
            task_id=art.task_id,
            semver=art.semver,
            content_hash_hex=art.content_hash_hex,
            state=art.state.value,
            state_version=art.state_version,
            created_at=art.created_at,
            promoted_at=art.promoted_at,
            deprecated_at=art.deprecated_at,
        )

    # ---- Rollback ----
    @app.post(
        "/v1/registry/rollbacks",
        response_model=RollbackResponseModel,
    )
    async def rollback(
        body: RollbackRequestModel, request: Request,
    ) -> RollbackResponseModel:
        if ctx.settings.api.require_mfa_rollback and not _has_mfa(request):
            raise HTTPException(
                status_code=401, detail="MFA required for rollback",
            )
        # M6 — tenant authority comes from the gateway-signed header,
        # not from the request body. If the body disagrees with the
        # header we refuse (403 TenantMismatch) rather than silently
        # honour the body and leak into another tenant.
        header_tenant = _header_tenant(request)
        if header_tenant is None:
            raise HTTPException(
                status_code=401,
                detail="X-EFE-Tenant-Id header is required",
            )
        if body.tenant_id != header_tenant:
            raise HTTPException(
                status_code=403,
                detail=(
                    f"body.tenant_id={body.tenant_id!r} does not match "
                    f"X-EFE-Tenant-Id={header_tenant!r}"
                ),
            )
        idem = (
            bytes.fromhex(body.idempotency_key_hex)
            if body.idempotency_key_hex else None
        )
        receipt = await ctx.registry.rollback(
            tenant_id=header_tenant,
            task_id=body.task_id,
            actor=body.actor,
            reason=body.reason,
            trigger_kind=body.trigger_kind,
            idempotency_key=idem,
        )
        return RollbackResponseModel(
            rollback_id=receipt.rollback_id,
            tenant_id=receipt.tenant_id,
            task_id=receipt.task_id,
            from_artifact_id=receipt.from_artifact_id,
            to_artifact_id=receipt.to_artifact_id,
            latency_ms=receipt.latency_ms,
            final_state=receipt.final_state,
            phases=receipt.phases,
        )

    return app


def _has_mfa(request: Any) -> bool:
    # In production the gateway sets X-EFE-MFA-Verified=true after
    # MFA challenge. Tests can inject it directly.
    hdr = getattr(request, "headers", {})
    val = hdr.get("x-efe-mfa-verified") if hasattr(hdr, "get") else None
    return str(val).lower() in ("true", "1", "yes")


def _header_tenant(request: Any) -> str | None:
    hdr = getattr(request, "headers", {})
    if not hasattr(hdr, "get"):
        return None
    val = hdr.get("x-efe-tenant-id")
    if val is None or not str(val).strip():
        return None
    return str(val).strip()


__all__ = ["build_app"]

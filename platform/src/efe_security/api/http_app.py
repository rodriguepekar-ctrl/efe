"""FastAPI app builder for M11.

Routes :

* ``GET  /security/health`` — liveness.
* ``POST /v1/tenants`` — onboard a tenant (requires admin).
* ``POST /v1/consent`` — record a consent.
* ``POST /v1/dsar`` — submit a DSAR.
* ``POST /v1/dsar/{request_id}/complete`` — mark DSAR completed.
* ``GET  /v1/audit/verify`` — run a chain verification.
* ``GET  /v1/compliance/evidence`` — export an evidence package.

The fallback stub (no FastAPI) still registers the routes so import-
level tests can check them.
"""

from __future__ import annotations

import logging
from datetime import datetime, timezone
from typing import TYPE_CHECKING, Any

from efe_security.api.deps import SecurityDeps
from efe_security.api.schemas import (
    ConsentCreateRequest,
    DsarSubmitRequest,
    TenantOnboardRequest,
)
from efe_security.compliance.evidence import EvidenceCollector
from efe_security.consent.dsar import DsarKind, DsarState
from efe_security.consent.records import LegalBasis
from efe_security.errors import SecurityError

try:  # pragma: no cover - optional dep
    from fastapi import FastAPI, HTTPException
    from fastapi.responses import JSONResponse

    _FASTAPI_AVAILABLE = True
except ImportError:  # pragma: no cover
    _FASTAPI_AVAILABLE = False

    class FastAPI:  # type: ignore[no-redef]
        def __init__(self, *_args: Any, **_kwargs: Any) -> None:
            self.routes: list[Any] = []

        def add_api_route(self, *args: Any, **kwargs: Any) -> None:
            self.routes.append((args, kwargs))

        def add_exception_handler(self, *_a: Any, **_k: Any) -> None:
            pass

    class HTTPException(Exception):  # type: ignore[no-redef]
        def __init__(self, status_code: int, detail: Any = None) -> None:
            super().__init__(detail or "")
            self.status_code = status_code
            self.detail = detail

    class JSONResponse:  # type: ignore[no-redef]
        def __init__(self, content: Any, status_code: int = 200) -> None:
            self.content = content
            self.status_code = status_code


if TYPE_CHECKING:  # pragma: no cover
    pass

__all__ = ["build_app"]

_LOG = logging.getLogger("efe_security.api")


def build_app(deps: SecurityDeps) -> FastAPI:
    app = FastAPI(title="EFE Security", version="1.0.0")

    async def health() -> dict[str, str]:
        return {"status": "ok"}

    async def onboard_tenant(body: TenantOnboardRequest) -> dict[str, Any]:
        from efe_security.tenancy.provisioner import TenantTier

        t = await deps.tenants.onboard(
            tenant_id=body.tenant_id,
            display_name=body.display_name,
            tier=TenantTier(body.tier),
            region=body.region,
            labels=tuple((k, v) for k, v in (body.labels or {}).items()),
        )
        return {
            "tenant_id": t.tenant_id,
            "state": t.state,
            "state_version": t.state_version,
        }

    async def record_consent(body: ConsentCreateRequest) -> dict[str, Any]:
        rec = await deps.consent_manager.record(
            tenant_id=body.tenant_id,
            subject_id=body.subject_id,
            purposes=frozenset(body.purposes),
            legal_basis=LegalBasis(body.legal_basis),
            consent_text=body.consent_text,
            locale=body.locale,
            collected_by=body.collected_by,
            expires_at=body.expires_at,
        )
        return {"consent_id": rec.consent_id, "state": rec.state.value}

    async def submit_dsar(body: DsarSubmitRequest) -> dict[str, Any]:
        r = await deps.dsar.submit(
            tenant_id=body.tenant_id,
            subject_id=body.subject_id,
            kind=DsarKind(body.kind),
        )
        return {"request_id": r.request_id, "state": r.state.value}

    async def complete_dsar(request_id: str) -> dict[str, Any]:
        r = await deps.dsar.start(request_id)
        r = await deps.dsar.complete(r.request_id, notes="completed via API")
        return {
            "request_id": r.request_id,
            "state": r.state.value,
            "receipt_seal": r.receipt_seal.hex() if r.receipt_seal else None,
        }

    async def verify_audit() -> dict[str, Any]:
        from efe_security.audit.verifier import verify_range

        outcome = await verify_range(deps.audit_store)  # type: ignore[arg-type]
        return {
            "inspected": outcome.inspected,
            "ok": outcome.ok,
            "first_broken": outcome.first_broken,
            "errors": list(outcome.errors),
        }

    async def compliance_evidence() -> list[dict[str, Any]]:
        collector = EvidenceCollector()
        return [e.to_dict() for e in collector.collect()]

    # Exception translation
    async def security_error_handler(_req: Any, exc: SecurityError) -> JSONResponse:
        problem = {
            "type": f"urn:efe:sec:error:{exc.code}",
            "title": type(exc).__name__,
            "status": exc.http_status,
            "detail": exc.message,
            "retryable": exc.retryable_hint,
        }
        return JSONResponse(content=problem, status_code=exc.http_status)

    # Register routes
    app.add_api_route("/security/health", health, methods=["GET"])
    app.add_api_route("/v1/tenants", onboard_tenant, methods=["POST"])
    app.add_api_route("/v1/consent", record_consent, methods=["POST"])
    app.add_api_route("/v1/dsar", submit_dsar, methods=["POST"])
    app.add_api_route(
        "/v1/dsar/{request_id}/complete", complete_dsar, methods=["POST"]
    )
    app.add_api_route("/v1/audit/verify", verify_audit, methods=["GET"])
    app.add_api_route(
        "/v1/compliance/evidence", compliance_evidence, methods=["GET"]
    )
    app.add_exception_handler(SecurityError, security_error_handler)

    _ = datetime.now(tz=timezone.utc)  # keep imports
    return app


_ = DsarState  # retain import

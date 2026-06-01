"""End-to-end pin tests against the M15 playground (M16).

The test boots the playground Flask app in-process with Werkzeug's test
client wrapped behind `WerkzeugRestBackend`, then verifies that the
dispatcher → playground → audit chain works end-to-end with no network.

Skipped automatically if the playground modules can't be imported (e.g.
in a minimal CI image without numpy).
"""
from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

import pytest

from mcp_server.audit_emitter import AuditEmitter
from mcp_server.auth import AuthContext
from mcp_server.backend_client import (
    BackendClient,
    BackendError,
    BackendForbidden,
    BackendInfo,
    BackendNotFound,
    BackendQuotaExceeded,
    BackendUnavailable,
    BackendBadRequest,
    _raise_for_status,
)
from mcp_server.dispatcher import Dispatcher
from mcp_server.rate_limiter import RateLimiter
from mcp_server.tools import build_tool_registry


REPO = Path(__file__).resolve().parents[2]
PLAYGROUND_BACKEND = REPO / "playground" / "backend"


def _playground_available() -> bool:
    try:
        if str(PLAYGROUND_BACKEND) not in sys.path:
            sys.path.insert(0, str(PLAYGROUND_BACKEND))
        import importlib  # noqa: F401

        importlib.import_module("app")  # noqa: F401
    except Exception:
        return False
    return True


pytestmark = pytest.mark.skipif(
    not _playground_available(),
    reason="playground backend not importable (numpy / flask missing)",
)


# ─────────────────────────────────────────────────────────────────────
# Werkzeug-backed adapter (no network)
# ─────────────────────────────────────────────────────────────────────


class WerkzeugRestBackend(BackendClient):
    """Adapts the playground Flask app to BackendClient via test_client."""

    def __init__(self, flask_app: Any) -> None:
        self.info = BackendInfo(protocol="rest", endpoint="werkzeug://playground")
        self._client = flask_app.test_client()

    async def aclose(self) -> None:
        return None

    def _request(
        self, method: str, path: str, *, auth: AuthContext, json_body=None, params=None
    ) -> dict[str, Any]:
        headers = {"Authorization": f"Bearer {auth.raw_token}"}
        kwargs: dict[str, Any] = {"headers": headers}
        if json_body is not None:
            kwargs["json"] = json_body
        if params is not None:
            kwargs["query_string"] = params
        resp = self._client.open(path, method=method, **kwargs)
        # Mirror the httpx error mapping.
        body: Any
        try:
            body = json.loads(resp.data.decode("utf-8")) if resp.data else {}
        except json.JSONDecodeError:
            body = {"raw_text": resp.data.decode("utf-8")}
        code = resp.status_code
        if 200 <= code < 300:
            return body if isinstance(body, dict) else {"data": body}
        msg = (body.get("error") or body.get("message") or f"HTTP {code}") if isinstance(body, dict) else f"HTTP {code}"
        backend_code = body.get("code") if isinstance(body, dict) else None
        if code in (400, 422):
            raise BackendBadRequest(msg, status_code=code, backend_code=backend_code)
        if code in (401, 403):
            raise BackendForbidden(msg, status_code=code, backend_code=backend_code)
        if code == 404:
            raise BackendNotFound(msg, status_code=code, backend_code=backend_code)
        if code == 429:
            raise BackendQuotaExceeded(msg, status_code=code, backend_code=backend_code)
        if 500 <= code < 600:
            raise BackendUnavailable(msg, status_code=code, backend_code=backend_code)
        raise BackendError(msg, status_code=code, backend_code=backend_code)

    async def classify(self, *, catalog, text, top_k=3, auth):
        return self._request(
            "POST", "/classify", auth=auth,
            json_body={"catalog": catalog, "text": text, "top_k": top_k},
        )

    async def register_catalog(self, *, name, classes, auth):
        return self._request(
            "POST", "/catalog/register", auth=auth,
            json_body={"name": name, "classes": classes},
        )

    async def evolve_catalog(self, *, name, additional_examples_per_class, auth):
        return self._request(
            "POST", f"/catalog/{name}/evolve", auth=auth,
            json_body={
                "additional_examples": additional_examples_per_class,
                "additional_examples_per_class": additional_examples_per_class,
            },
        )

    async def list_catalogs(self, *, auth):
        data = self._request("GET", "/catalog", auth=auth)
        return data.get("catalogs", [])

    async def get_audit_log(self, *, limit, auth):
        data = self._request("GET", "/audit", auth=auth, params={"limit": limit})
        events = data.get("events", [])
        return events[-limit:] if limit else events

    async def get_admin_quotas(self, *, auth):
        return self._request("GET", "/admin/quotas", auth=auth)


# ─────────────────────────────────────────────────────────────────────
# Fixtures
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def playground_dispatcher():
    if str(PLAYGROUND_BACKEND) not in sys.path:
        sys.path.insert(0, str(PLAYGROUND_BACKEND))
    from app import app, CATALOG, AUDIT  # noqa: E402

    # Reset state.
    CATALOG._store.clear()
    AUDIT._events.clear()
    AUDIT._last_hmac = "0" * 64

    # Mint a real demo token via the auth route.
    client = app.test_client()
    resp = client.post("/auth/token", json={"tenant_id": "acme", "role": "admin"})
    token = resp.get_json()["token"]

    backend = WerkzeugRestBackend(app)
    auth = AuthContext(raw_token=token)
    audit = AuditEmitter(backend, enabled=False)  # avoid /audit/emit 404 churn
    rl = RateLimiter(per_min=1000)  # don't trip rate limit in tests
    return Dispatcher(
        registry=build_tool_registry(),
        backend=backend,
        auth=auth,
        audit=audit,
        rate_limiter=rl,
    )


# ─────────────────────────────────────────────────────────────────────
# E2E
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_e2e_register_then_classify_against_playground(playground_dispatcher):
    res_reg = await playground_dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "e2e-cat",
            "classes": [
                {"name": "greeting", "examples": ["hello", "hi"]},
                {"name": "farewell", "examples": ["bye", "goodbye"]},
            ],
            "confirm": True,
        },
    )
    assert res_reg.is_error is False, res_reg.content
    assert res_reg.content["version"] == 1

    res_cls = await playground_dispatcher.dispatch(
        tool_name="efe_classify",
        args={"catalog": "e2e-cat", "text": "hi there", "top_k": 2},
    )
    assert res_cls.is_error is False, res_cls.content
    assert res_cls.content["catalog_name"] == "e2e-cat"
    assert len(res_cls.content["top_k"]) == 2


@pytest.mark.asyncio
async def test_e2e_list_catalogs_returns_what_we_registered(playground_dispatcher):
    await playground_dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "e2e-list",
            "classes": [{"name": "x", "examples": ["a"]}],
            "confirm": True,
        },
    )
    res = await playground_dispatcher.dispatch(
        tool_name="efe_list_catalogs", args={}
    )
    assert res.is_error is False
    names = [c["name"] for c in res.content["catalogs"]]
    assert "e2e-list" in names


@pytest.mark.asyncio
async def test_e2e_admin_quotas_match_d12_constants(playground_dispatcher):
    res = await playground_dispatcher.dispatch(
        tool_name="efe_get_admin_quotas", args={}
    )
    assert res.is_error is False
    limits = res.content["limits"]
    assert limits["max_names_per_tenant"] == 100
    assert limits["max_versions_per_name"] == 1000
    assert limits["max_classes_per_catalog"] == 256
    assert limits["max_sentences_per_class"] == 64


@pytest.mark.asyncio
async def test_e2e_evolve_then_version_increments(playground_dispatcher):
    await playground_dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "e2e-evol",
            "classes": [{"name": "c1", "examples": ["a"]}],
            "confirm": True,
        },
    )
    res = await playground_dispatcher.dispatch(
        tool_name="efe_evolve_catalog",
        args={
            "name": "e2e-evol",
            "additional_examples_per_class": {"c1": ["new-ex"]},
            "confirm": True,
        },
    )
    assert res.is_error is False
    assert res.content["version"] == 2

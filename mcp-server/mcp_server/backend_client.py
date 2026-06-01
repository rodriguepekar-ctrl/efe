"""Backend client abstraction (M16).

The MCP server can talk to either :
  - **REST** — the M15 playground (or any HTTP/JSON EFE deployment) ;
  - **gRPC** — production EFE 1.0 GA cluster (default for `https://` with
    ALPN h2 advertised, or when `EFE_BACKEND_PROTOCOL=grpc` is set) ;
  - **Unix socket** — local sidecar pattern (M19 future).

The backend protocol is **auto-detected** from `EFE_ENDPOINT` :
  - `http://…` or `https://…` → REST (HTTPX) ;
  - `unix:///…`               → Unix socket (HTTPX over UDS) ;
  - `grpc://…` or `grpcs://…` → gRPC (only if `grpcio` extra is installed).

A single abstract interface (`BackendClient`) is exposed so tool handlers
don't care about the wire protocol. Backend selection is decided once at
startup.
"""
from __future__ import annotations

import abc
import os
from dataclasses import dataclass
from typing import Any, Optional
from urllib.parse import urlparse

import httpx

from .auth import AuthContext, authorization_header


# ─────────────────────────────────────────────────────────────────────
# Errors
# ─────────────────────────────────────────────────────────────────────


class BackendError(Exception):
    """Wraps any error from the EFE backend, translated to MCP isError."""

    def __init__(
        self,
        message: str,
        *,
        status_code: Optional[int] = None,
        backend_code: Optional[str] = None,
        retryable: bool = False,
    ) -> None:
        super().__init__(message)
        self.status_code = status_code
        self.backend_code = backend_code
        self.retryable = retryable

    def to_dict(self) -> dict[str, Any]:
        return {
            "message": str(self),
            "status_code": self.status_code,
            "backend_code": self.backend_code,
            "retryable": self.retryable,
        }


class BackendUnavailable(BackendError):
    """5xx, network error, timeout — retryable by default."""

    def __init__(self, message: str, **kw: Any) -> None:
        kw.setdefault("retryable", True)
        super().__init__(message, **kw)


class BackendForbidden(BackendError):
    """403 — caller authenticated but tenant does not have access."""


class BackendNotFound(BackendError):
    """404 — catalog or version does not exist for caller's tenant."""


class BackendQuotaExceeded(BackendError):
    """429 — D12 catalog quota or M11 rate limit hit."""

    def __init__(self, message: str, **kw: Any) -> None:
        kw.setdefault("retryable", True)
        super().__init__(message, **kw)


class BackendBadRequest(BackendError):
    """400 — caller supplied invalid arguments."""


# ─────────────────────────────────────────────────────────────────────
# Abstract interface
# ─────────────────────────────────────────────────────────────────────


@dataclass
class BackendInfo:
    protocol: str   # 'rest' | 'grpc' | 'unix'
    endpoint: str   # human-readable, e.g. 'https://efe.example.com'
    version: Optional[str] = None


class BackendClient(abc.ABC):
    """Abstract client. Tool handlers depend only on this interface."""

    info: BackendInfo

    @abc.abstractmethod
    async def classify(
        self, *, catalog: str, text: str, top_k: int = 3, auth: AuthContext
    ) -> dict[str, Any]: ...

    @abc.abstractmethod
    async def register_catalog(
        self, *, name: str, classes: list[dict[str, Any]], auth: AuthContext
    ) -> dict[str, Any]: ...

    @abc.abstractmethod
    async def evolve_catalog(
        self,
        *,
        name: str,
        additional_examples_per_class: dict[str, list[str]],
        auth: AuthContext,
    ) -> dict[str, Any]: ...

    @abc.abstractmethod
    async def list_catalogs(self, *, auth: AuthContext) -> list[dict[str, Any]]: ...

    @abc.abstractmethod
    async def get_audit_log(
        self, *, limit: int, auth: AuthContext
    ) -> list[dict[str, Any]]: ...

    @abc.abstractmethod
    async def get_admin_quotas(self, *, auth: AuthContext) -> dict[str, Any]: ...

    @abc.abstractmethod
    async def aclose(self) -> None: ...

    # ─── M17 extensions (default implementations compose M16 + EFE 1.1) ──

    async def encode_batch(
        self, *, texts: list[str], auth: AuthContext
    ) -> list[list[float]]:
        """Embed a list of texts. Default fallback : per-item /encode."""
        out: list[list[float]] = []
        for t in texts:
            r = await self._encode_single(text=t, auth=auth)
            out.append(r["embedding"])
        return out

    async def _encode_single(
        self, *, text: str, auth: AuthContext
    ) -> dict[str, Any]:
        """Single-text /encode. Concrete clients override."""
        raise NotImplementedError("encode is required for M17 advanced+ profile")

    async def explain_classification(
        self, *, catalog: str, text: str, predicted_class: str | None, auth: AuthContext,
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def compare_classes(
        self, *, catalog: str, class_a: str, class_b: str, auth: AuthContext
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def score_distribution(
        self, *, catalog: str, class_name: str, sample_count: int,
        texts: list[str] | None, include_negatives: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def find_similar_examples(
        self, *, catalog: str, text: str, top_k: int, scope: str, auth: AuthContext,
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def diagnose_catalog(
        self, *, catalog: str, depth: str, include_examples_text: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def detect_drift(
        self, *, catalog: str, version_a: int | None, version_b: int | None, auth: AuthContext,
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def audit_health_check(
        self, *, from_timestamp: float | None, to_timestamp: float | None,
        deep: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def export_catalog(
        self, *, catalog: str, version: int | None, format: str,
        include_centroids: bool, include_audit_excerpt: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def consolidate_versions(
        self, *, catalog: str, keep_last_n: int, dry_run: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def list_catalog_classes(
        self, *, catalog: str, auth: AuthContext,
    ) -> list[dict[str, Any]]:
        raise NotImplementedError

    async def get_class_examples(
        self, *, catalog: str, class_name: str, auth: AuthContext,
    ) -> dict[str, Any]:
        raise NotImplementedError

    async def get_info(self, *, auth: AuthContext) -> dict[str, Any]:
        raise NotImplementedError

    async def get_admin_bundle_info(self, *, auth: AuthContext) -> dict[str, Any]:
        """Audit fix M18-D49-A6 (LOW) : public method for the M15-A5
        `/admin/bundle-info` endpoint, replacing the previous direct
        `_request()` call that broke the abstract interface.

        Default raises NotImplementedError so unsupported backends
        surface cleanly. The REST implementation maps it to
        `GET /admin/bundle-info` ; gRPC backends (M19) may override.
        """
        raise NotImplementedError

    async def bulk_classify(
        self,
        *,
        catalog: str,
        texts: list[str],
        top_k: int,
        include_explanations: bool,
        auth: AuthContext,
        progress: Any = None,  # noqa: ARG002 — passed by tools/bulk_classify
    ) -> dict[str, Any]:
        """Audit fix M17-A8 (LOW) : declared on the abstract class so
        every concrete client documents whether it supports the bulk
        endpoint. Default raises NotImplementedError ; the bulk_classify
        tool catches that and falls back to per-item /classify."""
        raise NotImplementedError(
            "bulk_classify not supported by this backend ; "
            "the tool will fall back to per-item classify"
        )

    # ─── M18 D47 : bundle introspection ─────────────────────────────

    async def list_known_bundles(
        self,
        *,
        registry_filter: str | None,
        slug_filter: str | None,
        limit: int,
        auth: AuthContext,
    ) -> dict[str, Any]:
        """List bundles registered with the EFE backend (1.2 contract).

        Default impl raises NotImplementedError ; concrete clients may
        compose this from `/admin/bundles` (1.2) or fall back to direct
        OCI registry listing.
        """
        raise NotImplementedError("list_known_bundles requires EFE 1.2 backend")

    async def get_bundle_record(
        self, *, slug: str, version: str, auth: AuthContext,
    ) -> dict[str, Any]:
        """Return a bundle record stored by the backend (registered_at,
        last_verified_at, manifest_sha256, etc.). Optional ; if the
        backend doesn't track this, the tool falls back to direct OCI."""
        raise NotImplementedError("get_bundle_record requires EFE 1.2 backend")

    async def get_trust_store(self, *, auth: AuthContext) -> dict[str, Any]:
        """Return the public-keys trust store maintained by the backend.

        Default impl raises ; concrete clients may proxy
        `GET /admin/trust-store` (1.2). The `EFE_OCI_PUBLIC_KEY*`
        env vars are the primary sources ; this is a fallback.
        """
        raise NotImplementedError("get_trust_store requires EFE 1.2 backend")

    # ─── M18 D49 : deployment helpers (EFE 1.2 contract) ────────────

    async def get_deployment_current(self, *, auth: AuthContext) -> dict[str, Any]:
        """Return the active deployment metadata.

        EFE 1.2 endpoint `GET /admin/deployments/current`. Default
        raises ; tools fall back to local composition based on
        `/info` (M15-A5 carry-over).
        """
        raise NotImplementedError(
            "get_deployment_current requires EFE 1.2 backend"
        )

    async def get_deployment_history(
        self, *, limit: int, auth: AuthContext,
    ) -> list[dict[str, Any]]:
        """Return the last N deployments (newest first).

        EFE 1.2 endpoint `GET /admin/deployments/history?limit=N`.
        Default raises ; there is no good local fallback (the data
        does not exist anywhere else), so the tool surfaces
        `not_supported` to the agent.
        """
        raise NotImplementedError(
            "get_deployment_history requires EFE 1.2 backend"
        )

    async def dry_run_deployment(
        self,
        *,
        target_slug: str,
        target_version: str,
        target_manifest_sha256: str | None,
        environment: str,
        auth: AuthContext,
    ) -> dict[str, Any]:
        """Backend-computed dry-run diff (EFE 1.2 endpoint).

        EFE 1.2 `POST /admin/deployments/dry-run`. Default raises ;
        the tool then composes the diff locally using
        `dry_run.compute_diff()`.
        """
        raise NotImplementedError(
            "dry_run_deployment requires EFE 1.2 backend"
        )

    # ─── M18 D50 : deploy write tools (EFE 1.2 contract) ────────────

    async def register_bundle(
        self,
        *,
        slug: str,
        version: str,
        registry_url: str,
        manifest_sha256: str | None,
        verify_before_register: bool,
        idempotency_key: str | None,
        auth: AuthContext,
    ) -> dict[str, Any]:
        """Register a bundle with the backend's known set.

        EFE 1.2 endpoint `POST /admin/bundles/register`. The bundle is
        NOT downloaded ; the backend just stores the (slug, version,
        registry_url, manifest_sha256) tuple in its catalog so that
        future deploy/promote operations can reference it.

        No fallback : write operations MUST be backend-mediated.
        """
        raise NotImplementedError(
            "register_bundle requires EFE 1.2 backend "
            "(POST /admin/bundles/register not exposed)"
        )

    async def promote_bundle(
        self,
        *,
        slug: str,
        version: str,
        scope: str,
        idempotency_key: str | None,
        auth: AuthContext,
    ) -> dict[str, Any]:
        """Mark a registered bundle as 'current' for the given scope.

        EFE 1.2 endpoint `POST /admin/bundles/<slug>/<version>/promote`.
        This is a metadata-only operation : the bundle is NOT deployed
        to the cluster ; the operator must still run `helm upgrade`.

        No fallback : write operations MUST be backend-mediated.
        """
        raise NotImplementedError(
            "promote_bundle requires EFE 1.2 backend "
            "(POST /admin/bundles/<slug>/<version>/promote not exposed)"
        )

    async def request_deployment_ticket(
        self,
        *,
        slug: str,
        version: str,
        environment: str,
        rationale: str,
        tracker: str,
        idempotency_key: str | None,
        auth: AuthContext,
    ) -> dict[str, Any]:
        """File a deployment-request ticket with the chosen tracker.

        EFE 1.2 endpoint `POST /admin/tickets`. The backend wraps the
        third-party tracker API (Linear/Jira/GitHub Issues) under the
        organisation's existing credentials ; the MCP server itself
        never holds tracker secrets.

        No fallback : write operations MUST be backend-mediated.
        """
        raise NotImplementedError(
            "request_deployment_ticket requires EFE 1.2 backend "
            "(POST /admin/tickets not exposed)"
        )


# ─────────────────────────────────────────────────────────────────────
# REST implementation
# ─────────────────────────────────────────────────────────────────────


class RestBackendClient(BackendClient):
    """Talks to the M15 playground REST API (or any EFE REST gateway).

    Endpoints expected (mirror playground/backend/app.py) :
      POST /classify
      POST /catalog/register
      POST /catalog/<name>/evolve
      GET  /catalog
      GET  /audit
      GET  /admin/quotas
    """

    def __init__(
        self,
        endpoint: str,
        *,
        timeout_seconds: float = 30.0,
        verify_tls: bool = True,
    ) -> None:
        self._endpoint = endpoint.rstrip("/")
        self.info = BackendInfo(protocol="rest", endpoint=self._endpoint)
        self._client = httpx.AsyncClient(
            base_url=self._endpoint,
            timeout=timeout_seconds,
            verify=verify_tls,
            headers={"User-Agent": "efe-mcp-server/0.1.0"},
        )

    async def aclose(self) -> None:
        await self._client.aclose()

    async def _request(
        self,
        method: str,
        path: str,
        *,
        auth: AuthContext,
        json_body: Optional[dict[str, Any]] = None,
        params: Optional[dict[str, Any]] = None,
        idempotency_key: Optional[str] = None,
    ) -> dict[str, Any]:
        """Audit fix M18-FINAL-A4 (MED) : `idempotency_key` flows via
        the `Idempotency-Key` header (Stripe / Square / EFE M11 cache
        convention), NOT in the JSON body. Pre-fix, D50 write tools
        passed the key in the body where the M11 cache wouldn't see
        it → idempotency replay broken silently.
        """
        headers = dict(authorization_header(auth))
        if idempotency_key:
            headers["Idempotency-Key"] = idempotency_key
        try:
            resp = await self._client.request(
                method,
                path,
                headers=headers,
                json=json_body,
                params=params,
            )
        except httpx.TimeoutException as exc:
            raise BackendUnavailable(f"timeout calling {method} {path}") from exc
        except httpx.HTTPError as exc:
            raise BackendUnavailable(f"transport error calling {method} {path}: {exc}") from exc

        return _raise_for_status(resp)

    async def classify(
        self, *, catalog: str, text: str, top_k: int = 3, auth: AuthContext
    ) -> dict[str, Any]:
        return await self._request(
            "POST",
            "/classify",
            auth=auth,
            json_body={"catalog": catalog, "text": text, "top_k": top_k},
        )

    async def register_catalog(
        self, *, name: str, classes: list[dict[str, Any]], auth: AuthContext
    ) -> dict[str, Any]:
        return await self._request(
            "POST",
            "/catalog/register",
            auth=auth,
            json_body={"name": name, "classes": classes},
        )

    async def evolve_catalog(
        self,
        *,
        name: str,
        additional_examples_per_class: dict[str, list[str]],
        auth: AuthContext,
    ) -> dict[str, Any]:
        return await self._request(
            "POST",
            f"/catalog/{name}/evolve",
            auth=auth,
            # Playground field name is `additional_examples` ; production gRPC
            # uses `additional_examples_per_class`. We send both for forward
            # compatibility — the playground ignores the unused one.
            json_body={
                "additional_examples": additional_examples_per_class,
                "additional_examples_per_class": additional_examples_per_class,
            },
        )

    async def list_catalogs(self, *, auth: AuthContext) -> list[dict[str, Any]]:
        data = await self._request("GET", "/catalog", auth=auth)
        return data.get("catalogs", [])

    async def get_audit_log(
        self, *, limit: int, auth: AuthContext
    ) -> list[dict[str, Any]]:
        data = await self._request(
            "GET", "/audit", auth=auth, params={"limit": limit}
        )
        events = data.get("events", [])
        if limit and len(events) > limit:
            events = events[-limit:]
        return events

    async def get_admin_quotas(self, *, auth: AuthContext) -> dict[str, Any]:
        return await self._request("GET", "/admin/quotas", auth=auth)

    # ─── M17 implementations ────────────────────────────────────────

    async def _encode_single(self, *, text: str, auth: AuthContext) -> dict[str, Any]:
        return await self._request(
            "POST", "/encode", auth=auth, json_body={"text": text},
        )

    async def encode_batch(
        self, *, texts: list[str], auth: AuthContext
    ) -> list[list[float]]:
        # Try the EFE 1.1 batch endpoint first
        try:
            r = await self._request(
                "POST", "/encode/batch", auth=auth, json_body={"texts": texts},
            )
            return r.get("embeddings", [])
        except (BackendNotFound, BackendBadRequest):
            pass
        # Fallback : per-text /encode
        out: list[list[float]] = []
        for t in texts:
            r = await self._encode_single(text=t, auth=auth)
            out.append(r.get("embedding", []))
        return out

    async def explain_classification(
        self, *, catalog: str, text: str, predicted_class: str | None, auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", f"/catalog/{catalog}/explain", auth=auth,
                json_body={
                    "text": text,
                    **({"predicted_class": predicted_class} if predicted_class else {}),
                },
            )
        except BackendNotFound:
            # Fallback : compose via classify(top_k=2)
            return await self._explain_via_classify_fallback(
                catalog=catalog, text=text,
                predicted_class=predicted_class, auth=auth,
            )

    async def _explain_via_classify_fallback(
        self, *, catalog: str, text: str,
        predicted_class: str | None, auth: AuthContext,
    ) -> dict[str, Any]:
        cls_resp = await self.classify(catalog=catalog, text=text, top_k=32, auth=auth)
        top_k = cls_resp.get("top_k", [])
        if not top_k:
            raise BackendNotFound(f"catalog {catalog!r} returned no scores")
        chosen = top_k[0]
        if predicted_class is not None:
            # Audit fix M17-A7 (LOW) : never silently fallback to top-1
            # when the user named a specific class — surface the gap so
            # the user knows their input wasn't honored. Previous behavior
            # silently used top-1, returning an explanation for a
            # different class than requested.
            match = next(
                (s for s in top_k if s["class_name"] == predicted_class), None,
            )
            if match is None:
                raise BackendBadRequest(
                    f"predicted_class {predicted_class!r} not found in "
                    f"catalog {catalog!r}",
                    status_code=400,
                    backend_code="invalid_predicted_class",
                )
            chosen = match
        runner = next(
            (s for s in top_k if s["class_name"] != chosen["class_name"]),
            None,
        )
        gap = chosen["score"] - (runner["score"] if runner else 0.0)
        return {
            "predicted_class": chosen["class_name"],
            "score": chosen["score"],
            "above_threshold": chosen.get("above_threshold", False),
            "threshold": chosen.get("confidence_threshold", 0.5),
            "runner_up": (
                {
                    "class_name": runner["class_name"],
                    "score": runner["score"],
                    "gap": round(gap, 4),
                } if runner else None
            ),
            "contributors": [],   # not available in fallback mode
            "ambiguity_signal": {
                "is_ambiguous": gap < 0.05,
                "reason": (
                    f"gap_to_runner_up={gap:.4f}"
                    + (" is below the 0.05 ambiguity threshold" if gap < 0.05
                       else " is above the 0.05 ambiguity threshold")
                ),
            },
            "threshold_sensitivity": {
                "decision_unchanged_in_range": [
                    round(runner["score"] if runner else 0.0, 4),
                    round(chosen["score"], 4),
                ],
                "decision_changes_at": None,
            },
            "fallback": "minimal",
        }

    async def compare_classes(
        self, *, catalog: str, class_a: str, class_b: str, auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", f"/catalog/{catalog}/compare", auth=auth,
                json_body={"class_a": class_a, "class_b": class_b},
            )
        except BackendNotFound:
            return await self._compare_via_local_fallback(
                catalog=catalog, class_a=class_a, class_b=class_b, auth=auth,
            )

    async def _compare_via_local_fallback(
        self, *, catalog: str, class_a: str, class_b: str, auth: AuthContext,
    ) -> dict[str, Any]:
        from .analysis.reasoning import compare_centroids
        import numpy as np

        ca_info = await self.get_class_examples(catalog=catalog, class_name=class_a, auth=auth)
        cb_info = await self.get_class_examples(catalog=catalog, class_name=class_b, auth=auth)
        ca_arr = np.asarray(ca_info["centroid"], dtype=np.float32)
        cb_arr = np.asarray(cb_info["centroid"], dtype=np.float32)
        # We may not have example embeddings in fallback ; pass None
        result = compare_centroids(
            class_a_name=class_a,
            class_a_centroid=ca_arr,
            class_a_examples=ca_info.get("examples", []),
            class_a_example_embeddings=None,
            class_b_name=class_b,
            class_b_centroid=cb_arr,
            class_b_examples=cb_info.get("examples", []),
            class_b_example_embeddings=None,
        )
        return {
            "catalog": catalog,
            "class_a": class_a,
            "class_b": class_b,
            "centroid_cosine": round(result.centroid_cosine, 4),
            "centroid_distance_l2": round(result.centroid_distance_l2, 4),
            "separation_score": round(result.separation_score, 4),
            "ambiguity_band": result.ambiguity_band,
            "boundary_examples": result.boundary_examples,
            "recommended_action": (
                "review_examples" if result.ambiguity_band in ("high", "critical")
                else "no_action_needed"
            ),
            "fallback": "local_compute",
        }

    async def score_distribution(
        self, *, catalog: str, class_name: str, sample_count: int,
        texts: list[str] | None, include_negatives: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", f"/catalog/{catalog}/score-distribution", auth=auth,
                json_body={
                    "class_name": class_name,
                    "sample_count": sample_count,
                    **({"texts": texts} if texts else {}),
                    "include_negatives": include_negatives,
                },
            )
        except BackendNotFound:
            return await self._score_distribution_fallback(
                catalog=catalog, class_name=class_name, sample_count=sample_count,
                texts=texts, include_negatives=include_negatives, auth=auth,
            )

    async def _score_distribution_fallback(
        self, *, catalog: str, class_name: str, sample_count: int,
        texts: list[str] | None, include_negatives: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        from .analysis.stats import histogram, percentiles, recommend_threshold_f1

        # Build positive + negative text pools
        if texts:
            pos_texts, neg_texts = texts, []
        else:
            cat = await self._fetch_catalog_classes(catalog=catalog, auth=auth)
            pos_class = next((c for c in cat if c["name"] == class_name), None)
            if not pos_class:
                raise BackendNotFound(f"class {class_name!r} not found")
            pos_texts = (pos_class.get("examples") or [])[:max(1, sample_count // 2)]
            neg_texts = []
            if include_negatives:
                for c in cat:
                    if c["name"] == class_name:
                        continue
                    neg_texts.extend(c.get("examples", []))
                neg_texts = neg_texts[: max(1, sample_count // 2)]

        # Score each text via classify ; pick this class' score
        async def _score(t: str) -> float:
            r = await self.classify(catalog=catalog, text=t, top_k=32, auth=auth)
            for s in r.get("top_k", []):
                if s["class_name"] == class_name:
                    return float(s["score"])
            return 0.0

        pos_scores: list[float] = [await _score(t) for t in pos_texts] if pos_texts else []
        neg_scores: list[float] = [await _score(t) for t in neg_texts] if neg_texts else []

        hist = histogram(positive_scores=pos_scores, negative_scores=neg_scores)
        rec = recommend_threshold_f1(
            positive_scores=pos_scores, negative_scores=neg_scores,
        )
        return {
            "catalog": catalog,
            "class_name": class_name,
            "histogram": {
                "buckets": hist.buckets,
                "positives": hist.positives,
                "negatives": hist.negatives,
            },
            "percentiles_positives": percentiles(pos_scores),
            "percentiles_negatives": percentiles(neg_scores) if neg_scores else {},
            "recommended_threshold": {
                "value": rec.threshold,
                "expected_precision": rec.expected_precision,
                "expected_recall": rec.expected_recall,
                "f1": rec.f1,
            },
            "current_threshold": 0.5,
            "drift_from_recommended": round(rec.threshold - 0.5, 4),
            "positive_scores": pos_scores,
            "negative_scores": neg_scores,
            "fallback": "local_compute",
        }

    async def _fetch_catalog_classes(
        self, *, catalog: str, auth: AuthContext,
    ) -> list[dict[str, Any]]:
        # First check for full version endpoint (M17 EFE 1.1 surfaces it)
        try:
            data = await self._request(
                "GET", f"/catalog/{catalog}", auth=auth,
            )
            return data.get("classes", [])
        except BackendNotFound:
            pass
        # M16 playground style : get the latest version explicitly
        cat_list = await self.list_catalogs(auth=auth)
        match = next((c for c in cat_list if c.get("name") == catalog), None)
        if not match:
            raise BackendNotFound(f"catalog {catalog!r} not found")
        v = match.get("latest_version")
        if v is None:
            raise BackendNotFound(f"catalog {catalog!r} has no version yet")
        data = await self._request(
            "GET", f"/catalog/{catalog}/versions/{v}", auth=auth,
        )
        return data.get("classes", [])

    async def find_similar_examples(
        self, *, catalog: str, text: str, top_k: int, scope: str, auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", f"/catalog/{catalog}/find-similar", auth=auth,
                json_body={"text": text, "top_k": top_k, "scope": scope},
            )
        except BackendNotFound:
            return await self._find_similar_fallback(
                catalog=catalog, text=text, top_k=top_k, scope=scope, auth=auth,
            )

    async def _find_similar_fallback(
        self, *, catalog: str, text: str, top_k: int, scope: str, auth: AuthContext,
    ) -> dict[str, Any]:
        import numpy as np

        cat_classes = await self._fetch_catalog_classes(catalog=catalog, auth=auth)
        encoded = await self._encode_single(text=text, auth=auth)
        emb = np.asarray(encoded["embedding"], dtype=np.float32)
        norm = float(np.linalg.norm(emb)) or 1.0
        emb_unit = emb / norm

        if scope == "top_class":
            cls_resp = await self.classify(
                catalog=catalog, text=text, top_k=1, auth=auth,
            )
            top_cls_name = cls_resp["top_k"][0]["class_name"]
            cat_classes = [c for c in cat_classes if c["name"] == top_cls_name]

        scored: list[dict[str, Any]] = []
        for cls in cat_classes:
            for ex in cls.get("examples", []):
                ex_enc = await self._encode_single(text=ex, auth=auth)
                ex_arr = np.asarray(ex_enc["embedding"], dtype=np.float32)
                ex_norm = float(np.linalg.norm(ex_arr)) or 1.0
                sim = float(np.dot(emb_unit, ex_arr / ex_norm))
                scored.append({
                    "example": ex, "class_name": cls["name"], "similarity": round(sim, 4),
                })
        scored.sort(key=lambda x: x["similarity"], reverse=True)
        return {
            "results": scored[:top_k],
            "scope": scope,
            "embedding_norm": norm,
            "fallback": "local_compute",
        }

    async def diagnose_catalog(
        self, *, catalog: str, depth: str, include_examples_text: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", f"/catalog/{catalog}/diagnose", auth=auth,
                json_body={"depth": depth, "include_examples_text": include_examples_text},
            )
        except BackendNotFound:
            return await self._diagnose_fallback(
                catalog=catalog, depth=depth,
                include_examples_text=include_examples_text, auth=auth,
            )

    async def _diagnose_fallback(
        self, *, catalog: str, depth: str, include_examples_text: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        cat_classes = await self._fetch_catalog_classes(catalog=catalog, auth=auth)
        cat_list = await self.list_catalogs(auth=auth)
        match = next((c for c in cat_list if c.get("name") == catalog), {})
        version = match.get("latest_version")
        fingerprint = match.get("fingerprint")

        sizes = [len(c.get("examples", [])) for c in cat_classes]
        coverage = (
            sum(1 for s in sizes if s >= 5) / max(1, len(sizes))
            if sizes else 0.0
        )
        balance = 0.0
        if sizes:
            mean = sum(sizes) / len(sizes)
            if mean > 0:
                spread = max(sizes) - min(sizes)
                balance = max(0.0, 1.0 - (spread / mean))

        underrep = [
            {"class_name": c["name"], "size": len(c.get("examples", [])), "recommended_size": 5}
            for c in cat_classes if len(c.get("examples", [])) < 5
        ]

        # Skip pairwise ambiguity in quick mode
        ambiguity_matrix: list[dict[str, Any]] = []
        threshold_recommendations: list[dict[str, Any]] = []
        if depth in ("standard", "full") and len(cat_classes) >= 2:
            for i in range(len(cat_classes)):
                for j in range(i + 1, len(cat_classes)):
                    cmp = await self.compare_classes(
                        catalog=catalog,
                        class_a=cat_classes[i]["name"],
                        class_b=cat_classes[j]["name"],
                        auth=auth,
                    )
                    if cmp.get("ambiguity_band") in ("moderate", "high", "critical"):
                        ambiguity_matrix.append({
                            "class_a": cat_classes[i]["name"],
                            "class_b": cat_classes[j]["name"],
                            "separation": cmp.get("separation_score", 0.0),
                            "band": cmp.get("ambiguity_band", "none"),
                        })

        recs: list[str] = []
        for u in underrep[:5]:
            recs.append(
                f"Add ≥{5 - u['size']} examples to class '{u['class_name']}'"
            )
        for amb in ambiguity_matrix[:5]:
            recs.append(
                f"Review boundary between {amb['class_a']!r} and {amb['class_b']!r}"
            )

        # Audit fix M17-A3 (MED) : the ambiguity score must be 1 -
        # (n_ambig_pairs / total_pairs) where total_pairs = C(n, 2),
        # not n_classes. Previous formula massively over-weighted
        # ambiguity for catalogs with many classes (e.g. 256 classes ×
        # 5 ambig pairs gave score 0.98 when reality is ~0.0001).
        n_pairs = (len(cat_classes) * (len(cat_classes) - 1)) // 2
        ambiguity_score = (
            1.0 - (len(ambiguity_matrix) / n_pairs)
            if n_pairs > 0 else 1.0
        )
        scores = {
            "ambiguity": round(max(0.0, min(1.0, ambiguity_score)), 4),
            "coverage": round(coverage, 4),
            "balance": round(balance, 4),
            "threshold_quality": 0.85,    # placeholder in fallback mode
            "audit_completeness": 1.0,
        }
        avg_score = sum(scores.values()) / len(scores)
        if avg_score > 0.85:
            health = "excellent"
        elif avg_score > 0.7:
            health = "good"
        elif avg_score > 0.55:
            health = "fair"
        elif avg_score > 0.4:
            health = "poor"
        else:
            health = "critical"

        return {
            "catalog": catalog,
            "version": version,
            "fingerprint": fingerprint,
            "overall_health": health,
            "scores": scores,
            "ambiguity_matrix": ambiguity_matrix,
            "underrepresented_classes": underrep,
            "threshold_recommendations": threshold_recommendations,
            "drift_signals": [],
            "recommended_actions": recs,
            "fallback": "local_compute",
        }

    async def detect_drift(
        self, *, catalog: str, version_a: int | None, version_b: int | None, auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", f"/catalog/{catalog}/drift", auth=auth,
                json_body={
                    **({"version_a": version_a} if version_a else {}),
                    **({"version_b": version_b} if version_b else {}),
                },
            )
        except BackendNotFound:
            return await self._detect_drift_fallback(
                catalog=catalog, version_a=version_a, version_b=version_b, auth=auth,
            )

    async def _detect_drift_fallback(
        self, *, catalog: str, version_a: int | None, version_b: int | None, auth: AuthContext,
    ) -> dict[str, Any]:
        from .analysis.drift import drift_recommendation, overall_drift_score, per_class_drift

        cat_list = await self.list_catalogs(auth=auth)
        match = next((c for c in cat_list if c.get("name") == catalog), None)
        if not match:
            raise BackendNotFound(f"catalog {catalog!r} not found")
        all_versions = match.get("all_versions") or []
        version_numbers = sorted(v["version"] for v in all_versions)
        if len(version_numbers) < 2:
            return {
                "catalog": catalog,
                "version_a": version_numbers[0] if version_numbers else None,
                "version_b": version_numbers[0] if version_numbers else None,
                "fingerprint_a": None,
                "fingerprint_b": None,
                "per_class_drift": [],
                "new_classes": [],
                "removed_classes": [],
                "overall_drift": 0.0,
                "recommendation": "Catalog has fewer than 2 versions ; nothing to compare.",
                "fallback": "local_compute",
            }
        v_b = version_b if version_b is not None else version_numbers[-1]
        v_a = version_a if version_a is not None else version_numbers[-2]
        ver_a = await self._request("GET", f"/catalog/{catalog}/versions/{v_a}", auth=auth)
        ver_b = await self._request("GET", f"/catalog/{catalog}/versions/{v_b}", auth=auth)
        drifts, new_c, removed_c = per_class_drift(
            classes_a=ver_a.get("classes", []),
            classes_b=ver_b.get("classes", []),
        )
        overall = overall_drift_score(drifts)
        return {
            "catalog": catalog,
            "version_a": v_a,
            "version_b": v_b,
            "fingerprint_a": ver_a.get("fingerprint"),
            "fingerprint_b": ver_b.get("fingerprint"),
            "per_class_drift": [
                {
                    "class_name": d.class_name,
                    "centroid_shift_cosine": d.centroid_shift_cosine,
                    "examples_added": d.examples_added,
                    "examples_removed": d.examples_removed,
                    "size_a": d.size_a,
                    "size_b": d.size_b,
                    "drift_severity": d.drift_severity,
                }
                for d in drifts
            ],
            "new_classes": new_c,
            "removed_classes": removed_c,
            "overall_drift": overall,
            "recommendation": drift_recommendation(overall, len(new_c), len(removed_c)),
            "fallback": "local_compute",
        }

    async def audit_health_check(
        self, *, from_timestamp: float | None, to_timestamp: float | None,
        deep: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        params: dict[str, Any] = {"deep": "true" if deep else "false"}
        if from_timestamp is not None:
            params["from_timestamp"] = from_timestamp
        if to_timestamp is not None:
            params["to_timestamp"] = to_timestamp
        try:
            return await self._request(
                "GET", "/audit/integrity", auth=auth, params=params,
            )
        except BackendNotFound:
            # Fallback to playground's /audit/integrity (shape compat)
            data = await self._request("GET", "/audit/integrity", auth=auth)
            data.setdefault("verification_method", "incremental")
            return data

    async def export_catalog(
        self, *, catalog: str, version: int | None, format: str,
        include_centroids: bool, include_audit_excerpt: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            params = {
                "format": format,
                "include_centroids": "true" if include_centroids else "false",
                "include_audit_excerpt": "true" if include_audit_excerpt else "false",
            }
            if version is not None:
                params["version"] = version
            return await self._request(
                "GET", f"/catalog/{catalog}/export", auth=auth, params=params,
            )
        except BackendNotFound:
            return await self._export_fallback(
                catalog=catalog, version=version, format=format,
                include_centroids=include_centroids, auth=auth,
            )

    async def _export_fallback(
        self, *, catalog: str, version: int | None, format: str,
        include_centroids: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        import json as _json

        v = version
        if v is None:
            cat_list = await self.list_catalogs(auth=auth)
            match = next((c for c in cat_list if c.get("name") == catalog), None)
            if not match:
                raise BackendNotFound(f"catalog {catalog!r} not found")
            v = match.get("latest_version")
        data = await self._request(
            "GET", f"/catalog/{catalog}/versions/{v}", auth=auth,
        )
        if not include_centroids:
            for cls in data.get("classes", []):
                cls.pop("centroid", None)
        body = _json.dumps(data, sort_keys=True, ensure_ascii=False)
        return {
            "catalog": catalog,
            "version": v,
            "format": format,
            "size_bytes": len(body.encode("utf-8")),
            "content": data,
            "exported_at": _now_unix(),
            "fallback": "local_compute",
        }

    async def consolidate_versions(
        self, *, catalog: str, keep_last_n: int, dry_run: bool, auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", f"/catalog/{catalog}/consolidate", auth=auth,
                json_body={"keep_last_n": keep_last_n, "dry_run": dry_run},
            )
        except BackendNotFound:
            if dry_run:
                cat_list = await self.list_catalogs(auth=auth)
                match = next((c for c in cat_list if c.get("name") == catalog), None)
                if not match:
                    raise BackendNotFound(f"catalog {catalog!r} not found")
                versions = sorted(
                    v["version"] for v in match.get("all_versions") or []
                )
                preserved = versions[-keep_last_n:] if keep_last_n < len(versions) else versions
                to_delete = versions[: max(0, len(versions) - keep_last_n)]
                return {
                    "catalog": catalog,
                    "current_versions_count": len(versions),
                    "keep_last_n": keep_last_n,
                    "to_delete_count": len(to_delete),
                    "to_delete_versions": to_delete,
                    "preserved_versions": preserved,
                    "estimated_savings_bytes": 50_000 * len(to_delete),
                    "dry_run": True,
                    "fallback": "local_compute",
                    "next_action_hint": (
                        "Backend does not support consolidate ; only dry-run is "
                        "available. Upgrade to EFE 1.1 to actually delete."
                    ),
                }
            raise BackendError(
                "consolidate_versions with dry_run=false requires EFE 1.1 backend",
                backend_code="not_supported",
                status_code=501,
            )

    async def list_catalog_classes(
        self, *, catalog: str, auth: AuthContext,
    ) -> list[dict[str, Any]]:
        return await self._fetch_catalog_classes(catalog=catalog, auth=auth)

    async def get_class_examples(
        self, *, catalog: str, class_name: str, auth: AuthContext,
    ) -> dict[str, Any]:
        classes = await self._fetch_catalog_classes(catalog=catalog, auth=auth)
        match = next((c for c in classes if c.get("name") == class_name), None)
        if not match:
            raise BackendNotFound(
                f"class {class_name!r} not found in catalog {catalog!r}"
            )
        return match

    async def get_info(self, *, auth: AuthContext) -> dict[str, Any]:
        try:
            return await self._request("GET", "/info", auth=auth)
        except (BackendForbidden, BackendNotFound):
            return {}

    async def get_admin_bundle_info(self, *, auth: AuthContext) -> dict[str, Any]:
        """REST impl of the M15-A5 `/admin/bundle-info` endpoint.

        Returns `{}` (empty) when the endpoint isn't exposed, mirroring
        `get_info()`'s tolerant behaviour. Callers must handle empty.
        """
        try:
            return await self._request("GET", "/admin/bundle-info", auth=auth)
        except (BackendForbidden, BackendNotFound):
            return {}

    async def bulk_classify(
        self, *, catalog: str, texts: list[str], top_k: int,
        include_explanations: bool, auth: AuthContext, progress: Any = None,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", f"/catalog/{catalog}/bulk-classify", auth=auth,
                json_body={
                    "texts": texts, "top_k": top_k,
                    "include_explanations": include_explanations,
                },
            )
        except BackendNotFound as exc:
            raise NotImplementedError(
                "backend does not expose bulk-classify ; falling back"
            ) from exc

    # ─── M18 D47 : bundle introspection ─────────────────────────────

    async def list_known_bundles(
        self,
        *,
        registry_filter: str | None,
        slug_filter: str | None,
        limit: int,
        auth: AuthContext,
    ) -> dict[str, Any]:
        params: dict[str, Any] = {"limit": limit}
        if registry_filter:
            params["registry"] = registry_filter
        if slug_filter:
            params["slug"] = slug_filter
        try:
            return await self._request(
                "GET", "/admin/bundles", auth=auth, params=params,
            )
        except BackendNotFound as exc:
            raise NotImplementedError(
                "backend does not expose /admin/bundles ; "
                "falling back to direct OCI listing"
            ) from exc

    async def get_bundle_record(
        self, *, slug: str, version: str, auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "GET", f"/admin/bundles/{slug}/{version}", auth=auth,
            )
        except BackendNotFound as exc:
            raise NotImplementedError(
                "backend does not expose /admin/bundles/<slug>/<version>"
            ) from exc

    async def get_trust_store(self, *, auth: AuthContext) -> dict[str, Any]:
        try:
            return await self._request("GET", "/admin/trust-store", auth=auth)
        except BackendNotFound as exc:
            raise NotImplementedError(
                "backend does not expose /admin/trust-store"
            ) from exc

    # ─── M18 D49 : deployment helpers ───────────────────────────────

    async def get_deployment_current(self, *, auth: AuthContext) -> dict[str, Any]:
        try:
            return await self._request(
                "GET", "/admin/deployments/current", auth=auth,
            )
        except BackendNotFound as exc:
            raise NotImplementedError(
                "backend does not expose /admin/deployments/current"
            ) from exc

    async def get_deployment_history(
        self, *, limit: int, auth: AuthContext,
    ) -> list[dict[str, Any]]:
        try:
            data = await self._request(
                "GET", "/admin/deployments/history",
                auth=auth, params={"limit": limit},
            )
        except BackendNotFound as exc:
            raise NotImplementedError(
                "backend does not expose /admin/deployments/history"
            ) from exc
        deployments = data.get("deployments") or []
        if isinstance(deployments, list):
            return deployments[:limit]
        return []

    async def dry_run_deployment(
        self,
        *,
        target_slug: str,
        target_version: str,
        target_manifest_sha256: str | None,
        environment: str,
        auth: AuthContext,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", "/admin/deployments/dry-run", auth=auth,
                json_body={
                    "target_bundle": {
                        "slug": target_slug,
                        "version": target_version,
                        "manifest_sha256": target_manifest_sha256,
                    },
                    "environment": environment,
                },
            )
        except BackendNotFound as exc:
            raise NotImplementedError(
                "backend does not expose /admin/deployments/dry-run"
            ) from exc

    # ─── M18 D50 : deploy write tools ───────────────────────────────

    async def register_bundle(
        self, *, slug, version, registry_url, manifest_sha256,
        verify_before_register, idempotency_key, auth,
    ) -> dict[str, Any]:
        # Audit fix M18-FINAL-A4 : idempotency_key flows via the
        # Idempotency-Key header (M11 cache convention), not body.
        body: dict[str, Any] = {
            "slug": slug,
            "version": version,
            "registry_url": registry_url,
            "verify_before_register": bool(verify_before_register),
        }
        if manifest_sha256:
            body["manifest_sha256"] = manifest_sha256
        try:
            return await self._request(
                "POST", "/admin/bundles/register", auth=auth, json_body=body,
                idempotency_key=idempotency_key,
            )
        except BackendNotFound as exc:
            raise NotImplementedError(
                "backend does not expose /admin/bundles/register"
            ) from exc

    async def promote_bundle(
        self, *, slug, version, scope, idempotency_key, auth,
    ) -> dict[str, Any]:
        try:
            return await self._request(
                "POST", f"/admin/bundles/{slug}/{version}/promote",
                auth=auth, json_body={"scope": scope},
                idempotency_key=idempotency_key,
            )
        except BackendNotFound as exc:
            raise NotImplementedError(
                f"backend does not expose /admin/bundles/{slug}/{version}/promote"
            ) from exc

    async def request_deployment_ticket(
        self, *, slug, version, environment, rationale, tracker,
        idempotency_key, auth,
    ) -> dict[str, Any]:
        body: dict[str, Any] = {
            "bundle": {"slug": slug, "version": version},
            "environment": environment,
            "rationale": rationale,
            "tracker": tracker,
        }
        try:
            return await self._request(
                "POST", "/admin/tickets", auth=auth, json_body=body,
                idempotency_key=idempotency_key,
            )
        except BackendNotFound as exc:
            raise NotImplementedError(
                "backend does not expose /admin/tickets"
            ) from exc


def _now_unix() -> float:
    import time as _t
    return _t.time()


def _raise_for_status(resp: httpx.Response) -> dict[str, Any]:
    """Map HTTP status to BackendError subclasses."""
    try:
        body: Any = resp.json()
    except ValueError:
        body = {"raw_text": resp.text}

    code = resp.status_code
    if 200 <= code < 300:
        if isinstance(body, dict):
            return body
        return {"data": body}

    msg = ""
    backend_code = None
    if isinstance(body, dict):
        msg = body.get("error") or body.get("message") or body.get("detail") or ""
        backend_code = body.get("code")
    msg = msg or f"HTTP {code}"

    if code == 400 or code == 422:
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


# ─────────────────────────────────────────────────────────────────────
# Unix-socket implementation (M19 future ; thin shim over httpx UDS)
# ─────────────────────────────────────────────────────────────────────


class UnixSocketBackendClient(RestBackendClient):
    """REST over a Unix domain socket (sidecar pattern).

    Endpoint format : `unix:///var/run/efe.sock`. The path after `unix://`
    is the socket file. Falls back to local HTTP semantics.
    """

    def __init__(self, endpoint: str, *, timeout_seconds: float = 30.0) -> None:
        if not endpoint.startswith("unix://"):
            raise ValueError(f"expected unix://… endpoint, got {endpoint!r}")
        socket_path = endpoint[len("unix://"):]
        # httpx supports UDS via `transport=httpx.AsyncHTTPTransport(uds=...)`
        self._endpoint = "http://efe.local"
        self.info = BackendInfo(protocol="unix", endpoint=endpoint)
        transport = httpx.AsyncHTTPTransport(uds=socket_path)
        self._client = httpx.AsyncClient(
            base_url=self._endpoint,
            timeout=timeout_seconds,
            transport=transport,
            headers={"User-Agent": "efe-mcp-server/0.1.0"},
        )


# ─────────────────────────────────────────────────────────────────────
# Auto-detect factory
# ─────────────────────────────────────────────────────────────────────


def detect_backend(
    endpoint: str, *, timeout_seconds: float = 30.0, verify_tls: bool = True
) -> BackendClient:
    """Pick the right backend client from `endpoint`.

    Resolution order :
      1. `EFE_BACKEND_PROTOCOL` env var (explicit override) ;
      2. URL scheme :
           - `http://`  / `https://` → REST ;
           - `unix://`               → Unix socket ;
           - `grpc://`  / `grpcs://` → gRPC (requires `grpcio` extra).
    """
    override = os.environ.get("EFE_BACKEND_PROTOCOL", "").strip().lower()
    parsed = urlparse(endpoint)
    scheme = (override or parsed.scheme or "").lower()

    if scheme in ("http", "https", "rest"):
        # If override is `rest`, keep the original endpoint URL.
        url = endpoint if not override else endpoint
        return RestBackendClient(url, timeout_seconds=timeout_seconds, verify_tls=verify_tls)
    if scheme in ("unix",):
        return UnixSocketBackendClient(endpoint, timeout_seconds=timeout_seconds)
    if scheme in ("grpc", "grpcs"):
        # gRPC is an optional extra to keep the default install lightweight.
        try:
            from .backend_client_grpc import GrpcBackendClient  # type: ignore[import-not-found]
        except ImportError as exc:
            raise BackendError(
                "gRPC backend requested but `grpcio` extra is not installed ; "
                "install with `pip install efe-mcp-server[grpc]`"
            ) from exc
        return GrpcBackendClient(  # type: ignore[name-defined]
            endpoint, timeout_seconds=timeout_seconds
        )
    raise BackendError(
        f"unsupported endpoint scheme {scheme!r} ; "
        f"expected http(s), unix, or grpc(s) — got {endpoint!r}"
    )

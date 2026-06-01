"""High-level admin client wrapping the gRPC AdminService.

Sprint D11.4 — public sub-package introduced in v0.1.11. The
client is **synchronous** ; an async variant follows the same
staged-delivery pattern as :class:`efe_tool.AsyncEfeClient` (full
native impl in v0.2.0).

Tenant scoping
──────────────
Every method calls the gRPC ``AdminService`` with the JWT in
``authorization`` metadata. The server filters / enforces tenant
scope on the caller side:

  * :meth:`list_catalogs` — server returns only catalogs visible
    to the JWT's ``tenant_id`` claim.
  * :meth:`list_cone_versions` / :meth:`get_cone` /
    :meth:`register_cone` / :meth:`evolve_cone` /
    :meth:`delete_cone` — server reject (PERMISSION_DENIED) if
    the request's ``tenant`` field doesn't match the JWT.

The SDK does NOT enforce this client-side ; it relies on the
server's ``assert_tenant_match`` invariant (D11 architecture §5.1).

Usage
─────

::

    from efe_tool.admin import EfeAdminClient

    with EfeAdminClient(
        endpoint="efe.example.com:50051",
        jwt=os.environ["EFE_JWT"],
    ) as admin:
        for ref in admin.list_catalogs():
            print(f"{ref.name}@{ref.version}: {ref.num_classes} classes")
"""

from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
from typing import Any, Iterator

from efe_tool.errors import (
    EfeAuthError,
    EfeError,
    EfeInternalError,
    EfeInvalidArgumentError,
    EfeNotFoundError,
    raise_from_grpc,
)


# ─────────────────────────────────────────────────────────────────────
# Public types
# ─────────────────────────────────────────────────────────────────────

@dataclass(frozen=True)
class CatalogRef:
    """Lightweight descriptor of a catalog visible to the caller's tenant.

    Returned by :meth:`EfeAdminClient.list_catalogs`. The server
    sets ``tenant`` to the caller's tenant_id on every response —
    the SDK NEVER returns catalogs from a foreign tenant.

    Attributes:
        tenant: The owning tenant (always == caller's tenant_id).
        name: Human-readable catalog name (matches ``metadata.name``
            in the YAML).
        version: SemVer-shaped version string.
        num_classes: Number of classes defined.
        num_anchors: Aggregate number of anchor prompts.
    """
    tenant: str
    name: str
    version: str
    num_classes: int
    num_anchors: int


class AdminError(EfeError):
    """Admin-flow error. Subclass of :class:`efe_tool.errors.EfeError`
    so legacy ``except EfeError`` continues to catch."""


# ─────────────────────────────────────────────────────────────────────
# Client
# ─────────────────────────────────────────────────────────────────────

class EfeAdminClient:
    """Synchronous gRPC admin client.

    Args:
        endpoint: ``host:port`` of the EFE gRPC server.
        jwt: Bearer token. Must carry ``tenant_id`` and either
            ``admin:cone:read`` or ``admin:cone:write`` scopes
            depending on which methods you call.
        insecure: Use plaintext channel (dev only).
        timeout_s: Per-call timeout in seconds.

    The client lazily opens the gRPC channel on first call. Use
    as a context manager for deterministic cleanup::

        with EfeAdminClient(endpoint="...", jwt="...") as admin:
            ...
    """

    def __init__(
        self,
        *,
        endpoint: str,
        jwt: str,
        insecure: bool = False,
        timeout_s: float = 10.0,
    ) -> None:
        if not endpoint:
            raise AdminError("endpoint is required")
        if not jwt:
            raise AdminError("jwt is required for admin operations")
        self._endpoint = endpoint
        self._jwt = jwt
        self._insecure = insecure
        self._timeout_s = timeout_s
        self._channel: Any | None = None
        self._stub: Any | None = None

    # ─── Lifecycle ───────────────────────────────────────────────

    def __enter__(self) -> "EfeAdminClient":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def close(self) -> None:
        if self._channel is not None:
            try:
                self._channel.close()
            except Exception:  # pragma: no cover — defensive
                pass
            self._channel = None
            self._stub = None

    def _ensure_stub(self) -> Any:
        if self._stub is not None:
            return self._stub
        import grpc
        from efe_tool._proto.efe.v1 import admin_pb2_grpc as _admin_grpc

        if self._insecure:
            self._channel = grpc.insecure_channel(self._endpoint)
        else:
            creds = grpc.ssl_channel_credentials()
            self._channel = grpc.secure_channel(self._endpoint, creds)
        self._stub = _admin_grpc.AdminServiceStub(self._channel)
        return self._stub

    def _metadata(self) -> list[tuple[str, str]]:
        return [("authorization", f"Bearer {self._jwt}")]

    @contextmanager
    def _grpc_call(self, op_name: str) -> Iterator[None]:
        """Translate gRPC errors → typed :class:`EfeError` subclasses."""
        try:
            yield
        except Exception as exc:  # noqa: BLE001 — re-raise typed
            import grpc
            if isinstance(exc, grpc.RpcError):
                raise raise_from_grpc(exc) from exc
            raise AdminError(
                f"admin {op_name} failed: {exc}"
            ) from exc

    # ─── Catalog operations ─────────────────────────────────────

    def list_catalogs(
        self,
        *,
        tenant: str | None = None,
        page_size: int = 100,
        page_token: str | None = None,
    ) -> list[CatalogRef]:
        """List catalogs visible to the authenticated tenant.

        Sprint D12 (v0.1.12) — closes the D11-B1 audit debt. The
        underlying ``ListCatalogs`` RPC now lives in the new
        ``efe.v1.catalog.CatalogService`` (separate file
        ``protos/efe/v1/catalog.proto``).

        The server filters by the JWT's ``tenant_id`` claim ;
        this method NEVER returns catalogs from foreign tenants.

        Args:
            tenant: tenant_id, MUST match the JWT claim. If unset,
                the SDK extracts it from the JWT.
            page_size: max items per response (server caps at 500).
            page_token: opaque continuation token from a previous
                ``next_page_token``. Empty for first page.

        Returns:
            list of :class:`CatalogRef` (lightweight summary).

        Raises:
            EfeAuthError: token absent / invalid / expired.
            EfeNotFoundError: pre-D12 server (no CatalogService).
            EfeError: any other gRPC failure.
        """
        try:
            from efe_tool._proto.efe.v1 import catalog_pb2, catalog_pb2_grpc
        except ImportError as exc:
            raise AdminError(
                "ListCatalogs requires the regenerated catalog_pb2 "
                "stubs (Sprint D12). Reinstall efe-tool ≥ 0.1.12, "
                "or regenerate via: "
                "python -m grpc_tools.protoc --proto_path=protos "
                "--python_out=clients/python/efe_tool/_proto "
                "--grpc_python_out=clients/python/efe_tool/_proto "
                "protos/efe/v1/catalog.proto"
            ) from exc

        stub = self._catalog_stub()
        with self._grpc_call("list_catalogs"):
            req = catalog_pb2.ListCatalogsRequest(
                tenant=tenant or self._tenant_from_jwt(),
                page_size=int(page_size),
                page_token=page_token or "",
            )
            resp = stub.ListCatalogs(
                req, metadata=self._metadata(),
                timeout=self._timeout_s,
            )
        return [
            CatalogRef(
                tenant=c.tenant,
                name=c.name,
                version=int(c.latest_version),
                num_classes=int(c.num_classes),
                num_anchors=int(c.num_anchors),
            )
            for c in resp.catalogs
        ]

    # ─── Catalog admin (D12 NEW) ─────────────────────────────────

    def register_catalog(
        self,
        *,
        tenant: str,
        name: str,
        version: str,
        content: "dict",
        description: str = "",
        labels: "dict[str, str] | None" = None,
    ) -> "RegisteredCatalog":
        """Register a new catalog version. Atomic.

        Args:
            tenant: tenant_id ; MUST match the JWT claim.
            name: catalog name (``[a-z][a-z0-9_-]{0,62}``).
            version: SemVer-shaped (informational ; the server
                assigns the integer version monotonically).
            content: YAML-parseable dict — schema matches
                :class:`efe_tool.AnchorCatalog` ``apiVersion:
                efe.mode1/v1``. Required keys: ``embedder``,
                ``classes`` (≥ 2), ``cone_policy``, ``thresholds``.
            description: free-text audit note.
            labels: free-form key-value pairs.

        Returns:
            :class:`RegisteredCatalog` with the canonical key
            ``tenant/name@vN``.

        Raises:
            EfeAuthError: token issue.
            EfeInvalidArgumentError: schema violation, embedder
                fingerprint mismatch, quota exceeded.
            EfeError: other gRPC failure.
        """
        from efe_tool._proto.efe.v1 import catalog_pb2

        stub = self._catalog_stub()
        proto_content = _content_dict_to_proto(content, catalog_pb2)
        with self._grpc_call("register_catalog"):
            req = catalog_pb2.RegisterCatalogRequest(
                tenant=tenant,
                name=name,
                version=version,
                content=proto_content,
                created_by=self._jwt_subject(),
                description=description,
                labels=dict(labels or {}),
            )
            resp = stub.RegisterCatalog(
                req, metadata=self._metadata(),
                timeout=self._timeout_s,
            )
        return _proto_handle_to_registered(resp, catalog_pb2)

    def evolve_catalog(
        self,
        *,
        from_key: str,
        op: "EvolveOp",
        op_payload: "dict",
        description: str = "",
    ) -> "RegisteredCatalog":
        """Apply an evolution operation, returning the new version.

        Args:
            from_key: canonical key of the source version
                (``tenant/name@vN``).
            op: discriminated tag (cf. :class:`EvolveOp`).
            op_payload: per-op parameters (cf. helpers in
                :mod:`efe_tool.admin.builders`).
            description: free-text audit note.

        Returns:
            :class:`RegisteredCatalog` for the NEW version
            (incremented +1 relative to ``from``).

        Raises:
            EfeAuthError, EfeInvalidArgumentError, EfeNotFoundError.
        """
        from efe_tool._proto.efe.v1 import catalog_pb2

        stub = self._catalog_stub()
        op_proto = _evolve_op_to_proto(op, op_payload, catalog_pb2)
        with self._grpc_call("evolve_catalog"):
            req_kwargs: dict[str, Any] = {
                "from_key": from_key,
                "op": op_proto,
            }
            if description:
                # Thread caller-supplied description through
                # `metadata_override` ; the server may also stamp
                # `created_by` from the validated JWT (we send our
                # best-effort `sub` as a hint).
                req_kwargs["metadata_override"] = catalog_pb2.CatalogMetadata(
                    description=description,
                    created_by=self._jwt_subject(),
                )
            req = catalog_pb2.EvolveCatalogRequest(**req_kwargs)
            resp = stub.EvolveCatalog(
                req, metadata=self._metadata(),
                timeout=self._timeout_s,
            )
        return _proto_handle_to_registered(resp, catalog_pb2)

    def get_catalog(
        self,
        key: str,
        *,
        include_compiled: bool = False,
    ) -> "RegisteredCatalog":
        """Fetch a specific catalog version by canonical key.

        Args:
            key: canonical key ``tenant/name@vN``.
            include_compiled: if True, response includes per-class
                compiled anchor metadata (requires
                ``admin:catalog:read`` scope).

        Returns:
            :class:`RegisteredCatalog`.

        Raises:
            EfeAuthError, EfeNotFoundError, EfeInvalidArgumentError.
        """
        from efe_tool._proto.efe.v1 import catalog_pb2

        # Full-parse the key — catches all malformed forms
        # (empty segments, missing 'v', non-integer version, etc.).
        _parse_canonical_key(key)

        stub = self._catalog_stub()
        with self._grpc_call("get_catalog"):
            req = catalog_pb2.GetCatalogRequest(
                key=key, include_compiled=bool(include_compiled),
            )
            resp = stub.GetCatalog(
                req, metadata=self._metadata(),
                timeout=self._timeout_s,
            )
        return _proto_canonical_to_registered(resp, key, catalog_pb2)

    def list_catalog_versions(
        self,
        *,
        tenant: str,
        name: str,
    ) -> list[str]:
        """List all versions for ``(tenant, name)``.

        ``tenant`` MUST match the JWT claim.

        Returns:
            list of canonical keys, sorted ascending by version.

        Raises:
            EfeAuthError, EfeNotFoundError, EfeInvalidArgumentError.
        """
        from efe_tool._proto.efe.v1 import catalog_pb2

        if not tenant or not name:
            raise EfeInvalidArgumentError(
                "tenant and name must be non-empty"
            )

        stub = self._catalog_stub()
        with self._grpc_call("list_catalog_versions"):
            req = catalog_pb2.ListCatalogVersionsRequest(
                tenant=tenant, name=name,
            )
            resp = stub.ListCatalogVersions(
                req, metadata=self._metadata(),
                timeout=self._timeout_s,
            )
        return list(resp.keys)

    def delete_catalog(
        self,
        key: str,
        *,
        force: bool = False,
    ) -> None:
        """Delete a specific catalog version.

        Args:
            key: canonical key ``tenant/name@vN``.
            force: if True, cascade-delete dependent versions.

        Raises:
            EfeAuthError, EfeNotFoundError ; EfeInvalidArgumentError
            if ``force=False`` and the key is referenced.
        """
        from efe_tool._proto.efe.v1 import catalog_pb2

        # Full-parse the key — catches all malformed forms
        # (empty segments, missing 'v', non-integer version, etc.).
        # Same validation as `get_catalog` and `list_catalog_versions`.
        _parse_canonical_key(key)

        stub = self._catalog_stub()
        with self._grpc_call("delete_catalog"):
            req = catalog_pb2.DeleteCatalogRequest(
                key=key, force=bool(force),
            )
            stub.DeleteCatalog(
                req, metadata=self._metadata(),
                timeout=self._timeout_s,
            )

    # ─── Internal helpers (D12) ─────────────────────────────────

    def _catalog_stub(self):
        """Lazily build the CatalogServiceStub on the same channel
        as the AdminServiceStub."""
        if getattr(self, "_catalog_stub_cache", None) is not None:
            return self._catalog_stub_cache
        # Reuse the channel created by `_ensure_stub()` so we don't
        # pay another TCP/TLS handshake for catalog ops.
        self._ensure_stub()
        from efe_tool._proto.efe.v1 import catalog_pb2_grpc
        self._catalog_stub_cache = catalog_pb2_grpc.CatalogServiceStub(
            self._channel,
        )
        return self._catalog_stub_cache

    def _tenant_from_jwt(self) -> str:
        """Extract `tenant_id` claim from the JWT without verification.

        The JWT signature is the SERVER's job to verify ; this is
        only used to populate request fields where the server
        will re-validate against the same JWT (`assert_tenant_match`
        invariant).
        """
        import base64
        import json
        try:
            _, payload_b64, _ = self._jwt.split(".", 2)
        except ValueError as exc:
            raise AdminError(
                "JWT does not have the 3-segment form ; "
                "cannot extract tenant_id"
            ) from exc
        # Pad the base64 payload (JWT spec strips trailing '=').
        padded = payload_b64 + "=" * (-len(payload_b64) % 4)
        try:
            claims = json.loads(base64.urlsafe_b64decode(padded))
        except Exception as exc:
            raise AdminError(
                f"JWT payload is not valid base64-JSON: {exc}"
            ) from exc
        tenant = claims.get("tenant_id", "")
        if not tenant:
            raise AdminError(
                "JWT has no `tenant_id` claim — cannot scope "
                "the request to a tenant. Either re-issue the JWT "
                "with a `tenant_id` claim, or pass `tenant=` "
                "explicitly to the SDK method."
            )
        return str(tenant)

    def _jwt_subject(self) -> str:
        """Best-effort extraction of `sub` claim for audit
        attribution. Returns an empty string when absent — the
        server may also stamp `sub` from the validated JWT."""
        import base64
        import json
        try:
            _, payload_b64, _ = self._jwt.split(".", 2)
        except ValueError:
            return ""
        padded = payload_b64 + "=" * (-len(payload_b64) % 4)
        try:
            claims = json.loads(base64.urlsafe_b64decode(padded))
        except Exception:
            return ""
        return str(claims.get("sub", ""))

    # ─── Cone operations ────────────────────────────────────────

    def list_cone_versions(self, *, tenant: str, task: str) -> list[str]:
        """List cone versions for ``(tenant, task)``.

        ``tenant`` MUST match the JWT's ``tenant_id`` claim.

        Args:
            tenant: tenant_id (must match JWT's tenant_id).
            task: task identifier within the tenant.

        Returns:
            sorted list of canonical keys (``tenant/task@vN``).

        Raises:
            EfeAuthError: token issue.
            EfeInvalidArgumentError: tenant mismatch with JWT,
                or empty tenant/task.
            EfeNotFoundError: no cones registered for (tenant, task).
        """
        from efe_tool._proto.efe.v1 import admin_pb2

        if not tenant or not task:
            raise EfeInvalidArgumentError(
                "tenant and task must be non-empty"
            )

        stub = self._ensure_stub()
        with self._grpc_call("list_cone_versions"):
            req = admin_pb2.ListVersionsRequest(tenant=tenant, task=task)
            resp = stub.ListVersions(
                req,
                metadata=self._metadata(),
                timeout=self._timeout_s,
            )
        return list(resp.keys)

    def get_cone(self, key: str) -> Any:
        """Fetch one canonical cone by key.

        Args:
            key: canonical key, format ``tenant/task@vN``.

        Returns:
            ``CanonicalCone`` proto message (cone + metadata).

        Raises:
            EfeNotFoundError: key doesn't exist.
            EfeAuthError: token issue.
            EfeInvalidArgumentError: malformed key.
        """
        from efe_tool._proto.efe.v1 import admin_pb2

        if not key:
            raise EfeInvalidArgumentError("key must be non-empty")
        if "/" not in key or "@" not in key:
            raise EfeInvalidArgumentError(
                f"key {key!r} is not in 'tenant/task@vN' form"
            )

        stub = self._ensure_stub()
        with self._grpc_call("get_cone"):
            req = admin_pb2.GetConeRequest(key=key)
            resp = stub.GetCone(
                req,
                metadata=self._metadata(),
                timeout=self._timeout_s,
            )
        return resp

    def delete_cone(self, key: str, *, force: bool = False) -> None:
        """Delete a cone version.

        Args:
            key: canonical key.
            force: if True, also remove referenced cones (use with
                care — breaks downstream classifiers).

        Raises:
            EfeAuthError: token issue.
            EfeNotFoundError: key doesn't exist.
            EfeInvalidArgumentError: cone is referenced and
                ``force=False``.
        """
        from efe_tool._proto.efe.v1 import admin_pb2

        stub = self._ensure_stub()
        with self._grpc_call("delete_cone"):
            req = admin_pb2.DeleteConeRequest(key=key, force=force)
            stub.DeleteCone(
                req,
                metadata=self._metadata(),
                timeout=self._timeout_s,
            )

    # ─── Diagnostics ────────────────────────────────────────────

    def health(self) -> bool:
        """Lightweight reachability probe — channel-only, no RPC fired.

        Opens the gRPC channel via :meth:`_ensure_stub` and returns
        :pydata:`True` if the channel can be created. Auth is NOT
        probed (no metadata exchange happens). For an auth-aware
        probe, call :meth:`list_catalogs` directly and catch
        :class:`EfeAuthError`.

        Sprint D12 (v0.1.12) — formerly delegated to ``list_catalogs``
        (which raised :class:`NotImplementedError` until D12). The
        channel-open fallback is kept as the canonical health check
        because it costs zero RPCs and works against pre-D12 servers
        for backward compatibility.

        Returns:
            ``True`` if the gRPC channel was successfully opened.

        Raises:
            AdminError: channel creation failed (DNS / TCP / TLS).
            EfeError: re-raised if `_ensure_stub` produced a typed
                error (rare — the channel constructor is mostly
                deterministic).
        """
        try:
            # `_ensure_stub` opens the channel and creates the
            # AdminServiceStub. No RPC fired ; if this returns,
            # the endpoint is at least TCP-reachable.
            self._ensure_stub()
            return True
        except Exception as exc:  # noqa: BLE001
            if isinstance(exc, EfeError):
                raise
            raise AdminError(f"health probe failed: {exc}") from exc


# ─────────────────────────────────────────────────────────────────────
# Sprint D12 — proto ⇄ Python conversion helpers
# ─────────────────────────────────────────────────────────────────────
#
# These are MODULE-PRIVATE (underscore prefix) ; they are not part of
# the SDK's public API. Tests may import them directly to verify
# round-trip fidelity:
#
#     from efe_tool.admin.client import (
#         _content_dict_to_proto, _proto_canonical_to_registered,
#     )
#
# All helpers are pure functions (no I/O, no global state) ; they
# raise :class:`EfeInvalidArgumentError` on schema violations so the
# caller can surface a friendly error before hitting the wire.
# ─────────────────────────────────────────────────────────────────────


def _content_dict_to_proto(content: Any, pb2: Any) -> Any:
    """Convert a YAML-parsed catalog dict to a ``CatalogContent`` proto.

    The dict shape mirrors :class:`efe_tool.AnchorCatalog`'s
    ``apiVersion: efe.mode1/v1`` body (cf. ``efe_tool/catalog.py``):

    .. code-block:: yaml

        embedder:
          family: sentence-transformers
          model: paraphrase-multilingual-MiniLM-L12-v2
          dim: 384
          normalize: l2
        classes:
          - name: refund
            description: Refund / billing reversal requests.
            canonical_sentences: [...]
            color: "#ef4444"
            deprecated: false
          - ...
        cone_policy:
          max_projection_distance: 0.7
        thresholds:
          confidence_high: 0.8
          confidence_medium: 0.5

    Validation invariants (D12 architecture §5.3 — register-time
    schema gate ; the server re-checks all of these):

      * ``content`` MUST be a dict.
      * ``classes`` MUST have ≥ 2 entries (a 1-class catalog is
        degenerate ; the disambiguation rests on having ≥ 2
        candidate cones).
      * ``embedder`` MUST carry ``family``, ``model``, ``dim``,
        ``normalize``.
      * ``thresholds`` MUST have ``confidence_high`` ≥
        ``confidence_medium`` (server re-validates ; SDK is best-
        effort).

    Args:
        content: YAML-parsed dict.
        pb2: the ``catalog_pb2`` module (passed in to avoid a
            top-level import that would force grpc-tools at SDK-
            import time).

    Returns:
        ``catalog_pb2.CatalogContent`` proto.

    Raises:
        EfeInvalidArgumentError: schema violation.
    """
    if not isinstance(content, dict):
        raise EfeInvalidArgumentError(
            f"content must be a dict ; got {type(content).__name__}"
        )

    embedder_d = content.get("embedder") or {}
    if not isinstance(embedder_d, dict):
        raise EfeInvalidArgumentError(
            "content.embedder must be a dict (family/model/dim/normalize)"
        )
    classes_list = content.get("classes") or []
    if not isinstance(classes_list, list):
        raise EfeInvalidArgumentError(
            "content.classes must be a list of class dicts"
        )
    if len(classes_list) < 2:
        raise EfeInvalidArgumentError(
            f"content.classes must have ≥ 2 entries (got {len(classes_list)})"
        )

    cone_d = content.get("cone_policy") or {}
    thresh_d = content.get("thresholds") or {}
    if not isinstance(thresh_d, dict):
        raise EfeInvalidArgumentError(
            "content.thresholds must be a dict "
            "(confidence_high / confidence_medium)"
        )

    # Threshold sanity — match `policy.rs::validate_thresholds` :
    #   confidence_medium ∈ (0, 1)
    #   confidence_high ∈ (0, 1]
    #   confidence_high ≥ confidence_medium
    # The server re-validates (authoritative) ; the SDK fails fast
    # so the caller doesn't pay an RTT for an obvious mistake.
    ch = float(thresh_d.get("confidence_high", 0.0))
    cm = float(thresh_d.get("confidence_medium", 0.0))
    if not (0.0 < cm < 1.0):
        raise EfeInvalidArgumentError(
            f"thresholds.confidence_medium ({cm}) must be in (0, 1)"
        )
    if not (0.0 < ch <= 1.0):
        raise EfeInvalidArgumentError(
            f"thresholds.confidence_high ({ch}) must be in (0, 1]"
        )
    if ch < cm:
        raise EfeInvalidArgumentError(
            f"thresholds.confidence_high ({ch}) must be ≥ "
            f"confidence_medium ({cm})"
        )

    return pb2.CatalogContent(
        embedder=pb2.EmbedderSpec(
            family=str(embedder_d.get("family", "")),
            model=str(embedder_d.get("model", "")),
            dim=int(embedder_d.get("dim", 0)),
            normalize=str(embedder_d.get("normalize", "")),
        ),
        classes=[_class_dict_to_proto(c, pb2) for c in classes_list],
        cone_policy=pb2.ConePolicy(
            max_projection_distance=float(
                cone_d.get("max_projection_distance", 0.0),
            ),
        ),
        thresholds=pb2.ThresholdPolicy(
            confidence_high=ch,
            confidence_medium=cm,
        ),
    )


def _class_dict_to_proto(c: Any, pb2: Any) -> Any:
    """Convert a single class dict to a ``CatalogClass`` proto.

    Validation invariants (mirror of ``policy.rs::validate_class``) :

      * ``c`` is a dict.
      * ``c["name"]`` is non-empty after strip().
      * ``c["canonical_sentences"]`` is a non-empty list.
      * Every sentence in ``canonical_sentences`` is a non-empty
        string after strip() — empty / whitespace-only entries
        would be silently encoded as anchors that the embedder
        cannot meaningfully embed.
    """
    if not isinstance(c, dict):
        raise EfeInvalidArgumentError(
            f"class entry must be a dict ; got {type(c).__name__}"
        )
    name = str(c.get("name", "")).strip()
    if not name:
        raise EfeInvalidArgumentError("class.name must be non-empty")
    sentences = c.get("canonical_sentences") or []
    if not isinstance(sentences, list) or not sentences:
        raise EfeInvalidArgumentError(
            f"class {name!r}.canonical_sentences must be a non-empty list"
        )
    coerced: list[str] = []
    for i, s in enumerate(sentences):
        # Reject None / non-string-coercible silently.
        if s is None:
            raise EfeInvalidArgumentError(
                f"class {name!r}.canonical_sentences[{i}] is None"
            )
        s_str = str(s)
        if not s_str.strip():
            raise EfeInvalidArgumentError(
                f"class {name!r}.canonical_sentences[{i}] is "
                f"empty or whitespace-only"
            )
        coerced.append(s_str)
    return pb2.CatalogClass(
        name=name,
        description=str(c.get("description", "")),
        canonical_sentences=coerced,
        color=str(c.get("color", "")),
        deprecated=bool(c.get("deprecated", False)),
    )


def _evolve_op_to_proto(op: Any, payload: Any, pb2: Any) -> Any:
    """Convert ``(EvolveOp enum + payload dict)`` → ``CatalogEvolutionOp``.

    Payload shapes per op (cf. ``protos/efe/v1/catalog.proto`` § 8.1):

      * APPEND_CLASS::

            {"cls": {<CatalogClass dict>}}

        ``"class"`` is also accepted as an alias for ``"cls"`` —
        useful when round-tripping a YAML payload that uses the
        natural English word.

      * REMOVE_CLASS::

            {"class_name": str}

      * UPDATE_THRESHOLDS::

            {"thresholds": {"confidence_high": float,
                            "confidence_medium": float}}

      * REPLACE_CLASS_ANCHORS::

            {"class_name": str,
             "canonical_sentences": [str, ...]}

      * RECOMPILE::

            {"new_embedder": {"family": ..., "model": ...,
                              "dim": ..., "normalize": ...}}

    Args:
        op: an :class:`efe_tool.admin.types.EvolveOp` enum variant.
        payload: per-op parameters (see shapes above).
        pb2: the ``catalog_pb2`` module.

    Returns:
        ``catalog_pb2.CatalogEvolutionOp`` proto with the right
        oneof variant set.

    Raises:
        EfeInvalidArgumentError: unknown op or malformed payload.
    """
    from .types import EvolveOp

    if not isinstance(op, EvolveOp):
        raise EfeInvalidArgumentError(
            f"op must be an EvolveOp ; got {type(op).__name__}"
        )
    payload = payload or {}
    if not isinstance(payload, dict):
        raise EfeInvalidArgumentError(
            f"op_payload must be a dict ; got {type(payload).__name__}"
        )

    if op is EvolveOp.APPEND_CLASS:
        cls_dict = payload.get("cls") or payload.get("class") or {}
        return pb2.CatalogEvolutionOp(
            append_class=pb2.AppendClassOp(
                cls=_class_dict_to_proto(cls_dict, pb2),
            ),
        )

    if op is EvolveOp.REMOVE_CLASS:
        class_name = str(payload.get("class_name", "")).strip()
        if not class_name:
            raise EfeInvalidArgumentError(
                "REMOVE_CLASS payload must have a non-empty 'class_name'"
            )
        return pb2.CatalogEvolutionOp(
            remove_class=pb2.RemoveClassOp(class_name=class_name),
        )

    if op is EvolveOp.UPDATE_THRESHOLDS:
        t_dict = payload.get("thresholds") or {}
        if not isinstance(t_dict, dict):
            raise EfeInvalidArgumentError(
                "UPDATE_THRESHOLDS payload must have a 'thresholds' dict"
            )
        ch = float(t_dict.get("confidence_high", 0.0))
        cm = float(t_dict.get("confidence_medium", 0.0))
        if not (0.0 < cm < 1.0):
            raise EfeInvalidArgumentError(
                f"thresholds.confidence_medium ({cm}) must be in (0, 1)"
            )
        if not (0.0 < ch <= 1.0):
            raise EfeInvalidArgumentError(
                f"thresholds.confidence_high ({ch}) must be in (0, 1]"
            )
        if ch < cm:
            raise EfeInvalidArgumentError(
                f"thresholds.confidence_high ({ch}) must be ≥ "
                f"confidence_medium ({cm})"
            )
        return pb2.CatalogEvolutionOp(
            update_thresholds=pb2.UpdateThresholdsOp(
                thresholds=pb2.ThresholdPolicy(
                    confidence_high=ch,
                    confidence_medium=cm,
                ),
            ),
        )

    if op is EvolveOp.REPLACE_CLASS_ANCHORS:
        class_name = str(payload.get("class_name", "")).strip()
        sentences = payload.get("canonical_sentences") or []
        if not class_name:
            raise EfeInvalidArgumentError(
                "REPLACE_CLASS_ANCHORS requires 'class_name'"
            )
        if not isinstance(sentences, list) or not sentences:
            raise EfeInvalidArgumentError(
                "REPLACE_CLASS_ANCHORS requires a non-empty "
                "'canonical_sentences' list"
            )
        return pb2.CatalogEvolutionOp(
            replace_class_anchors=pb2.ReplaceClassAnchorsOp(
                class_name=class_name,
                canonical_sentences=[str(s) for s in sentences],
            ),
        )

    if op is EvolveOp.RECOMPILE:
        emb = payload.get("new_embedder") or {}
        if not isinstance(emb, dict):
            raise EfeInvalidArgumentError(
                "RECOMPILE payload must have a 'new_embedder' dict"
            )
        return pb2.CatalogEvolutionOp(
            recompile=pb2.RecompileOp(
                new_embedder=pb2.EmbedderSpec(
                    family=str(emb.get("family", "")),
                    model=str(emb.get("model", "")),
                    dim=int(emb.get("dim", 0)),
                    normalize=str(emb.get("normalize", "")),
                ),
            ),
        )

    # Defensive — should never hit because EvolveOp is enum-bounded.
    raise EfeInvalidArgumentError(  # pragma: no cover
        f"unknown EvolveOp variant: {op!r}"
    )


def _proto_canonical_to_registered(
    canonical: Any, key: str, pb2: Any,
) -> Any:
    """Convert ``CanonicalCatalog`` proto + canonical key → dataclass.

    Reverse of :func:`_content_dict_to_proto` for the ``content``
    sub-message ; also unpacks audit metadata + optional compiled
    refs.

    Args:
        canonical: ``catalog_pb2.CanonicalCatalog`` proto.
        key: canonical key ``"tenant/name@vN"`` (parsed for
            ``tenant`` / ``name`` / ``version`` fields on the
            dataclass).
        pb2: the ``catalog_pb2`` module (currently unused, kept for
            symmetry / future use).

    Returns:
        :class:`efe_tool.admin.types.RegisteredCatalog`.

    Raises:
        EfeInvalidArgumentError: malformed key.
    """
    del pb2  # unused — kept for API symmetry
    from .types import (
        CompiledClassRef,
        RegisteredCatalog,
    )

    tenant, name, version = _parse_canonical_key(key)
    content_d = _content_proto_to_dict(canonical.content)
    metadata = _metadata_proto_to_dataclass(canonical.metadata)
    compiled: list[CompiledClassRef] | None = None
    compiled_msgs = list(canonical.compiled)
    if compiled_msgs:
        compiled = [
            CompiledClassRef(
                class_name=c.class_name,
                num_anchors=int(c.num_anchors),
                embedder_fingerprint=c.embedder_fingerprint,
            )
            for c in compiled_msgs
        ]
    return RegisteredCatalog(
        key=key,
        tenant=tenant,
        name=name,
        version=version,
        content=content_d,
        metadata=metadata,
        compiled=compiled,
    )


def _proto_handle_to_registered(handle: Any, pb2: Any) -> Any:
    """Convert ``CatalogHandle`` proto → :class:`RegisteredCatalog`.

    A ``CatalogHandle`` is the wrapper returned by
    ``RegisterCatalog`` / ``EvolveCatalog`` ; it carries the
    canonical key + the full ``CanonicalCatalog`` body.
    """
    return _proto_canonical_to_registered(handle.catalog, handle.key, pb2)


def _content_proto_to_dict(content: Any) -> dict[str, Any]:
    """Reverse of :func:`_content_dict_to_proto`.

    Returns the YAML-parseable dict shape that a caller can feed
    back into :class:`efe_tool.AnchorCatalog` if they want to use
    Mode-1 client-side classification against a server-registered
    catalog.
    """
    return {
        "embedder": {
            "family": content.embedder.family,
            "model": content.embedder.model,
            "dim": int(content.embedder.dim),
            "normalize": content.embedder.normalize,
        },
        "classes": [
            {
                "name": c.name,
                "description": c.description,
                "canonical_sentences": list(c.canonical_sentences),
                "color": c.color,
                "deprecated": bool(c.deprecated),
            }
            for c in content.classes
        ],
        "cone_policy": {
            "max_projection_distance": float(
                content.cone_policy.max_projection_distance,
            ),
        },
        "thresholds": {
            "confidence_high": float(content.thresholds.confidence_high),
            "confidence_medium": float(content.thresholds.confidence_medium),
        },
    }


def _metadata_proto_to_dataclass(meta: Any) -> Any:
    """Convert a ``CatalogMetadata`` proto → dataclass.

    Handles the proto3 quirk that ``created_at`` is a message
    field (so ``HasField`` works) ; falls back to a sentinel epoch
    datetime when unset (which would indicate a server bug — the
    server stamps every register/evolve).
    """
    from datetime import datetime, timezone

    from .types import CatalogAuditMetadata

    if meta.HasField("created_at"):
        # ToDatetime() returns a naive datetime in UTC. Re-stamp
        # tzinfo so downstream code can do tz-aware math without
        # surprises.
        created_at = meta.created_at.ToDatetime().replace(tzinfo=timezone.utc)
    else:
        # Defensive — a server that doesn't stamp created_at is
        # buggy. Use epoch sentinel rather than raising so callers
        # can still introspect the rest of the metadata.
        created_at = datetime.fromtimestamp(0, tz=timezone.utc)

    return CatalogAuditMetadata(
        created_at=created_at,
        created_by=meta.created_by,
        description=meta.description,
        parent_key=meta.parent_key,
        evolution_op=meta.evolution_op,
        labels=dict(meta.labels),
        content_sha256=meta.content_sha256,
    )


def _parse_canonical_key(key: str) -> tuple[str, str, int]:
    """Parse ``"tenant/name@vN"`` → ``(tenant, name, N)``.

    The 'v' prefix is OPTIONAL — both ``"acme/billing@v3"`` and
    ``"acme/billing@3"`` parse to ``("acme", "billing", 3)``.

    Raises:
        EfeInvalidArgumentError: not in the expected form, or the
            version segment isn't an integer.
    """
    if not isinstance(key, str) or not key:
        raise EfeInvalidArgumentError(
            f"key must be a non-empty string ; got {key!r}"
        )
    if "/" not in key or "@" not in key:
        raise EfeInvalidArgumentError(
            f"key {key!r} is not in 'tenant/name@vN' form"
        )
    tenant, rest = key.split("/", 1)
    if "@" not in rest:
        raise EfeInvalidArgumentError(
            f"key {key!r} is missing the '@vN' segment"
        )
    name, version_str = rest.rsplit("@", 1)
    if not tenant or not name or not version_str:
        raise EfeInvalidArgumentError(
            f"key {key!r} has an empty tenant / name / version segment"
        )
    if version_str.startswith("v") or version_str.startswith("V"):
        version_str = version_str[1:]
    try:
        version = int(version_str)
    except ValueError as exc:
        raise EfeInvalidArgumentError(
            f"version segment of key {key!r} is not an integer: "
            f"{version_str!r}"
        ) from exc
    if version < 1:
        raise EfeInvalidArgumentError(
            f"version of key {key!r} must be ≥ 1 ; got {version}"
        )
    return tenant, name, version

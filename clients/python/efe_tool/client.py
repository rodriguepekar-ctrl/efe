"""High-level Mode-1 client.

Orchestrates: embedder → cache → gRPC Execute → nearest-anchor
decoding → threshold policy → :class:`ClassifyResult`.

Usage:
    >>> client = EfeClient.from_env()
    >>> catalog = AnchorCatalog.load("catalogs/support_triage.yaml",
    ...                              embedder=client.embedder)
    >>> result = client.classify("j'ai été débité sans raison", catalog)
    >>> result.label, result.confidence
    ('fraud', 0.91)
"""

from __future__ import annotations

import logging
import os
import time
import uuid
from typing import Any

import numpy as np

from .cache import CacheKey, LRUCache
from .catalog import AnchorCatalog
from .embedder import Embedder, LocalSTEmbedder, fingerprint_of_text_hash
from .errors import BridgeError, ConfigError, NetworkError
from .spec_builder import build_execute_request
from .types import ClassifyResult, SubScore

_log = logging.getLogger("efe_tool.client")


class EfeClient:
    """Thread-safe client for the EFE gRPC server.

    Construct once at start-up and reuse across requests. The embedder,
    gRPC channel and cache are all long-lived.
    """

    def __init__(
        self,
        endpoint: str,
        *,
        embedder: Embedder,
        jwt: str | None = None,
        tenant: str | None = None,
        timeout_s: float = 5.0,
        cache_size: int = 10_000,
        cache_ttl_s: float = 3600.0,
        insecure: bool = True,
    ) -> None:
        if not endpoint:
            raise ConfigError("EfeClient: endpoint is required (e.g. '127.0.0.1:50051')")
        self._endpoint = endpoint
        self._embedder = embedder
        self._jwt = jwt
        self._tenant = tenant
        self._timeout_s = float(timeout_s)
        self._insecure = insecure
        self._cache = LRUCache(max_entries=cache_size, ttl_s=cache_ttl_s)

        # Channel + stub are created lazily so import-time stays cheap
        # and retrying on transient bootstrap failures is easy.
        self._channel: Any | None = None
        self._stub: Any | None = None

    # ------------------------------------------------------------------
    # Construction helpers
    # ------------------------------------------------------------------

    @classmethod
    def from_env(cls) -> "EfeClient":
        """Build a client from ``EFE_*`` environment variables.

        Variables read:
            EFE_ENDPOINT          host:port (required)
            EFE_JWT               bearer token (optional)
            EFE_TENANT            tenant id (optional)
            EFE_TIMEOUT_S         per-call timeout in seconds (default 5.0)
            EFE_EMBEDDER_MODEL    sentence-transformers model name
            EFE_INSECURE          "1" / "0" (default 1)
        """
        endpoint = os.environ.get("EFE_ENDPOINT")
        if not endpoint:
            raise ConfigError("EFE_ENDPOINT is required")
        return cls(
            endpoint=endpoint,
            embedder=LocalSTEmbedder.from_env(),
            jwt=os.environ.get("EFE_JWT") or None,
            tenant=os.environ.get("EFE_TENANT") or None,
            timeout_s=float(os.environ.get("EFE_TIMEOUT_S", "5.0")),
            insecure=os.environ.get("EFE_INSECURE", "1") != "0",
        )

    @property
    def embedder(self) -> Embedder:
        """Expose the embedder so callers can load catalogs with it."""
        return self._embedder

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    def _ensure_stub(self) -> Any:
        if self._stub is not None:
            return self._stub
        try:
            import grpc  # type: ignore
            from efe_tool._proto.efe.v1 import efe_pb2_grpc  # type: ignore
        except ImportError as e:  # pragma: no cover
            raise ConfigError(
                "grpc stubs missing. Run `clients/python/scripts/gen_protos.sh` once to "
                "generate them, and `pip install grpcio grpcio-tools`."
            ) from e
        if self._insecure:
            self._channel = grpc.insecure_channel(self._endpoint)
        else:  # pragma: no cover — we don't have TLS creds in tests
            self._channel = grpc.secure_channel(
                self._endpoint, grpc.ssl_channel_credentials()
            )
        self._stub = efe_pb2_grpc.SwarmServiceStub(self._channel)
        return self._stub

    def close(self) -> None:
        """Shut the gRPC channel."""
        if self._channel is not None:
            try:
                self._channel.close()
            except Exception:  # pragma: no cover
                pass
            self._channel = None
            self._stub = None

    def __enter__(self) -> "EfeClient":  # pragma: no cover
        return self

    def __exit__(self, *exc: object) -> None:  # pragma: no cover
        self.close()

    # ------------------------------------------------------------------
    # Health + describe (diagnostics)
    # ------------------------------------------------------------------

    def healthcheck(self) -> bool:
        """Call ``grpc.health.v1.Health/Check``; ``True`` iff SERVING."""
        try:
            import grpc  # type: ignore
            from grpc_health.v1 import health_pb2, health_pb2_grpc  # type: ignore
        except ImportError:  # pragma: no cover
            # grpcio-health-checking isn't a hard dep; we can also probe
            # via DescribeSpec round-trip. Simplest fallback:
            return self._healthcheck_via_describe()
        self._ensure_stub()
        hstub = health_pb2_grpc.HealthStub(self._channel)  # type: ignore[arg-type]
        try:
            resp = hstub.Check(
                health_pb2.HealthCheckRequest(service=""),
                timeout=self._timeout_s,
                metadata=self._grpc_metadata(),
            )
        except grpc.RpcError:  # pragma: no cover
            return False
        return resp.status == health_pb2.HealthCheckResponse.SERVING

    def _healthcheck_via_describe(self) -> bool:
        try:
            from efe_tool._proto.efe.v1 import efe_pb2, types_pb2  # type: ignore
        except ImportError:  # pragma: no cover
            return False
        stub = self._ensure_stub()
        # Build a minimal but valid spec for DescribeSpec. Unlike
        # Execute, DescribeSpec does not run the essaim; it just
        # validates the shape.
        spec = types_pb2.SwarmSpec(
            dim_latent=4,
            problem_embedding=[1.0, 0.0, 0.0, 0.0],
            population=[types_pb2.PopulationEntry(template_code="na-sem", count=1)],
            max_rounds=1,
            alignment_cone=types_pb2.AlignmentCone(
                anchors=[types_pb2.Anchor(values=[1.0, 0.0, 0.0, 0.0])],
                max_projection_distance=0.5,
            ),
        )
        try:
            stub.DescribeSpec(
                efe_pb2.DescribeSpecRequest(spec=spec),
                timeout=self._timeout_s,
                metadata=self._grpc_metadata(),
            )
            return True
        except Exception:  # pragma: no cover
            return False

    # ------------------------------------------------------------------
    # The star: classify
    # ------------------------------------------------------------------

    def classify(
        self,
        text: str,
        catalog: AnchorCatalog,
        *,
        request_id: str | None = None,
        bypass_cache: bool = False,
        return_sub_scores: bool = True,
        include_raw_prompt: bool = False,
        max_rounds: int = 16,
        budget_wall_clock_ms: int = 5_000,
    ) -> ClassifyResult:
        """Classify ``text`` against ``catalog``.

        Args:
            text: The user input. Must be a non-empty string.
            catalog: A loaded :class:`AnchorCatalog`. Its embedder
                fingerprint must match this client's embedder.
            request_id: Optional client-chosen idempotency key. When
                ``None``, a deterministic UUIDv5 derived from
                ``(catalog.version, text_hash)`` is used — so a second
                call for the same text within the server's idempotency
                window returns the cached reply.
            bypass_cache: Skip the local LRU cache. Server-side
                idempotency still applies.
            return_sub_scores: Also compute the per-class cosine scores
                (catalog.N extra cosines; trivial cost).
            include_raw_prompt: Only for dev. Leaking user text into
                audit is against MODE1_ARCHITECTURE.md §3.1.
            max_rounds: Upper bound on fusion rounds (16 is the MVP
                default; tight budgets can go as low as 8).

        Returns:
            A :class:`ClassifyResult`.

        Raises:
            EmbedderError, NetworkError, BridgeError.
        """
        if not isinstance(text, str) or not text.strip():
            raise BridgeError("classify: text must be a non-empty string")
        if catalog.embedder_fingerprint != self._embedder.fingerprint():
            raise BridgeError(
                f"embedder mismatch: client uses '{self._embedder.fingerprint()}', "
                f"catalog '{catalog.name}@{catalog.version}' was compiled with "
                f"'{catalog.embedder_fingerprint}'."
            )

        text_hash = fingerprint_of_text_hash(text)
        cache_key = CacheKey(
            catalog_id=catalog.name,
            catalog_version=catalog.version,
            text_hash=text_hash,
        )

        # 1. Local cache.
        if not bypass_cache:
            cached = self._cache.get(cache_key)
            if cached is not None:
                return cached

        # 2. Embed.
        t0 = time.perf_counter()
        vec = self._embedder.encode(text)
        t_embed = time.perf_counter() - t0

        # 3. Build request.
        rid = request_id or self._derive_request_id(catalog, text_hash)
        req = build_execute_request(
            request_id=rid,
            text=text,
            problem_embedding=vec,
            catalog=catalog,
            max_rounds=max_rounds,
            budget_wall_clock_ms=budget_wall_clock_ms,
            include_raw_prompt=include_raw_prompt,
        )

        # 4. gRPC.
        t1 = time.perf_counter()
        try:
            resp = self._do_execute(req)
        except Exception as e:
            # Map transport errors to a specific class so callers can
            # distinguish retryable issues.
            raise self._map_grpc_error(e) from e
        t_rpc = time.perf_counter() - t1

        # 5. Decode centroid and pick nearest anchor.
        payload = resp.output.WhichOneof("payload")
        if payload != "raw":
            # The server answered with a different payload kind than we
            # requested (or nothing). This happens e.g. when the swarm
            # aborts mid-flight; treat as low-conf reject rather than a
            # hard error so the LLM can fall back to asking a question.
            _log.warning(
                "unexpected decode payload: got %r, expected 'raw'", payload
            )
            return self._build_reject_result(
                catalog=catalog,
                confidence=float(resp.confidence),
                alignment_delta=float(resp.alignment_delta),
                rounds=int(resp.rounds_executed),
                swarm_id=resp.swarm_id,
                request_id=rid,
                text_hash=text_hash,
                total_ms=float(resp.total_ms),
            )
        projected = np.asarray(resp.output.raw.values, dtype=np.float32)
        if projected.shape != (catalog.dim,):
            _log.warning(
                "decode shape mismatch: got %s, expected (%d,)", projected.shape, catalog.dim
            )
            return self._build_reject_result(
                catalog=catalog,
                confidence=float(resp.confidence),
                alignment_delta=float(resp.alignment_delta),
                rounds=int(resp.rounds_executed),
                swarm_id=resp.swarm_id,
                request_id=rid,
                text_hash=text_hash,
                total_ms=float(resp.total_ms),
            )

        label, top_cos, sub_scores = self._decode_nearest_anchor(
            projected, catalog, compute_sub_scores=return_sub_scores
        )

        # 6. Apply threshold policy.
        zone = catalog.thresholds.zone(float(resp.confidence))
        reject = zone == "reject"

        # 7. Alignment-breach override: if the centroid travelled too
        #    far during projection we can't trust the label.
        if resp.alignment_delta > float(catalog.max_projection_distance):
            reject = True
            label = "uncertain"

        result = ClassifyResult(
            label="uncertain" if reject else label,
            confidence=float(resp.confidence),
            alignment_delta=float(resp.alignment_delta),
            rounds=int(resp.rounds_executed),
            reject=reject,
            swarm_id=resp.swarm_id,
            request_id=rid,
            text_hash=text_hash,
            catalog_version=catalog.version,
            top_cosine=float(top_cos),
            sub_scores=tuple(sub_scores) if sub_scores else None,
            total_ms=float(resp.total_ms),
            extra={
                "embed_ms": round(t_embed * 1000, 3),
                "rpc_ms": round(t_rpc * 1000, 3),
                "zone": zone,
            },
        )

        if not bypass_cache:
            self._cache.put(cache_key, result)
        return result

    # ------------------------------------------------------------------
    # Internals
    # ------------------------------------------------------------------

    def _do_execute(self, req: Any) -> Any:
        stub = self._ensure_stub()
        return stub.Execute(
            req, timeout=self._timeout_s, metadata=self._grpc_metadata()
        )

    def _grpc_metadata(self) -> list[tuple[str, str]]:
        md: list[tuple[str, str]] = []
        if self._jwt:
            md.append(("authorization", f"Bearer {self._jwt}"))
        if self._tenant:
            md.append(("x-tenant-id", self._tenant))
        return md

    def _derive_request_id(self, catalog: AnchorCatalog, text_hash: str) -> str:
        """Deterministic UUIDv5 so repeated calls hit server-side idempotency."""
        ns = uuid.UUID("1e6c5f60-8f10-4b4e-9c2e-000000efe000")
        return str(uuid.uuid5(ns, f"{catalog.name}:{catalog.version}:{text_hash}"))

    def _map_grpc_error(self, e: Exception) -> BridgeError:
        import grpc  # type: ignore

        if isinstance(e, grpc.RpcError):
            code = e.code()
            details = getattr(e, "details", lambda: "")() or ""
            # DEADLINE_EXCEEDED is ambiguous: the server emits it for
            # genuine transport timeouts AND for application-level
            # ConvergenceTimeout. We discriminate by message content.
            if code == grpc.StatusCode.DEADLINE_EXCEEDED:
                if "convergence" in details.lower():
                    return BridgeError(
                        f"swarm did not converge in time: {details} "
                        "(consider increasing max_rounds or budget_wall_clock_ms)"
                    )
                return NetworkError(f"transport deadline exceeded: {details}")
            if code == grpc.StatusCode.UNAVAILABLE:
                return NetworkError(f"server unavailable: {details}")
            if code == grpc.StatusCode.RESOURCE_EXHAUSTED:
                return BridgeError(
                    f"resource budget exhausted on the server: {details} "
                    "(consider raising budget_wall_clock_ms or "
                    "EFE_RATELIMIT_* on the server)"
                )
            return BridgeError(f"gRPC error {code.name if code else '?'}: {details}")
        return BridgeError(f"unexpected error: {e}")

    def _decode_nearest_anchor(
        self,
        projected: np.ndarray,
        catalog: AnchorCatalog,
        *,
        compute_sub_scores: bool,
    ) -> tuple[str, float, list[SubScore] | None]:
        """Cosine-compare against every anchor; return the best label + all scores."""
        # Anchors are already L2-normalised. Normalise the projected
        # centroid too (defensive — the server already projects onto
        # the cone but numerics can drift).
        norm = float(np.linalg.norm(projected))
        if norm < 1e-8:
            # Degenerate centroid — treat as uncertain.
            return "uncertain", 0.0, None
        p = projected / norm
        cosines = catalog.anchors @ p
        best_idx = int(np.argmax(cosines))
        top_cos = float(cosines[best_idx])
        label = catalog.classes[best_idx].name
        sub_scores = (
            [SubScore(label=catalog.classes[i].name, cosine=float(cosines[i]))
             for i in range(len(catalog.classes))]
            if compute_sub_scores
            else None
        )
        return label, top_cos, sub_scores

    def _build_reject_result(
        self,
        *,
        catalog: AnchorCatalog,
        confidence: float,
        alignment_delta: float,
        rounds: int,
        swarm_id: str,
        request_id: str,
        text_hash: str,
        total_ms: float,
    ) -> ClassifyResult:
        return ClassifyResult(
            label="uncertain",
            confidence=confidence,
            alignment_delta=alignment_delta,
            rounds=rounds,
            reject=True,
            swarm_id=swarm_id,
            request_id=request_id,
            text_hash=text_hash,
            catalog_version=catalog.version,
            top_cosine=None,
            sub_scores=None,
            total_ms=total_ms,
        )

    # ------------------------------------------------------------------
    # Diagnostics
    # ------------------------------------------------------------------

    def cache_stats(self) -> dict[str, int]:
        """Returns cache counters (hits, misses, size, capacity)."""
        return self._cache.stats()

    def invalidate_catalog(self, catalog: AnchorCatalog) -> int:
        """Drop cache entries for a specific catalog after an evolution."""
        return self._cache.invalidate_prefix(catalog.name)

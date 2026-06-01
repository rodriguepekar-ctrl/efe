"""Shared fixtures for MCP server tests."""
from __future__ import annotations

import sys
from pathlib import Path

import pytest

# Ensure the package is importable when running pytest from the repo root.
ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from mcp_server.audit_emitter import AuditEmitter  # noqa: E402
from mcp_server.auth import AuthContext  # noqa: E402
from mcp_server.dispatcher import Dispatcher  # noqa: E402
from mcp_server.rate_limiter import RateLimiter  # noqa: E402
from mcp_server.tools import build_tool_registry  # noqa: E402


# ─────────────────────────────────────────────────────────────────────
# Fake backend (no network)
# ─────────────────────────────────────────────────────────────────────


class FakeBackend:
    """In-memory backend that mirrors the playground REST surface.

    Each method records the auth.raw_token used so tests can assert
    that the dispatcher correctly forwards the bearer.
    """

    def __init__(self) -> None:
        from mcp_server.backend_client import BackendInfo

        self.info = BackendInfo(protocol="rest", endpoint="https://fake.test")
        self.calls: list[dict] = []
        self._catalogs: dict[str, dict] = {}
        self._audit: list[dict] = []
        self.next_failure: Exception | None = None

    async def aclose(self) -> None:
        return None

    def _maybe_fail(self) -> None:
        if self.next_failure is not None:
            exc = self.next_failure
            self.next_failure = None
            raise exc

    async def classify(self, *, catalog, text, top_k, auth):  # noqa: D401, ANN001
        self._maybe_fail()
        self.calls.append({"op": "classify", "catalog": catalog, "text": text, "top_k": top_k, "token": auth.raw_token})
        if catalog not in self._catalogs:
            from mcp_server.backend_client import BackendNotFound
            raise BackendNotFound(f"catalog {catalog!r} not found", status_code=404)
        cat = self._catalogs[catalog]
        return {
            "catalog_name": catalog,
            "catalog_version": cat["version"],
            "fingerprint": cat["fingerprint"],
            "top_k": [
                {"class_name": cat["classes"][0]["name"], "score": 0.92, "above_threshold": True}
            ][:top_k],
            "embedding_norm": 1.0,
            "embedder_info": {"backend": "hash", "model_name": "fake", "dim": 384},
        }

    async def register_catalog(self, *, name, classes, auth):  # noqa: D401, ANN001
        self._maybe_fail()
        self.calls.append({"op": "register", "name": name, "classes": classes, "token": auth.raw_token})
        existing = self._catalogs.get(name)
        version = (existing["version"] + 1) if existing else 1
        self._catalogs[name] = {
            "version": version,
            "fingerprint": "f" * 64,
            "classes": classes,
        }
        return {"name": name, "version": version, "fingerprint": "f" * 64, "class_count": len(classes)}

    async def evolve_catalog(self, *, name, additional_examples_per_class, auth):  # noqa: D401, ANN001
        self._maybe_fail()
        self.calls.append(
            {
                "op": "evolve",
                "name": name,
                "additional_examples_per_class": additional_examples_per_class,
                "token": auth.raw_token,
            }
        )
        existing = self._catalogs.get(name)
        if not existing:
            from mcp_server.backend_client import BackendNotFound
            raise BackendNotFound(f"catalog {name!r} not found", status_code=404)
        existing["version"] += 1
        return {
            "name": name,
            "version": existing["version"],
            "fingerprint": "g" * 64,
            "class_count": len(existing["classes"]),
        }

    async def list_catalogs(self, *, auth):  # noqa: D401, ANN001
        self._maybe_fail()
        self.calls.append({"op": "list", "token": auth.raw_token})
        return [
            {
                "name": n,
                "latest_version": c["version"],
                "fingerprint": c["fingerprint"],
                "class_count": len(c["classes"]),
            }
            for n, c in self._catalogs.items()
        ]

    async def get_audit_log(self, *, limit, auth):  # noqa: D401, ANN001
        self._maybe_fail()
        self.calls.append({"op": "audit", "limit": limit, "token": auth.raw_token})
        return self._audit[-limit:]

    async def get_admin_quotas(self, *, auth):  # noqa: D401, ANN001
        self._maybe_fail()
        self.calls.append({"op": "quotas", "token": auth.raw_token})
        return {
            "tenant_id": "acme",
            "limits": {
                "max_names_per_tenant": 100,
                "max_versions_per_name": 1000,
                "max_classes_per_catalog": 256,
                "max_sentences_per_class": 64,
            },
            "usage": {
                "names_used": len(self._catalogs),
                "remaining_names": 100 - len(self._catalogs),
            },
        }

    # ─── M17 backend extensions ─────────────────────────────────────

    def _seed_class(self, catalog: str, classes: list[dict]) -> None:
        """Test helper to inject classes with centroids + example_embeddings."""
        import numpy as np
        ext = []
        for c in classes:
            embs = c.get("example_embeddings")
            if embs is None and c.get("examples"):
                # Make deterministic L2-normalized fake embeddings
                embs = []
                for ex in c["examples"]:
                    rng = np.random.default_rng(abs(hash(ex)) % (2**32))
                    v = rng.standard_normal(384).astype(np.float32)
                    v /= np.linalg.norm(v) + 1e-9
                    embs.append(v.tolist())
            centroid = c.get("centroid")
            if centroid is None and embs:
                arr = np.asarray(embs, dtype=np.float32)
                cmean = arr.mean(axis=0)
                cmean /= np.linalg.norm(cmean) + 1e-9
                centroid = cmean.tolist()
            ext.append({
                "name": c["name"],
                "examples": c.get("examples", []),
                "example_embeddings": embs or [],
                "centroid": centroid or [0.0] * 384,
                "confidence_threshold": c.get("confidence_threshold", 0.5),
            })
        self._catalogs[catalog] = {
            "version": (self._catalogs.get(catalog, {}).get("version", 0)) + 1,
            "fingerprint": "f" * 64,
            "classes": ext,
        }

    async def encode_batch(self, *, texts, auth):
        import numpy as np
        out = []
        for t in texts:
            rng = np.random.default_rng(abs(hash(t)) % (2**32))
            v = rng.standard_normal(384).astype(np.float32)
            v /= np.linalg.norm(v) + 1e-9
            out.append(v.tolist())
        return out

    async def _encode_single(self, *, text, auth):
        embs = await self.encode_batch(texts=[text], auth=auth)
        return {"embedding": embs[0], "dim": 384, "norm": 1.0}

    async def list_catalog_classes(self, *, catalog, auth):
        cat = self._catalogs.get(catalog)
        if not cat:
            from mcp_server.backend_client import BackendNotFound
            raise BackendNotFound(f"catalog {catalog!r} not found", status_code=404)
        return cat["classes"]

    async def get_class_examples(self, *, catalog, class_name, auth):
        classes = await self.list_catalog_classes(catalog=catalog, auth=auth)
        match = next((c for c in classes if c["name"] == class_name), None)
        if not match:
            from mcp_server.backend_client import BackendNotFound
            raise BackendNotFound(f"class {class_name!r} not found", status_code=404)
        return match

    async def get_info(self, *, auth):
        return {"contract_version": "1.1", "embedder": {"backend": "fake", "dim": 384}}

    async def explain_classification(self, *, catalog, text, predicted_class, auth):
        cat = self._catalogs.get(catalog)
        if not cat:
            from mcp_server.backend_client import BackendNotFound
            raise BackendNotFound(f"catalog {catalog!r} not found", status_code=404)
        classes = cat["classes"]
        # Score against each class' centroid
        import numpy as np
        emb = np.asarray((await self._encode_single(text=text, auth=auth))["embedding"])
        scored = []
        for c in classes:
            centroid = np.asarray(c["centroid"], dtype=np.float32)
            score = float(np.dot(emb, centroid))
            scored.append((c["name"], score, c))
        scored.sort(key=lambda x: x[1], reverse=True)
        top = scored[0]
        if predicted_class:
            # M17-A7 fix : reject unknown predicted_class instead of silently
            # falling back to top-1 (which would surprise the user).
            match = next((s for s in scored if s[0] == predicted_class), None)
            if match is None:
                from mcp_server.backend_client import BackendBadRequest
                raise BackendBadRequest(
                    f"predicted_class {predicted_class!r} not found in catalog {catalog!r}",
                    status_code=400,
                    backend_code="invalid_predicted_class",
                )
            top = match
        runner = next((s for s in scored if s[0] != top[0]), None)
        gap = top[1] - (runner[1] if runner else 0.0)
        return {
            "predicted_class": top[0],
            "score": float(top[1]),
            "above_threshold": bool(top[1] >= top[2]["confidence_threshold"]),
            "threshold": top[2]["confidence_threshold"],
            "runner_up": ({"class_name": runner[0], "score": float(runner[1]), "gap": float(gap)} if runner else None),
            "contributors": [{"example": ex, "similarity_to_input": 0.5, "weight_in_centroid": 1.0/max(1, len(top[2]["examples"]))} for ex in (top[2]["examples"][:3])],
            "ambiguity_signal": {"is_ambiguous": gap < 0.05, "reason": f"gap={gap:.4f}"},
            "threshold_sensitivity": {"decision_unchanged_in_range": [runner[1] if runner else 0.0, top[1]], "decision_changes_at": None},
        }

    async def compare_classes(self, *, catalog, class_a, class_b, auth):
        return await self._compare_local(catalog=catalog, class_a=class_a, class_b=class_b, auth=auth)

    async def _compare_local(self, *, catalog, class_a, class_b, auth):
        from mcp_server.analysis.reasoning import compare_centroids
        import numpy as np
        a = await self.get_class_examples(catalog=catalog, class_name=class_a, auth=auth)
        b = await self.get_class_examples(catalog=catalog, class_name=class_b, auth=auth)
        cmp = compare_centroids(
            class_a_name=class_a,
            class_a_centroid=np.asarray(a["centroid"], dtype=np.float32),
            class_a_examples=a.get("examples", []),
            class_a_example_embeddings=np.asarray(a.get("example_embeddings", []), dtype=np.float32) if a.get("example_embeddings") else None,
            class_b_name=class_b,
            class_b_centroid=np.asarray(b["centroid"], dtype=np.float32),
            class_b_examples=b.get("examples", []),
            class_b_example_embeddings=np.asarray(b.get("example_embeddings", []), dtype=np.float32) if b.get("example_embeddings") else None,
        )
        return {
            "catalog": catalog, "class_a": class_a, "class_b": class_b,
            "centroid_cosine": cmp.centroid_cosine,
            "centroid_distance_l2": cmp.centroid_distance_l2,
            "separation_score": cmp.separation_score,
            "ambiguity_band": cmp.ambiguity_band,
            "boundary_examples": cmp.boundary_examples,
            "recommended_action": "no_action_needed",
        }

    async def score_distribution(self, *, catalog, class_name, sample_count, texts, include_negatives, auth):
        # Build a fake distribution
        positives = [0.7, 0.75, 0.8, 0.82, 0.85, 0.88, 0.9, 0.91, 0.92, 0.93]
        negatives = [0.2, 0.25, 0.3, 0.32, 0.35, 0.38, 0.4, 0.42, 0.45, 0.48] if include_negatives else []
        from mcp_server.analysis.stats import histogram, percentiles, recommend_threshold_f1
        h = histogram(positive_scores=positives, negative_scores=negatives)
        rec = recommend_threshold_f1(positive_scores=positives, negative_scores=negatives)
        return {
            "catalog": catalog,
            "class_name": class_name,
            "histogram": {"buckets": h.buckets, "positives": h.positives, "negatives": h.negatives},
            "percentiles_positives": percentiles(positives),
            "percentiles_negatives": percentiles(negatives) if negatives else {},
            "recommended_threshold": {
                "value": rec.threshold,
                "expected_precision": rec.expected_precision,
                "expected_recall": rec.expected_recall,
                "f1": rec.f1,
            },
            "current_threshold": 0.5,
            "drift_from_recommended": round(rec.threshold - 0.5, 4),
            "positive_scores": positives,
            "negative_scores": negatives,
        }

    async def find_similar_examples(self, *, catalog, text, top_k, scope, auth):
        classes = await self.list_catalog_classes(catalog=catalog, auth=auth)
        results = []
        for c in classes:
            for ex in c.get("examples", [])[:top_k]:
                results.append({"example": ex, "class_name": c["name"], "similarity": 0.6})
        results.sort(key=lambda x: x["similarity"], reverse=True)
        return {"results": results[:top_k], "scope": scope, "embedding_norm": 1.0}

    async def diagnose_catalog(self, *, catalog, depth, include_examples_text, auth):
        classes = await self.list_catalog_classes(catalog=catalog, auth=auth)
        underrep = [
            {"class_name": c["name"], "size": len(c.get("examples", [])), "recommended_size": 5}
            for c in classes if len(c.get("examples", [])) < 5
        ]
        return {
            "catalog": catalog,
            "version": self._catalogs[catalog]["version"],
            "fingerprint": self._catalogs[catalog]["fingerprint"],
            "overall_health": "good",
            "scores": {"ambiguity": 0.85, "coverage": 0.8, "balance": 0.7, "threshold_quality": 0.85, "audit_completeness": 1.0},
            "ambiguity_matrix": [],
            "underrepresented_classes": underrep,
            "threshold_recommendations": [],
            "drift_signals": [],
            "recommended_actions": [],
        }

    async def detect_drift(self, *, catalog, version_a, version_b, auth):
        return {
            "catalog": catalog,
            "version_a": version_a or 1,
            "version_b": version_b or 2,
            "fingerprint_a": "a"*64,
            "fingerprint_b": "b"*64,
            "per_class_drift": [],
            "new_classes": [],
            "removed_classes": [],
            "overall_drift": 0.02,
            "recommendation": "Drift is negligible.",
        }

    async def audit_health_check(self, *, from_timestamp, to_timestamp, deep, auth):
        return {
            "ok": True, "events_verified": 100, "broken_at": None,
            "first_event_at": 1735000000.0, "last_event_at": 1735900000.0,
            "last_hmac": "ab12" + "0"*60,
            "verification_method": "deep" if deep else "incremental",
        }

    async def export_catalog(self, *, catalog, version, format, include_centroids, include_audit_excerpt, auth):
        cat = self._catalogs.get(catalog)
        if not cat:
            from mcp_server.backend_client import BackendNotFound
            raise BackendNotFound(f"catalog {catalog!r} not found", status_code=404)
        v = version or cat["version"]
        content = {"name": catalog, "version": v, "fingerprint": cat["fingerprint"], "classes": cat["classes"]}
        return {
            "catalog": catalog, "version": v, "format": format,
            "size_bytes": 1024, "content": content,
            "exported_at": 1735000000.0,
        }

    async def consolidate_versions(self, *, catalog, keep_last_n, dry_run, auth):
        return {
            "catalog": catalog,
            "current_versions_count": 5,
            "keep_last_n": keep_last_n,
            "to_delete_count": max(0, 5 - keep_last_n),
            "to_delete_versions": list(range(1, max(0, 5 - keep_last_n) + 1)),
            "preserved_versions": list(range(max(1, 6 - keep_last_n), 6)),
            "dry_run": dry_run,
        }

    # M18 D48 — abstract bundle methods. Default: NotImplementedError so
    # tools / resources fall back to direct OCI access.
    async def list_known_bundles(self, *, registry_filter, slug_filter, limit, auth):
        raise NotImplementedError("FakeBackend has no /admin/bundles endpoint")

    async def get_bundle_record(self, *, slug, version, auth):
        raise NotImplementedError("FakeBackend has no /admin/bundles/{slug}/{ver} endpoint")

    async def get_trust_store(self, *, auth):
        raise NotImplementedError("FakeBackend has no /admin/trust-store endpoint")

    # M18 D49 — deployment helpers. Tests opt-in by setting `_deployment_state`.
    _deployment_state: dict = None  # type: ignore[assignment]
    _deployment_history: list = None  # type: ignore[assignment]
    _deployment_supports_dry_run: bool = False

    def set_deployment_state(self, state: dict | None) -> None:
        self._deployment_state = state

    def set_deployment_history(self, history: list | None) -> None:
        self._deployment_history = history

    def enable_dry_run_endpoint(self, value: bool = True) -> None:
        self._deployment_supports_dry_run = value

    async def get_deployment_current(self, *, auth):
        if self._deployment_state is None:
            raise NotImplementedError(
                "FakeBackend : call set_deployment_state() to opt in"
            )
        self.calls.append({"op": "deployment.current", "token": auth.raw_token})
        return dict(self._deployment_state)

    async def get_deployment_history(self, *, limit, auth):
        if self._deployment_history is None:
            raise NotImplementedError(
                "FakeBackend : call set_deployment_history() to opt in"
            )
        self.calls.append({
            "op": "deployment.history", "limit": limit, "token": auth.raw_token,
        })
        return list(self._deployment_history[:limit])

    async def dry_run_deployment(
        self, *, target_slug, target_version, target_manifest_sha256,
        environment, auth,
    ):
        if not self._deployment_supports_dry_run:
            raise NotImplementedError(
                "FakeBackend : call enable_dry_run_endpoint() to opt in"
            )
        self.calls.append({
            "op": "deployment.dry_run",
            "target_slug": target_slug,
            "target_version": target_version,
            "target_manifest_sha256": target_manifest_sha256,
            "environment": environment,
            "token": auth.raw_token,
        })
        return {
            "source": "backend_mocked",
            "current": self._deployment_state or {},
            "target": {
                "bundle": {"slug": target_slug, "version": target_version},
                "manifest_sha256": target_manifest_sha256,
            },
            "diff": {"manifest_changed": True},
            "risks": [],
            "blocking_issues": [],
            "recommended_actions": ["use efe_generate_helm_values"],
            "highest_risk_level": "LOW",
            "has_blockers": False,
            "environment": environment,
        }

    # ── M18 D50 — deploy write tools mocked storage ─────────────────
    _registered_bundles: dict | None = None
    _promoted_bundle: dict | None = None
    _filed_tickets: list | None = None
    _deploy_writes_enabled: bool = False

    def enable_deploy_writes(self, value: bool = True) -> None:
        self._deploy_writes_enabled = value
        if value:
            if self._registered_bundles is None:
                self._registered_bundles = {}
            if self._filed_tickets is None:
                self._filed_tickets = []

    async def register_bundle(
        self, *, slug, version, registry_url, manifest_sha256,
        verify_before_register, idempotency_key, auth,
    ):
        if not self._deploy_writes_enabled:
            raise NotImplementedError(
                "FakeBackend : call enable_deploy_writes() to opt in"
            )
        self.calls.append({
            "op": "deploy.register_bundle",
            "slug": slug, "version": version,
            "registry_url": registry_url,
            "manifest_sha256": manifest_sha256,
            "verify_before_register": verify_before_register,
            "idempotency_key": idempotency_key,
            "token": auth.raw_token,
        })
        bundle_id = f"{registry_url.rstrip('/')}/{slug}:{version}"
        if self._registered_bundles is not None:
            self._registered_bundles[bundle_id] = {
                "slug": slug, "version": version,
                "registry_url": registry_url,
                "manifest_sha256": manifest_sha256,
                "registered_at": "2026-05-01T00:00:00Z",
            }
        return {
            "bundle_id": bundle_id,
            "manifest_sha256": manifest_sha256,
            "audit_event_id": f"evt_register_{slug}_{version}",
        }

    async def promote_bundle(
        self, *, slug, version, scope, idempotency_key, auth,
    ):
        if not self._deploy_writes_enabled:
            raise NotImplementedError(
                "FakeBackend : call enable_deploy_writes() to opt in"
            )
        self.calls.append({
            "op": "deploy.promote_bundle",
            "slug": slug, "version": version, "scope": scope,
            "idempotency_key": idempotency_key,
            "token": auth.raw_token,
        })
        previous = (self._promoted_bundle or {}).get("version")
        self._promoted_bundle = {
            "slug": slug, "version": version, "scope": scope,
        }
        return {
            "tenant_id": "acme",
            "previous_current_version": previous,
            "audit_event_id": f"evt_promote_{slug}_{version}",
        }

    async def request_deployment_ticket(
        self, *, slug, version, environment, rationale, tracker,
        idempotency_key, auth,
    ):
        if not self._deploy_writes_enabled:
            raise NotImplementedError(
                "FakeBackend : call enable_deploy_writes() to opt in"
            )
        self.calls.append({
            "op": "deploy.request_ticket",
            "slug": slug, "version": version,
            "environment": environment, "rationale": rationale,
            "tracker": tracker, "idempotency_key": idempotency_key,
            "token": auth.raw_token,
        })
        if self._filed_tickets is None:
            self._filed_tickets = []
        ticket_id = f"EFE-{len(self._filed_tickets) + 1}"
        ticket = {
            "ticket_id": ticket_id,
            "ticket_url": f"https://{tracker}.example.com/issue/{ticket_id}",
            "status": "open",
            "audit_event_id": f"evt_ticket_{ticket_id}",
        }
        self._filed_tickets.append({
            **ticket, "tracker": tracker, "slug": slug, "version": version,
        })
        return ticket


# ─────────────────────────────────────────────────────────────────────
# Fixtures
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def fake_backend() -> FakeBackend:
    return FakeBackend()


@pytest.fixture
def auth_ctx() -> AuthContext:
    return AuthContext(raw_token="playground-test-token-abcd1234")


@pytest.fixture
def registry():
    return build_tool_registry()


@pytest.fixture
def rate_limiter() -> RateLimiter:
    return RateLimiter(per_min=100)


@pytest.fixture
def audit_emitter(fake_backend) -> AuditEmitter:
    return AuditEmitter(fake_backend, enabled=False)  # disable network-side emission for tests


@pytest.fixture
def dispatcher(registry, fake_backend, auth_ctx, audit_emitter, rate_limiter) -> Dispatcher:
    return Dispatcher(
        registry=registry,
        backend=fake_backend,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
    )


# ─────────────────────────────────────────────────────────────────────
# M17 fixtures
# ─────────────────────────────────────────────────────────────────────


from mcp_server.profile import Profile  # noqa: E402
from mcp_server.sampling import SamplingClient  # noqa: E402
from mcp_server.state import EmbeddingCache, SamplingCache  # noqa: E402


@pytest.fixture
def advanced_registry():
    return build_tool_registry(Profile.ADVANCED)


@pytest.fixture
def agent_native_registry():
    return build_tool_registry(Profile.AGENT_NATIVE)


class FakeSamplingSession:
    """In-memory sampling session for tests."""

    def __init__(self) -> None:
        self.calls: list[dict] = []
        self.next_response: str = "fake-response"
        # Allow programming a per-prompt response queue
        self.responses_by_prompt: dict[str, str] = {}
        self.fail_n: int = 0

    async def create_message(self, *, messages, max_tokens, model_preferences=None):
        self.calls.append({
            "messages": messages,
            "max_tokens": max_tokens,
            "model_preferences": model_preferences,
        })
        if self.fail_n > 0:
            self.fail_n -= 1
            raise RuntimeError("simulated sampling failure")
        prompt_text = ""
        if messages and isinstance(messages[0], dict):
            content = messages[0].get("content")
            if isinstance(content, dict):
                prompt_text = content.get("text", "")
        for prefix, resp in self.responses_by_prompt.items():
            if prefix in prompt_text:
                return _FakeResponse(resp)
        return _FakeResponse(self.next_response)


class _FakeResponse:
    def __init__(self, text: str) -> None:
        self.content = _FakeContent(text)


class _FakeContent:
    def __init__(self, text: str) -> None:
        self.text = text


@pytest.fixture
def fake_sampling_session() -> FakeSamplingSession:
    return FakeSamplingSession()


@pytest.fixture
def sampling_client(fake_sampling_session) -> SamplingClient:
    return SamplingClient(
        fake_sampling_session,
        cache=SamplingCache(),
        max_rounds_per_call=5,
        timeout_seconds=2.0,
        max_tokens_per_request=500,
        client_supports_sampling=True,
    )


@pytest.fixture
def embedding_cache() -> EmbeddingCache:
    return EmbeddingCache()


@pytest.fixture
def advanced_dispatcher(
    advanced_registry, fake_backend, auth_ctx, audit_emitter, rate_limiter,
    embedding_cache,
) -> Dispatcher:
    return Dispatcher(
        registry=advanced_registry,
        backend=fake_backend,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
        profile=Profile.ADVANCED,
        sampling=None,
        embedding_cache=embedding_cache,
    )


@pytest.fixture
def agent_native_dispatcher(
    agent_native_registry, fake_backend, auth_ctx, audit_emitter, rate_limiter,
    sampling_client, embedding_cache, bundle_cache, oci_factory,
) -> Dispatcher:
    return Dispatcher(
        registry=agent_native_registry,
        backend=fake_backend,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
        profile=Profile.AGENT_NATIVE,
        sampling=sampling_client,
        embedding_cache=embedding_cache,
        bundle_cache=bundle_cache,
        oci_factory=oci_factory,
    )


# ─────────────────────────────────────────────────────────────────────
# M18 D50 — agent-native-deploy fixtures (write tools opt-in)
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def agent_native_deploy_registry():
    return build_tool_registry(Profile.AGENT_NATIVE_DEPLOY)


@pytest.fixture
def agent_native_deploy_dispatcher(
    agent_native_deploy_registry, fake_backend, auth_ctx, audit_emitter,
    rate_limiter, sampling_client, embedding_cache, bundle_cache, oci_factory,
) -> Dispatcher:
    return Dispatcher(
        registry=agent_native_deploy_registry,
        backend=fake_backend,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
        profile=Profile.AGENT_NATIVE_DEPLOY,
        sampling=sampling_client,
        embedding_cache=embedding_cache,
        bundle_cache=bundle_cache,
        oci_factory=oci_factory,
    )


# ─────────────────────────────────────────────────────────────────────
# M18 fixtures — FakeOCIRegistry + signed test bundles
# ─────────────────────────────────────────────────────────────────────


import hashlib  # noqa: E402
import json as _json  # noqa: E402

from mcp_server.bundle import OCIRegistryClient  # noqa: E402
from mcp_server.state import BundleCache  # noqa: E402


@pytest.fixture
def bundle_cache() -> BundleCache:
    return BundleCache(capacity=8, ttl_seconds=600.0)


class FakeOCIRegistry:
    """In-memory OCI registry implementing the v2 GET surface.

    Test fixtures populate it via `add_bundle()`. The
    `OCIRegistryClient` instances built by the `oci_factory` fixture
    talk to this registry via httpx.AsyncClient with a custom
    transport, so we exercise the real client code path without
    network access.
    """

    def __init__(self) -> None:
        # repo -> reference -> {raw, parsed, sha256}
        self._manifests: dict[str, dict[str, dict]] = {}
        self._blobs: dict[str, bytes] = {}                 # digest -> bytes
        self._referrers: dict[str, list[dict]] = {}        # subject_digest -> [manifests]
        self._tags: dict[str, list[str]] = {}              # repo -> tag list
        self.calls: list[dict] = []

    # ── Population helpers ──────────────────────────────────────────

    def _store_blob(self, content: bytes) -> str:
        digest = "sha256:" + hashlib.sha256(content).hexdigest()
        self._blobs[digest] = content
        return digest

    def add_bundle(
        self,
        *,
        repo: str,
        slug: str,
        version: str,
        annotations: dict | None = None,
        signature_bytes: bytes | None = None,
        signature_artifact_type: str = "application/vnd.efe.bundle.signature.ed25519+json",
        sbom_bytes: bytes | None = None,
        sbom_artifact_type: str = "application/vnd.cyclonedx+json",
        slsa_attestation_bytes: bytes | None = None,
        slsa_artifact_type: str = "application/vnd.in-toto+json",
        sign_callback=None,
        layer_sizes: list[int] | None = None,
    ) -> str:
        """Add a bundle manifest + (optional) signature & SBOM referrers.

        Audit fix M18-D47-A5 (LOW) : `sign_callback`, when provided,
        is called with the canonical manifest bytes AFTER they're
        finalised, returning the signature blob bytes. This avoids the
        previous double-add+pop dance that fixtures had to do to sign
        the actual manifest content.

        Audit fix M18-D47-A2 (MED) : `layer_sizes` lets fixtures supply
        realistic OCI layer sizes so tests of `efe_diff_bundles` can
        assert non-fictitious size diffs.

        Returns the manifest sha256 (hex).
        """
        ann = dict(annotations or {})
        ann.setdefault("org.efe.bundle.slug", slug)
        ann.setdefault("org.efe.bundle.version", version)
        ann.setdefault("org.efe.bundle.framework", "hash")
        ann.setdefault("org.efe.bundle.dim", "384")
        ann.setdefault("org.efe.bundle.schema_version", "1.0")
        ann.setdefault("org.efe.bundle.is_dev_unsigned", "false")

        layers_field: list[dict[str, object]] = []
        if layer_sizes:
            for i, size in enumerate(layer_sizes):
                layers_field.append({
                    "mediaType": "application/vnd.efe.bundle.layer.tar+gzip",
                    "digest": "sha256:" + (f"{i:02x}" * 32),
                    "size": int(size),
                })
        manifest = {
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.efe.bundle.config+json",
                "digest": "sha256:" + ("0" * 64),
                "size": 0,
            },
            "layers": layers_field,
            "annotations": ann,
        }
        raw = _json.dumps(manifest, sort_keys=True).encode("utf-8")
        manifest_sha = hashlib.sha256(raw).hexdigest()
        # Audit fix M18-D47-A5 : if a sign_callback was supplied, sign
        # the canonical manifest bytes now and use the resulting
        # signature blob.
        if sign_callback is not None and signature_bytes is None:
            signature_bytes = sign_callback(raw)
        reference = f"{slug}-{version}"
        self._manifests.setdefault(repo, {})[reference] = {
            "raw": raw,
            "parsed": manifest,
            "sha256": manifest_sha,
        }
        # Tag list
        tags = self._tags.setdefault(repo, [])
        if reference not in tags:
            tags.append(reference)

        # Signature referrer
        subject_digest = "sha256:" + manifest_sha
        if signature_bytes is not None:
            sig_blob_digest = self._store_blob(signature_bytes)
            sig_manifest = {
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.artifact.manifest.v1+json",
                "artifactType": signature_artifact_type,
                "subject": {"digest": subject_digest, "size": len(raw)},
                "layers": [{
                    "mediaType": signature_artifact_type,
                    "digest": sig_blob_digest,
                    "size": len(signature_bytes),
                }],
            }
            self._referrers.setdefault(subject_digest, []).append(sig_manifest)

        # SBOM referrer
        if sbom_bytes is not None:
            sbom_blob_digest = self._store_blob(sbom_bytes)
            sbom_manifest = {
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.artifact.manifest.v1+json",
                "artifactType": sbom_artifact_type,
                "subject": {"digest": subject_digest, "size": len(raw)},
                "layers": [{
                    "mediaType": sbom_artifact_type,
                    "digest": sbom_blob_digest,
                    "size": len(sbom_bytes),
                }],
            }
            self._referrers.setdefault(subject_digest, []).append(sbom_manifest)

        # M18 D48 — SLSA provenance referrer
        if slsa_attestation_bytes is not None:
            slsa_blob_digest = self._store_blob(slsa_attestation_bytes)
            slsa_manifest = {
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.artifact.manifest.v1+json",
                "artifactType": slsa_artifact_type,
                "subject": {"digest": subject_digest, "size": len(raw)},
                "layers": [{
                    "mediaType": slsa_artifact_type,
                    "digest": slsa_blob_digest,
                    "size": len(slsa_attestation_bytes),
                }],
            }
            self._referrers.setdefault(subject_digest, []).append(slsa_manifest)

        return manifest_sha

    # ── HTTP-like dispatch ──────────────────────────────────────────

    def handle_request(self, method: str, path: str, params: dict | None) -> dict:
        """Return a dict suitable for httpx.MockTransport."""
        self.calls.append({"method": method, "path": path, "params": params})
        if method != "GET":
            return {"status_code": 405, "content": b"", "headers": {}}

        # /v2/<repo>/manifests/<reference>
        if "/manifests/" in path:
            _, _, repo_with_rest = path.partition("/v2/")
            repo, _, rest = repo_with_rest.partition("/manifests/")
            entry = self._manifests.get(repo, {}).get(rest)
            if entry is None:
                return {"status_code": 404, "content": b"", "headers": {}}
            return {
                "status_code": 200,
                "content": entry["raw"],
                "headers": {
                    "content-type": "application/vnd.oci.image.manifest.v1+json",
                },
            }

        # /v2/<repo>/blobs/<digest>
        if "/blobs/" in path:
            _, _, repo_with_rest = path.partition("/v2/")
            _, _, digest = repo_with_rest.partition("/blobs/")
            blob = self._blobs.get(digest)
            if blob is None:
                return {"status_code": 404, "content": b"", "headers": {}}
            return {
                "status_code": 200,
                "content": blob,
                "headers": {"content-type": "application/octet-stream"},
            }

        # /v2/<repo>/referrers/<digest>
        if "/referrers/" in path:
            _, _, repo_with_rest = path.partition("/v2/")
            _, _, subject = repo_with_rest.partition("/referrers/")
            referrers = list(self._referrers.get(subject, []))
            artifact_type = (params or {}).get("artifactType")
            if artifact_type:
                referrers = [
                    r for r in referrers
                    if r.get("artifactType") == artifact_type
                ]
            payload = {"manifests": referrers}
            return {
                "status_code": 200,
                "content": _json.dumps(payload).encode("utf-8"),
                "headers": {"content-type": "application/json"},
            }

        # /v2/<repo>/tags/list
        if path.endswith("/tags/list"):
            _, _, repo_with_rest = path.partition("/v2/")
            repo, _, _ = repo_with_rest.partition("/tags/list")
            payload = {"name": repo, "tags": list(self._tags.get(repo, []))}
            return {
                "status_code": 200,
                "content": _json.dumps(payload).encode("utf-8"),
                "headers": {"content-type": "application/json"},
            }

        return {"status_code": 404, "content": b"", "headers": {}}


@pytest.fixture
def fake_oci_registry() -> FakeOCIRegistry:
    return FakeOCIRegistry()


def _make_oci_factory(registry: FakeOCIRegistry):
    """Return a factory that builds OCIRegistryClient instances backed
    by the in-memory FakeOCIRegistry via httpx.MockTransport."""

    import httpx

    def _handler(request: httpx.Request) -> httpx.Response:
        result = registry.handle_request(
            request.method,
            request.url.path,
            dict(request.url.params),
        )
        return httpx.Response(
            status_code=result["status_code"],
            content=result["content"],
            headers=result["headers"],
        )

    transport = httpx.MockTransport(_handler)

    def factory(host: str) -> OCIRegistryClient:
        # Allowlist contains the host so the client builds successfully.
        # We keep the allowlist permissive (just the host part) and let
        # the OCI client's per-request check enforce the full path.
        client = OCIRegistryClient(
            host,
            allowlist=(host,),
        )
        # Audit fix M18-D47-A6 (LOW) : close the original httpx
        # AsyncClient before replacing it with the MockTransport one.
        # Pre-fix, the original was leaked on every fixture call.
        original = client._client
        try:
            # Best-effort sync teardown for the in-fixture replacement.
            # The original client never made a network request, so
            # close() is fast.
            import asyncio as _asyncio
            try:
                loop = _asyncio.get_event_loop()
                if loop.is_running():
                    # In-event-loop fixture : schedule the close.
                    loop.create_task(original.aclose())
                else:
                    loop.run_until_complete(original.aclose())
            except RuntimeError:
                # No running loop ; create a one-shot loop to close.
                _asyncio.new_event_loop().run_until_complete(original.aclose())
        except Exception:  # noqa: BLE001 — defensive; close is best-effort
            pass
        client._client = httpx.AsyncClient(
            base_url=client._url,
            transport=transport,
            timeout=10.0,
        )
        return client

    return factory


@pytest.fixture
def oci_factory(fake_oci_registry):
    return _make_oci_factory(fake_oci_registry)


# ─────────────────────────────────────────────────────────────────────
# Ed25519 test keypair helpers
# ─────────────────────────────────────────────────────────────────────


# ─────────────────────────────────────────────────────────────────────
# M18 D48 — SLSA test attestation builder
# ─────────────────────────────────────────────────────────────────────


def make_slsa_attestation_bytes(
    *,
    manifest_sha256: str,
    builder_id: str = "https://github.com/actions/runner/v2.319.0",
    build_type: str = (
        "https://slsa-framework.github.io/github-actions-buildtypes/workflow/v1"
    ),
    invocation_id: str = "test-run-123",
    started_on: str = "2026-04-15T10:30:00Z",
    finished_on: str = "2026-04-15T10:31:00Z",
    subject_name: str = "test-bundle",
    subject_digest: str | None = None,
) -> bytes:
    """Produce a deterministic SLSA Provenance v1.0 in-toto Statement.

    Useful for tests that need to populate the FakeOCIRegistry with a
    realistic attestation referrer.
    """
    digest = subject_digest if subject_digest is not None else manifest_sha256
    payload = {
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [
            {
                "name": subject_name,
                "digest": {"sha256": digest},
            }
        ],
        "predicateType": "https://slsa.dev/provenance/v1",
        "predicate": {
            "buildDefinition": {
                "buildType": build_type,
                "externalParameters": {},
                "internalParameters": {},
                "resolvedDependencies": [],
            },
            "runDetails": {
                "builder": {"id": builder_id, "version": {}},
                "metadata": {
                    "invocationId": invocation_id,
                    "startedOn": started_on,
                    "finishedOn": finished_on,
                },
                "byproducts": [],
            },
        },
    }
    return _json.dumps(payload).encode("utf-8")


@pytest.fixture
def slsa_attestation_factory():
    """Returns the make_slsa_attestation_bytes helper."""
    return make_slsa_attestation_bytes


@pytest.fixture
def ed25519_keypair():
    """Generate a fresh Ed25519 keypair for crypto tests."""
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import (
        Ed25519PrivateKey,
    )

    private_key = Ed25519PrivateKey.generate()
    public_key = private_key.public_key()
    public_pem = public_key.public_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PublicFormat.SubjectPublicKeyInfo,
    )

    def sign(data: bytes) -> bytes:
        return private_key.sign(data)

    return {
        "sign": sign,
        "public_pem": public_pem,
        "private_key": private_key,
        "public_key": public_key,
    }


# ─────────────────────────────────────────────────────────────────────
# M20 fixtures — feature-flag-gated dispatchers
# ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def m20_env(monkeypatch):
    """Activate the M20 helper-tools env-var for a test."""
    monkeypatch.setenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", "true")
    yield


@pytest.fixture
def m20_registry(m20_env):
    """Tool registry with M20 helpers enabled (default OFF otherwise)."""
    return build_tool_registry(Profile.AGENT_NATIVE)


@pytest.fixture
def m20_dispatcher(
    m20_registry,
    fake_backend,
    auth_ctx,
    audit_emitter,
    rate_limiter,
    sampling_client,
    embedding_cache,
    bundle_cache,
    oci_factory,
) -> Dispatcher:
    """Dispatcher in agent-native profile WITH M20 flag ON."""
    return Dispatcher(
        registry=m20_registry,
        backend=fake_backend,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
        profile=Profile.AGENT_NATIVE,
        sampling=sampling_client,
        embedding_cache=embedding_cache,
        bundle_cache=bundle_cache,
        oci_factory=oci_factory,
    )


@pytest.fixture
def m20_off_registry(monkeypatch):
    """Tool registry with M20 flag explicitly OFF (default behaviour)."""
    monkeypatch.delenv("EFE_FEATURE_M20_HELPER_TOOLS_ENABLED", raising=False)
    return build_tool_registry(Profile.AGENT_NATIVE)


@pytest.fixture
def m20_off_dispatcher(
    m20_off_registry,
    fake_backend,
    auth_ctx,
    audit_emitter,
    rate_limiter,
    sampling_client,
    embedding_cache,
    bundle_cache,
    oci_factory,
) -> Dispatcher:
    """Dispatcher with M20 explicitly OFF."""
    return Dispatcher(
        registry=m20_off_registry,
        backend=fake_backend,
        auth=auth_ctx,
        audit=audit_emitter,
        rate_limiter=rate_limiter,
        profile=Profile.AGENT_NATIVE,
        sampling=sampling_client,
        embedding_cache=embedding_cache,
        bundle_cache=bundle_cache,
        oci_factory=oci_factory,
    )


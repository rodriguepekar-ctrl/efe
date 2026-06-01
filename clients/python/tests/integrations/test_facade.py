"""Comprehensive tests for :class:`EfeFacade` using stubbed dependencies.

Does NOT require torch, grpcio or a running server.
"""

from __future__ import annotations

import asyncio
import threading
import time
from types import SimpleNamespace

import pytest

from efe_tool.integrations import (
    EfeConfig,
    EfeFacade,
    EfeFacadeCatalogError,
    EfeFacadeClosedError,
    EfeFacadeError,
    EfeFacadeInitError,
    EfeFacadeRejectedError,
    EfeFacadeTimeoutError,
    EfeFacadeTransientError,
    EfeFacadeUsageError,
    Verdict,
    request_id_var,
)


# ---------------------------------------------------------------------------
#  Classify happy path
# ---------------------------------------------------------------------------


class TestClassify:
    def test_returns_verdict(self, facade_with_stubs):
        v = facade_with_stubs.classify("hello", catalog="support_triage")
        assert isinstance(v, Verdict)
        assert v.label == "alpha"
        assert v.catalog == "support_triage"

    def test_records_stats(self, facade_with_stubs):
        facade_with_stubs.classify("x", catalog="support_triage")
        facade_with_stubs.classify("y", catalog="support_triage")
        s = facade_with_stubs.stats()
        assert s.classify_total == 2
        assert s.classify_failed == 0

    def test_request_id_propagates(self, facade_with_stubs):
        v = facade_with_stubs.classify(
            "x", catalog="support_triage", request_id="rid-1",
        )
        assert v.request_id == "rid-1"

    def test_request_id_from_contextvar(self, facade_with_stubs):
        token = request_id_var.set("rid-ctx")
        try:
            v = facade_with_stubs.classify("x", catalog="support_triage")
            assert v.request_id == "rid-ctx"
        finally:
            request_id_var.reset(token)

    def test_empty_text_raises_usage(self, facade_with_stubs):
        with pytest.raises(EfeFacadeUsageError):
            facade_with_stubs.classify("   ", catalog="support_triage")

    def test_non_string_text_raises_usage(self, facade_with_stubs):
        with pytest.raises(EfeFacadeUsageError):
            facade_with_stubs.classify(42, catalog="support_triage")  # type: ignore[arg-type]

    def test_unknown_catalog_raises_usage(self, facade_with_stubs):
        with pytest.raises(EfeFacadeUsageError) as ei:
            facade_with_stubs.classify("x", catalog="unknown")
        assert "unknown catalog" in ei.value.message.lower()

    def test_oversized_text_raises_usage(self, facade_with_stubs):
        big = "x" * (facade_with_stubs.config.max_text_length + 1)
        with pytest.raises(EfeFacadeUsageError):
            facade_with_stubs.classify(big, catalog="support_triage")


# ---------------------------------------------------------------------------
#  Strict mode
# ---------------------------------------------------------------------------


class TestStrictMode:
    def test_strict_raises_on_reject(self, facade_with_stubs):
        # Swap the stub client to return reject=True.
        facade_with_stubs._client._result = SimpleNamespace(
            label="uncertain", confidence=0.2, reject=True, rounds=5,
            alignment_delta=0.3, extra={"zone": "reject"},
        )
        with pytest.raises(EfeFacadeRejectedError):
            facade_with_stubs.classify("x", catalog="support_triage",
                                        strict=True)

    def test_strict_passes_on_trusted(self, facade_with_stubs):
        # Default stub is confidence=0.85, no reject — strict should pass.
        v = facade_with_stubs.classify("x", catalog="support_triage",
                                        strict=True)
        assert v.label == "alpha"


# ---------------------------------------------------------------------------
#  Bridge error mapping
# ---------------------------------------------------------------------------


class TestBridgeErrorMapping:
    def test_catalog_error_surfaces_as_facade_catalog(self, facade_with_stubs):
        from efe_tool.errors import CatalogError
        facade_with_stubs._client._raise = CatalogError("bad schema")
        with pytest.raises(EfeFacadeCatalogError):
            facade_with_stubs.classify("x", catalog="support_triage")

    def test_network_error_surfaces_as_transient(self, facade_with_stubs):
        from efe_tool.errors import NetworkError as BN
        facade_with_stubs._client._raise = BN("unavailable")
        with pytest.raises(EfeFacadeTransientError):
            facade_with_stubs.classify("x", catalog="support_triage")

    def test_timeout_surfaces_as_timeout(self, facade_with_stubs):
        from efe_tool.errors import NetworkError as BN
        facade_with_stubs._client._raise = BN("transport deadline exceeded")
        with pytest.raises(EfeFacadeTimeoutError):
            facade_with_stubs.classify("x", catalog="support_triage")

    def test_unexpected_error_wraps_cleanly(self, facade_with_stubs):
        facade_with_stubs._client._raise = RuntimeError("???")
        with pytest.raises(EfeFacadeError):
            facade_with_stubs.classify("x", catalog="support_triage")


# ---------------------------------------------------------------------------
#  Lifecycle
# ---------------------------------------------------------------------------


class TestLifecycle:
    def test_double_close_is_idempotent(self, facade_with_stubs):
        facade_with_stubs.close()
        facade_with_stubs.close()
        assert facade_with_stubs.is_closed

    def test_classify_after_close_raises(self, facade_with_stubs):
        facade_with_stubs.close()
        with pytest.raises(EfeFacadeClosedError):
            facade_with_stubs.classify("x", catalog="support_triage")

    def test_preload_without_real_deps_fails(self, default_config):
        """Without any stubbing, preload() tries to import torch.

        We can't exercise the happy path here, but we verify the
        exception type is typed.
        """
        facade = EfeFacade(default_config)
        try:
            facade.preload()
        except EfeFacadeError as e:
            # Either EfeFacadeInitError or EfeFacadeCatalogError is acceptable;
            # what matters is no raw ImportError leaks.
            assert e.error_code in {"EFE_INIT_FAILED", "EFE_CATALOG_INVALID",
                                    "EFE_BUG"}
        else:
            facade.close()


# ---------------------------------------------------------------------------
#  Healthcheck
# ---------------------------------------------------------------------------


class TestHealthcheck:
    def test_serving(self, facade_with_stubs):
        hs = facade_with_stubs.healthcheck()
        assert hs.status == "SERVING"

    def test_not_serving_after_close(self, facade_with_stubs):
        facade_with_stubs.close()
        hs = facade_with_stubs.healthcheck()
        assert hs.status == "NOT_SERVING"


# ---------------------------------------------------------------------------
#  Async
# ---------------------------------------------------------------------------


class TestAsync:
    def test_classify_async_returns_verdict(self, facade_with_stubs):
        async def _run():
            return await facade_with_stubs.classify_async(
                "x", catalog="support_triage",
            )
        v = asyncio.run(_run())
        assert isinstance(v, Verdict)
        assert v.label == "alpha"

    def test_classify_async_preserves_exceptions(self, facade_with_stubs):
        from efe_tool.errors import NetworkError as BN
        facade_with_stubs._client._raise = BN("down")

        async def _run():
            return await facade_with_stubs.classify_async(
                "x", catalog="support_triage",
            )
        with pytest.raises(EfeFacadeTransientError):
            asyncio.run(_run())


# ---------------------------------------------------------------------------
#  Concurrency
# ---------------------------------------------------------------------------


class TestConcurrency:
    def test_parallel_classify_no_crash(self, facade_with_stubs):
        errors: list[Exception] = []

        def worker(n: int):
            try:
                for _ in range(20):
                    facade_with_stubs.classify(
                        f"msg {n}", catalog="support_triage",
                    )
            except Exception as e:  # pragma: no cover
                errors.append(e)

        threads = [threading.Thread(target=worker, args=(i,)) for i in range(8)]
        for t in threads: t.start()
        for t in threads: t.join()

        assert errors == []
        assert facade_with_stubs.stats().classify_total == 8 * 20


# ---------------------------------------------------------------------------
#  Reload catalog
# ---------------------------------------------------------------------------


class TestReloadCatalog:
    def test_reload_unknown_raises(self, facade_with_stubs):
        with pytest.raises(EfeFacadeUsageError):
            facade_with_stubs.reload_catalog("does_not_exist")

    def test_reload_without_source_path_raises_catalog(self, facade_with_stubs):
        """The stub catalog's source_path is None → reload must raise
        :class:`EfeFacadeCatalogError` rather than crash on None."""
        from efe_tool.integrations import EfeFacadeCatalogError
        with pytest.raises(EfeFacadeCatalogError):
            facade_with_stubs.reload_catalog("support_triage")


class TestClassifyTimeoutS:
    """Regression: ``timeout_s`` parameter is currently accepted but
    documented as not honoured. Must validate it's positive and not
    crash on None."""

    def test_none_is_fine(self, facade_with_stubs):
        v = facade_with_stubs.classify(
            "x", catalog="support_triage", timeout_s=None,
        )
        assert v.label == "alpha"

    def test_positive_value_accepted(self, facade_with_stubs):
        v = facade_with_stubs.classify(
            "x", catalog="support_triage", timeout_s=3.5,
        )
        assert v.label == "alpha"

    def test_negative_raises_usage(self, facade_with_stubs):
        with pytest.raises(EfeFacadeUsageError):
            facade_with_stubs.classify(
                "x", catalog="support_triage", timeout_s=-1,
            )

    def test_zero_raises_usage(self, facade_with_stubs):
        with pytest.raises(EfeFacadeUsageError):
            facade_with_stubs.classify(
                "x", catalog="support_triage", timeout_s=0,
            )


class TestPreloadAtomicity:
    """Regression: a partial catalog load must not leave
    :attr:`EfeFacade._catalogs` half-populated. If one of N catalogs
    fails, none of them should be committed."""

    def test_partial_load_leaves_no_catalogs(self, default_config, stub_embedder):
        """Simulate: cfg declares two catalogs; the first loads, the
        second fails. After preload() raises, _catalogs must stay empty.
        """
        from types import SimpleNamespace
        cfg = default_config.with_override(catalogs=("a", "b"))
        facade = EfeFacade(cfg)
        facade._embedder = stub_embedder

        # Stub _resolve_catalog_path / AnchorCatalog.load.
        facade._resolve_catalog_path = (  # type: ignore[assignment]
            lambda name: f"/fake/{name}.yaml"
        )

        call_count = {"n": 0}

        def fake_load(path, *, embedder=None, **kw):
            call_count["n"] += 1
            if call_count["n"] == 1:
                return SimpleNamespace(name="a", source_path=str(path))
            raise RuntimeError("second catalog explodes")

        import efe_tool.catalog as cat_mod
        import pytest as _pytest
        orig_load = cat_mod.AnchorCatalog.load
        cat_mod.AnchorCatalog.load = staticmethod(fake_load)  # type: ignore
        try:
            with _pytest.raises(Exception):
                facade._init_catalogs()
            # Must be empty — partial commit is a data-consistency bug.
            assert facade._catalogs == {}
        finally:
            cat_mod.AnchorCatalog.load = orig_load  # type: ignore


# ---------------------------------------------------------------------------
#  Stats window
# ---------------------------------------------------------------------------


class TestStats:
    def test_p50_p95_populated(self, facade_with_stubs):
        for _ in range(20):
            facade_with_stubs.classify("x", catalog="support_triage")
        s = facade_with_stubs.stats()
        assert s.latency_p50_ms_last_window >= 0
        assert s.latency_p95_ms_last_window >= s.latency_p50_ms_last_window


# ---------------------------------------------------------------------------
#  Ensure-ready guard
# ---------------------------------------------------------------------------


class TestEnsureReady:
    def test_classify_before_preload_raises(self, default_config):
        facade = EfeFacade(default_config)
        with pytest.raises(EfeFacadeInitError):
            facade.classify("x", catalog="support_triage")

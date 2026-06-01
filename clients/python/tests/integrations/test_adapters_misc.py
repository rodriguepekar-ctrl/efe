"""Smoke tests for framework adapters that don't need the framework.

We verify the modules import cleanly and their public symbols exist.
Deeper framework-specific behaviour is covered by per-framework tests
(test_fastapi.py, test_django.py, …) gated on framework install.
"""

from __future__ import annotations

import pytest


class TestImports:
    def test_fastapi_module_imports(self):
        from efe_tool.integrations import fastapi as adapter
        assert hasattr(adapter, "efe_lifespan")
        assert hasattr(adapter, "get_efe")
        assert hasattr(adapter, "register_efe_exception_handlers")

    def test_django_module_imports(self):
        from efe_tool.integrations import django as adapter
        assert hasattr(adapter, "EfeAppConfig")
        assert hasattr(adapter, "get_config_from_settings")
        assert hasattr(adapter, "EfeMiddleware")

    def test_flask_module_imports(self):
        from efe_tool.integrations import flask as adapter
        assert hasattr(adapter, "init_efe")
        assert hasattr(adapter, "get_efe")

    def test_celery_module_imports(self):
        from efe_tool.integrations import celery as adapter
        assert hasattr(adapter, "setup_efe_for_worker")
        assert hasattr(adapter, "shutdown_efe")

    def test_notebook_module_imports(self):
        from efe_tool.integrations import notebook as adapter
        assert hasattr(adapter, "efe_notebook_singleton")

    def test_dagster_module_imports(self):
        from efe_tool.integrations import dagster as adapter
        assert hasattr(adapter, "efe_resource")

    def test_testing_module_imports(self):
        from efe_tool.integrations.testing import (
            FakeEfeFacade, make_verdict,
        )
        assert FakeEfeFacade is not None
        assert make_verdict is not None


class TestCeleryWithoutInit:
    def test_get_efe_without_setup_raises(self):
        from efe_tool.integrations import celery as adapter
        from efe_tool.integrations import EfeFacadeError
        adapter.reset_for_test()
        with pytest.raises(EfeFacadeError):
            adapter.get_efe()


class TestDjangoMiddlewareShape:
    def test_middleware_is_callable(self):
        from efe_tool.integrations.django import EfeMiddleware
        m = EfeMiddleware(lambda request: "response")
        # Calling through is fine without Django installed (we pass a str).
        assert m(None) == "response"

    def test_process_exception_passthrough(self):
        from efe_tool.integrations.django import EfeMiddleware
        m = EfeMiddleware(lambda request: None)
        # Non-EFE errors are not handled
        assert m.process_exception(None, RuntimeError("x")) is None


class TestDjangoMixin:
    """Regression: `EfeAppReadyMixin` provides a clean multiple-inheritance
    pattern instead of the fragile `__init_subclass__` trick that
    monkey-patched `__bases__` at runtime."""

    def test_mixin_class_exists_and_has_ready(self):
        from efe_tool.integrations.django import EfeAppReadyMixin
        assert hasattr(EfeAppReadyMixin, "ready")

    def test_setup_for_django_exists(self):
        from efe_tool.integrations.django import setup_for_django
        assert callable(setup_for_django)

    def test_efe_app_config_attribute_always_present(self):
        """`EfeAppConfig` must be accessible even when Django is missing
        (via a stub class). Instantiation behaviour varies by Django
        availability."""
        import efe_tool.integrations.django as m
        assert hasattr(m, "EfeAppConfig")

    def test_efe_app_config_instantiation_requires_django(self):
        """When Django is not installed, instantiating the stub raises
        a clear :class:`ImportError`. When Django IS installed, a
        subclass works and chains the ready() mixin."""
        import efe_tool.integrations.django as m
        try:
            import django.apps  # noqa: F401
        except ImportError:
            import pytest
            with pytest.raises(ImportError):
                m.EfeAppConfig()
        else:
            assert issubclass(m.EfeAppConfig, django.apps.AppConfig)  # noqa
            assert issubclass(m.EfeAppConfig, m.EfeAppReadyMixin)


class TestPreloadLeak:
    """Regression: every adapter that preloads the facade must close
    it on failure, so a retry doesn't leak gRPC channels."""

    def _failing_config(self):
        """A config that fails at preload (unknown catalog path)."""
        from efe_tool.integrations import EfeConfig
        return EfeConfig(
            endpoint="127.0.0.1:50051",
            catalogs=("does_not_exist_anywhere",),
            catalog_search_paths=("/tmp/__nope__",),
        )

    def test_notebook_releases_on_preload_failure(self):
        import pytest
        from efe_tool.integrations import notebook
        notebook.reset_for_test()
        with pytest.raises(Exception):
            notebook.efe_notebook_singleton(config=self._failing_config())
        # Singleton should NOT have been stored
        assert notebook._FACADE is None

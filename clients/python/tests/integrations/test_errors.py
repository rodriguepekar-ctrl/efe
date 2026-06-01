"""Tests for :mod:`efe_tool.integrations._errors`."""

from __future__ import annotations

import pytest

from efe_tool.integrations import (
    FACADE_EXIT_CODES,
    EfeFacadeCatalogError,
    EfeFacadeClosedError,
    EfeFacadeConfigError,
    EfeFacadeError,
    EfeFacadeInitError,
    EfeFacadeRejectedError,
    EfeFacadeTimeoutError,
    EfeFacadeTransientError,
    EfeFacadeUsageError,
    from_bridge_error,
    from_cli_error,
)


class TestStatusCodes:
    @pytest.mark.parametrize("cls,http", [
        (EfeFacadeConfigError, 500),
        (EfeFacadeInitError, 500),
        (EfeFacadeClosedError, 503),
        (EfeFacadeCatalogError, 500),
        (EfeFacadeUsageError, 422),
        (EfeFacadeRejectedError, 422),
        (EfeFacadeTransientError, 503),
        (EfeFacadeTimeoutError, 504),
    ])
    def test_mapping(self, cls, http):
        assert cls.suggested_http_status == http

    def test_base_defaults_to_500(self):
        assert EfeFacadeError.suggested_http_status == 500
        assert EfeFacadeError.error_code == "EFE_BUG"

    def test_registry_has_all(self):
        keys = set(FACADE_EXIT_CODES.keys())
        assert "EFE_USAGE" in keys and "EFE_TIMEOUT" in keys
        assert FACADE_EXIT_CODES["EFE_TIMEOUT"] == 504


class TestRetryable:
    def test_transient_is_retryable(self):
        assert EfeFacadeTransientError("x").retryable is True

    def test_timeout_is_retryable(self):
        assert EfeFacadeTimeoutError("x").retryable is True

    def test_usage_not_retryable(self):
        assert EfeFacadeUsageError("x").retryable is False

    def test_config_not_retryable(self):
        assert EfeFacadeConfigError("x").retryable is False


class TestPayload:
    def test_minimal(self):
        e = EfeFacadeUsageError("bad input")
        p = e.to_payload()
        assert p["error"] == "EFE_USAGE"
        assert p["http_status"] == 422
        assert p["retryable"] is False
        assert p["message"] == "bad input"
        assert "hint" not in p
        assert "context" not in p

    def test_with_hint_and_context(self):
        e = EfeFacadeUsageError(
            "bad", hint="try X", context={"catalog": "foo"},
        )
        p = e.to_payload()
        assert p["hint"] == "try X"
        assert p["context"] == {"catalog": "foo"}

    def test_with_cause(self):
        src = ValueError("underlying")
        e = EfeFacadeInitError("boom", cause=src)
        p = e.to_payload()
        assert p["cause"]["type"] == "ValueError"
        assert "underlying" in p["cause"]["detail"]

    def test_cause_truncated(self):
        src = ValueError("x" * 1000)
        e = EfeFacadeError("init", cause=src)
        assert len(e.to_payload()["cause"]["detail"]) <= 500


class TestFromCliError:
    def test_timeout(self):
        from efe_tool.cli.errors import CliTimeoutError
        out = from_cli_error(CliTimeoutError("deadline exceeded"))
        assert isinstance(out, EfeFacadeTimeoutError)

    def test_network(self):
        from efe_tool.cli.errors import NetworkError
        out = from_cli_error(NetworkError("unreachable"))
        assert isinstance(out, EfeFacadeTransientError)

    def test_server_error(self):
        from efe_tool.cli.errors import ServerError
        out = from_cli_error(ServerError("internal"))
        assert isinstance(out, EfeFacadeTransientError)

    def test_catalog_not_found(self):
        from efe_tool.cli.errors import CatalogNotFoundError
        out = from_cli_error(CatalogNotFoundError("missing"))
        assert isinstance(out, EfeFacadeCatalogError)

    def test_catalog_invalid(self):
        from efe_tool.cli.errors import CatalogInvalidError
        out = from_cli_error(CatalogInvalidError("bad"))
        assert isinstance(out, EfeFacadeCatalogError)

    def test_usage(self):
        from efe_tool.cli.errors import UsageError
        out = from_cli_error(UsageError("wrong"))
        assert isinstance(out, EfeFacadeUsageError)

    def test_generic(self):
        out = from_cli_error(RuntimeError("unknown"))
        assert isinstance(out, EfeFacadeError)


class TestFromBridgeError:
    def test_catalog(self):
        from efe_tool.errors import CatalogError
        out = from_bridge_error(CatalogError("bad yaml"))
        assert isinstance(out, EfeFacadeCatalogError)

    def test_config(self):
        from efe_tool.errors import ConfigError
        out = from_bridge_error(ConfigError("missing env"))
        assert isinstance(out, EfeFacadeConfigError)

    def test_embedder(self):
        from efe_tool.errors import EmbedderError
        out = from_bridge_error(EmbedderError("torch missing"))
        assert isinstance(out, EfeFacadeInitError)

    def test_network_deadline(self):
        from efe_tool.errors import NetworkError as BridgeNet
        out = from_bridge_error(BridgeNet("transport deadline exceeded"))
        assert isinstance(out, EfeFacadeTimeoutError)

    def test_network_unavailable(self):
        from efe_tool.errors import NetworkError as BridgeNet
        out = from_bridge_error(BridgeNet("unreachable"))
        assert isinstance(out, EfeFacadeTransientError)

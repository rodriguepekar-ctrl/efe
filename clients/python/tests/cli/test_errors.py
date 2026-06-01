"""Tests for :mod:`efe_tool.cli.errors`.

Validate the exit-code taxonomy, the BridgeError adapter, and the
``to_payload`` contract.
"""

from __future__ import annotations

import pytest

from efe_tool.cli.errors import (
    EXIT_CODE_TABLE,
    CatalogInvalidError,
    CatalogNotFoundError,
    CliError,
    CliTimeoutError,
    InterruptedError,
    LowConfidenceError,
    NetworkError,
    RejectedError,
    ServerError,
    UsageError,
    from_bridge_error,
)


class TestTaxonomyUnique:
    """No two CLI exceptions may share the same exit code."""

    @pytest.mark.parametrize("cls,expected_code", [
        (UsageError, 2),
        (CatalogNotFoundError, 3),
        (CatalogInvalidError, 4),
        (NetworkError, 5),
        (ServerError, 6),
        (CliTimeoutError, 7),
        (RejectedError, 8),
        (LowConfidenceError, 9),
        (InterruptedError, 130),
    ])
    def test_exit_code(self, cls: type[CliError], expected_code: int) -> None:
        assert cls.exit_code == expected_code

    def test_base_cliError_is_bug(self) -> None:
        assert CliError.exit_code == 126
        assert CliError.error_code == "BUG"

    def test_exit_codes_are_disjoint(self) -> None:
        codes = [
            UsageError.exit_code,
            CatalogNotFoundError.exit_code,
            CatalogInvalidError.exit_code,
            NetworkError.exit_code,
            ServerError.exit_code,
            CliTimeoutError.exit_code,
            RejectedError.exit_code,
            LowConfidenceError.exit_code,
            InterruptedError.exit_code,
            CliError.exit_code,
        ]
        assert len(codes) == len(set(codes))

    def test_registry_covers_all(self) -> None:
        assert 0 in EXIT_CODE_TABLE
        assert EXIT_CODE_TABLE[2] == "USAGE"
        assert EXIT_CODE_TABLE[7] == "TIMEOUT"
        assert EXIT_CODE_TABLE[126] == "BUG"
        assert EXIT_CODE_TABLE[130] == "INTERRUPTED"


class TestPayload:
    def test_minimal(self) -> None:
        err = UsageError("bad")
        payload = err.to_payload()
        assert payload == {
            "code": "USAGE",
            "exit_code": 2,
            "message": "bad",
        }

    def test_with_hint_and_context(self) -> None:
        err = UsageError("bad", hint="try --help", context={"flag": "--foo"})
        payload = err.to_payload()
        assert payload["hint"] == "try --help"
        assert payload["context"] == {"flag": "--foo"}

    def test_with_cause(self) -> None:
        src = ValueError("underlying")
        err = CliTimeoutError("timed out", cause=src)
        payload = err.to_payload()
        assert payload["cause"]["type"] == "ValueError"
        assert "underlying" in payload["cause"]["detail"]

    def test_cause_truncated(self) -> None:
        src = ValueError("x" * 1000)
        err = CatalogInvalidError("broken", cause=src)
        assert len(err.to_payload()["cause"]["detail"]) <= 500


class TestFromBridgeError:
    def test_catalog_error_maps_to_invalid(self) -> None:
        from efe_tool.errors import CatalogError
        cli_err = from_bridge_error(CatalogError("schema bad"))
        assert isinstance(cli_err, CatalogInvalidError)
        assert cli_err.exit_code == 4

    def test_config_error_maps_to_usage(self) -> None:
        from efe_tool.errors import ConfigError
        cli_err = from_bridge_error(ConfigError("env missing"))
        assert isinstance(cli_err, UsageError)

    def test_embedder_error_maps_to_invalid(self) -> None:
        from efe_tool.errors import EmbedderError
        cli_err = from_bridge_error(EmbedderError("model load failed"))
        assert isinstance(cli_err, CatalogInvalidError)

    def test_bridge_network_error_timeout(self) -> None:
        from efe_tool.errors import NetworkError as BridgeNet
        cli_err = from_bridge_error(BridgeNet("transport deadline exceeded"))
        assert isinstance(cli_err, CliTimeoutError)

    def test_bridge_network_error_other(self) -> None:
        from efe_tool.errors import NetworkError as BridgeNet
        cli_err = from_bridge_error(BridgeNet("server unavailable"))
        assert isinstance(cli_err, NetworkError)

    def test_bridge_convergence_timeout(self) -> None:
        from efe_tool.errors import BridgeError
        cli_err = from_bridge_error(
            BridgeError("swarm did not converge in time")
        )
        assert isinstance(cli_err, CliTimeoutError)

    def test_bridge_resource_exhausted(self) -> None:
        from efe_tool.errors import BridgeError
        cli_err = from_bridge_error(
            BridgeError("resource budget exhausted on the server")
        )
        assert isinstance(cli_err, CliTimeoutError)

    def test_bridge_generic(self) -> None:
        from efe_tool.errors import BridgeError
        cli_err = from_bridge_error(BridgeError("weird"))
        assert isinstance(cli_err, ServerError)

    def test_unknown_exception(self) -> None:
        cli_err = from_bridge_error(ZeroDivisionError("nope"))
        assert isinstance(cli_err, CliError)
        assert cli_err.exit_code == 126

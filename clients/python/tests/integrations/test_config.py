"""Tests for :mod:`efe_tool.integrations._config`."""

from __future__ import annotations

import pytest

from efe_tool.integrations import EfeConfig, EfeFacadeConfigError
from efe_tool.integrations._config import (
    DEFAULT_EMBEDDER,
    DEFAULT_ENDPOINT,
)


class TestDefaults:
    def test_default_values(self):
        cfg = EfeConfig()
        assert cfg.endpoint == DEFAULT_ENDPOINT
        assert cfg.embedder_model == DEFAULT_EMBEDDER
        assert cfg.classify_budget_ms == 5000
        assert cfg.max_rounds == 16
        assert cfg.metrics_enabled is True


class TestValidation:
    def test_no_scheme_allowed(self):
        with pytest.raises(EfeFacadeConfigError):
            EfeConfig(endpoint="http://host:50051")

    def test_port_required(self):
        with pytest.raises(EfeFacadeConfigError):
            EfeConfig(endpoint="host-without-port")

    def test_negative_budget(self):
        with pytest.raises(EfeFacadeConfigError):
            EfeConfig(classify_budget_ms=-1)

    def test_zero_timeout(self):
        with pytest.raises(EfeFacadeConfigError):
            EfeConfig(request_timeout_s=0)

    def test_max_rounds_bounds(self):
        with pytest.raises(EfeFacadeConfigError):
            EfeConfig(max_rounds=0)
        with pytest.raises(EfeFacadeConfigError):
            EfeConfig(max_rounds=1000)

    def test_invalid_catalog_name(self):
        with pytest.raises(EfeFacadeConfigError):
            EfeConfig(catalogs=("Bad Name!",))

    def test_valid_catalog_names(self):
        cfg = EfeConfig(catalogs=("support_triage", "legal-v2", "abc123"))
        assert len(cfg.catalogs) == 3


class TestEffectiveTimeout:
    def test_raises_below_budget_floor(self):
        cfg = EfeConfig(
            request_timeout_s=1.0, classify_budget_ms=5_000,
        )
        # floor = 5.0 + 0.5 = 5.5 → effective should be 5.5
        assert cfg.effective_request_timeout_s == pytest.approx(5.5)

    def test_preserves_when_above_floor(self):
        cfg = EfeConfig(
            request_timeout_s=10.0, classify_budget_ms=5_000,
        )
        assert cfg.effective_request_timeout_s == 10.0


class TestFromEnv:
    def test_empty_env_yields_defaults(self):
        cfg = EfeConfig.from_env(env={})
        assert cfg.endpoint == DEFAULT_ENDPOINT
        assert cfg.catalogs == ()

    def test_endpoint(self):
        cfg = EfeConfig.from_env(env={"EFE_ENDPOINT": "foo:9000"})
        assert cfg.endpoint == "foo:9000"

    def test_catalogs_split(self):
        cfg = EfeConfig.from_env(env={"EFE_CATALOGS": "a,b, c "})
        assert cfg.catalogs == ("a", "b", "c")

    def test_catalog_search_paths_split(self):
        cfg = EfeConfig.from_env(
            env={"EFE_CATALOG_SEARCH_PATHS": "/tmp/a:/tmp/b"},
        )
        assert cfg.catalog_search_paths == ("/tmp/a", "/tmp/b")

    def test_bool_env_true(self):
        for v in ("1", "true", "yes", "on"):
            cfg = EfeConfig.from_env(env={"EFE_TLS": v})
            assert cfg.tls is True

    def test_bool_env_false(self):
        for v in ("0", "false", "no", "off", ""):
            cfg = EfeConfig.from_env(env={"EFE_TLS": v})
            assert cfg.tls is False

    def test_int_fields(self):
        cfg = EfeConfig.from_env(env={
            "EFE_CLASSIFY_BUDGET_MS": "7000",
            "EFE_MAX_ROUNDS": "8",
            "EFE_ASYNC_EXECUTOR_MAX_WORKERS": "16",
        })
        assert cfg.classify_budget_ms == 7000
        assert cfg.max_rounds == 8
        assert cfg.async_executor_max_workers == 16

    def test_float_timeout(self):
        cfg = EfeConfig.from_env(env={"EFE_REQUEST_TIMEOUT_S": "30.5"})
        assert cfg.request_timeout_s == 30.5


class TestFromDict:
    def test_basic(self):
        cfg = EfeConfig.from_dict({
            "endpoint": "foo:1", "catalogs": ["a", "b"],
        })
        assert cfg.endpoint == "foo:1"
        assert cfg.catalogs == ("a", "b")

    def test_uppercase_keys_accepted(self):
        cfg = EfeConfig.from_dict({
            "ENDPOINT": "foo:1", "CATALOGS": ["a"],
        })
        assert cfg.endpoint == "foo:1"

    def test_unknown_keys_go_to_extra(self):
        cfg = EfeConfig.from_dict({
            "endpoint": "foo:1", "CUSTOM_KEY": "x",
        })
        assert cfg.extra == {"CUSTOM_KEY": "x"}


class TestOverride:
    def test_with_override(self):
        cfg = EfeConfig()
        new = cfg.with_override(classify_budget_ms=9999)
        assert new.classify_budget_ms == 9999
        assert cfg.classify_budget_ms == 5000  # original untouched


class TestToDict:
    """Regression: ``EfeConfig.to_dict()`` must produce a JSON-safe
    mapping with tuples flattened to lists, so Flask / Django adapters
    can merge it with their own config dicts."""

    def test_returns_dict(self):
        d = EfeConfig().to_dict()
        assert isinstance(d, dict)

    def test_tuples_become_lists(self):
        cfg = EfeConfig(
            catalogs=("a", "b"),
            catalog_search_paths=("/tmp/x", "/tmp/y"),
        )
        d = cfg.to_dict()
        assert d["catalogs"] == ["a", "b"]
        assert d["catalog_search_paths"] == ["/tmp/x", "/tmp/y"]

    def test_roundtrip_via_from_dict(self):
        original = EfeConfig(
            endpoint="h:1", classify_budget_ms=2500, max_rounds=12,
            catalogs=("support_triage",),
        )
        round_tripped = EfeConfig.from_dict(original.to_dict())
        assert round_tripped.endpoint == "h:1"
        assert round_tripped.classify_budget_ms == 2500
        assert round_tripped.max_rounds == 12
        assert round_tripped.catalogs == ("support_triage",)

    def test_json_serialisable(self):
        import json
        json.dumps(EfeConfig(catalogs=("x",)).to_dict())

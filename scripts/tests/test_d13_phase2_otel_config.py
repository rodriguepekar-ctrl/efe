"""Sprint D13 Phase 2 — OTel SDK config validation.

Tests the env-var contract documented in
``crates/efe-telemetry/src/otel/mod.rs``. We don't compile the
Rust here ; we just text-grep the file to confirm the env var
names match what's documented in the architecture + values.yaml.
"""

from __future__ import annotations

from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_RUST = _REPO / "crates" / "efe-telemetry" / "src" / "otel" / "mod.rs"


EXPECTED_ENV_VARS = (
    "EFE_OTEL_ENABLED",
    "EFE_OTEL_ENDPOINT",
    "EFE_OTEL_SAMPLE_RATE",
    "EFE_OTEL_FORCE_SAMPLE_ON_ERROR",
    "EFE_OTEL_SERVICE_NAME",
    "EFE_OTEL_SERVICE_VERSION",
)


def test_all_otel_env_vars_documented_in_rust():
    text = _RUST.read_text(encoding="utf-8")
    for var in EXPECTED_ENV_VARS:
        assert var in text, f"OTel module missing env var {var!r}"


def test_default_sample_rate_is_5_percent():
    text = _RUST.read_text(encoding="utf-8")
    assert "sample_rate: 0.05" in text, (
        "default sample rate is meant to be 0.05 (= 5 %)"
    )


def test_default_force_sample_on_error_is_true():
    text = _RUST.read_text(encoding="utf-8")
    assert "force_sample_on_error: true" in text, (
        "default force_sample_on_error is meant to be true (catch errors)"
    )


def test_default_service_name_is_efe_server():
    text = _RUST.read_text(encoding="utf-8")
    assert '"efe-server".to_owned()' in text, (
        "default service_name is meant to be 'efe-server'"
    )


def test_init_returns_sdk_not_implemented_in_v0_1_13():
    """v0.1.13 ships with init as a no-op (when disabled) and
    returns SdkNotImplemented (when enabled). This pin catches a
    premature D13-bis activation."""
    text = _RUST.read_text(encoding="utf-8")
    assert "SdkNotImplemented" in text, (
        "OtelInitError::SdkNotImplemented variant must exist"
    )


def test_values_yaml_otel_block_present():
    p = _REPO / "deploy" / "helm" / "efe" / "values.yaml"
    text = p.read_text(encoding="utf-8")
    assert "otel:" in text
    assert "sampleRate" in text
    assert "forceSampleOnError" in text
    assert "tempo:" in text


def test_otel_collector_config_exists_and_has_allowlist():
    p = _REPO / "deploy" / "observability" / "otel-collector" / "config.yaml"
    assert p.exists()
    text = p.read_text(encoding="utf-8")
    # PII-blocking attribute deletes
    for forbidden in ("text", "email", "jwt", "token", "password",
                      "api_key", "phone", "address"):
        assert f"key: {forbidden}" in text, (
            f"otel-collector config missing delete for forbidden attr "
            f"{forbidden!r}"
        )

"""Sprint **D13-bis** — OpenTelemetry SDK real implementation pin tests.

Cross-layer guards :

* The OTel `init` no longer returns `SdkNotImplemented` from the
  enabled branch.
* The 4 D13-bis Prometheus instruments are registered in the same
  `metrics::mod` re-export pattern as the other D-X sprints.
* `register_all_d13bis()` invokes every instrument constructor.
* The Helm `config.observability.otel` block has the documented
  fields + the configmap renders the matching `EFE_OTEL_*` env
  vars.
* `cargo` deps add the 5 OTel crates with version 0.27 / 0.28
  pinned (architecture § 3 + R-13bis-1).
* Resource builder includes `service.name` / `service.version` /
  `service.namespace = efe`.
* `ErrorBoostingSampler` exists and overrides `Drop` decisions.
* `MeasuringSpanExporter` exists and bumps the 4 metrics.
* `OtelGuard::force_flush` exists with a `Duration` timeout.
"""

from __future__ import annotations

import re
from pathlib import Path

import yaml

_REPO = Path(__file__).resolve().parents[2]
_TELEMETRY = _REPO / "crates" / "efe-telemetry"
_OTEL_MOD = _TELEMETRY / "src" / "otel" / "mod.rs"
_RESOURCE_RS = _TELEMETRY / "src" / "otel" / "resource.rs"
_SAMPLER_RS = _TELEMETRY / "src" / "otel" / "sampler.rs"
_FILTER_RS = _TELEMETRY / "src" / "otel" / "attribute_filter.rs"
_MEASURING_RS = _TELEMETRY / "src" / "otel" / "measuring_exporter.rs"
_INSTRUMENTS_RS = _TELEMETRY / "src" / "metrics" / "d13bis_instruments.rs"
_METRICS_MOD = _TELEMETRY / "src" / "metrics" / "mod.rs"
_OBS_INIT_RS = _TELEMETRY / "src" / "observability_init_d13.rs"
_CARGO = _TELEMETRY / "Cargo.toml"
_VALUES = _REPO / "deploy" / "helm" / "efe" / "values.yaml"
_CONFIGMAP = (
    _REPO / "deploy" / "helm" / "efe" / "templates" / "configmap.yaml"
)
_NETPOL = (
    _REPO / "deploy" / "helm" / "efe" / "templates" / "networkpolicy-otel.yaml"
)


# ─────────────────────────────────────────────────────────────────────
# Cargo deps
# ─────────────────────────────────────────────────────────────────────


def test_cargo_otel_deps_pinned():
    cargo = _CARGO.read_text(encoding="utf-8")
    for crate in [
        "opentelemetry",
        "opentelemetry_sdk",
        "opentelemetry-otlp",
        "opentelemetry-semantic-conventions",
        "tracing-opentelemetry",
    ]:
        assert crate in cargo, f"{crate} missing from Cargo.toml"


def test_cargo_otel_versions_match_architecture():
    """Architecture § 3.3 pins opentelemetry 0.27 + tracing-opentelemetry 0.28."""
    cargo = _CARGO.read_text(encoding="utf-8")
    assert re.search(
        r'opentelemetry\s*=\s*\{\s*version\s*=\s*"0\.27"', cargo
    ), "opentelemetry must be 0.27"
    assert re.search(
        r'opentelemetry-otlp\s*=\s*\{\s*version\s*=\s*"0\.27"', cargo
    ), "opentelemetry-otlp must be 0.27"
    assert re.search(
        r'tracing-opentelemetry\s*=\s*\{\s*version\s*=\s*"0\.28"', cargo
    ), "tracing-opentelemetry must be 0.28"


# ─────────────────────────────────────────────────────────────────────
# init() implementation
# ─────────────────────────────────────────────────────────────────────


def test_init_no_longer_returns_sdk_not_implemented_in_enabled_branch():
    """**Architecture § 22** acceptance criterion : the enabled branch
    of `init()` returns a real Provider, not the legacy stub error."""
    rs = _OTEL_MOD.read_text(encoding="utf-8")
    init_fn = re.search(
        r"pub fn init\(.*?\n}\n", rs, re.DOTALL
    )
    assert init_fn, "init function not found"
    body = init_fn.group()
    # The enabled branch must build a TracerProvider.
    assert "TracerProvider::builder" in body, (
        "init() must build a TracerProvider in the enabled branch"
    )
    # And install a propagator.
    assert "TraceContextPropagator" in body, (
        "init() must install a TraceContextPropagator"
    )


def test_init_returns_option_otel_guard():
    """The new return shape is `Result<Option<OtelGuard>, OtelInitError>`."""
    rs = _OTEL_MOD.read_text(encoding="utf-8")
    assert (
        "Result<Option<OtelGuard>, OtelInitError>" in rs
    ), "init() signature must be Result<Option<OtelGuard>, _>"


def test_otel_guard_struct_exists_with_force_flush():
    rs = _OTEL_MOD.read_text(encoding="utf-8")
    assert "pub struct OtelGuard" in rs
    assert "pub async fn force_flush" in rs
    assert "OtelShutdownError" in rs


def test_init_validates_sample_rate_range():
    rs = _OTEL_MOD.read_text(encoding="utf-8")
    assert "InvalidSampleRate" in rs
    assert "0.0..=1.0" in rs


# ─────────────────────────────────────────────────────────────────────
# Resource builder
# ─────────────────────────────────────────────────────────────────────


def test_resource_includes_canonical_service_attrs():
    rs = _RESOURCE_RS.read_text(encoding="utf-8")
    for attr in [
        '"service.name"',
        '"service.version"',
        '"service.namespace"',
    ]:
        assert attr in rs, f"resource must include {attr}"


def test_resource_includes_deployment_environment():
    rs = _RESOURCE_RS.read_text(encoding="utf-8")
    assert '"deployment.environment.name"' in rs
    assert "EFE_DEPLOY_ENV" in rs


def test_resource_includes_host_arch_and_os_type():
    rs = _RESOURCE_RS.read_text(encoding="utf-8")
    assert '"host.arch"' in rs
    assert '"os.type"' in rs


# ─────────────────────────────────────────────────────────────────────
# Sampler
# ─────────────────────────────────────────────────────────────────────


def test_error_boosting_sampler_exists():
    rs = _SAMPLER_RS.read_text(encoding="utf-8")
    assert "pub struct ErrorBoostingSampler" in rs
    assert "impl<S:" in rs and "ShouldSample" in rs
    # Pin the architecture promise : sample_reason="error_boost".
    assert '"error_boost"' in rs


def test_build_sampler_uses_parent_based():
    rs = _SAMPLER_RS.read_text(encoding="utf-8")
    assert "ParentBased" in rs
    assert "TraceIdRatioBased" in rs


# ─────────────────────────────────────────────────────────────────────
# Attribute filter
# ─────────────────────────────────────────────────────────────────────


def test_attribute_filter_uses_forbidden_prefixes():
    rs = _FILTER_RS.read_text(encoding="utf-8")
    assert "FORBIDDEN_ATTR_PREFIXES" in rs
    assert "is_forbidden_attribute" in rs
    assert "filter_attributes" in rs


# ─────────────────────────────────────────────────────────────────────
# Measuring exporter
# ─────────────────────────────────────────────────────────────────────


def test_measuring_exporter_bumps_metrics():
    rs = _MEASURING_RS.read_text(encoding="utf-8")
    assert "pub struct MeasuringSpanExporter" in rs
    # Bumps all 4 metrics.
    for fn in [
        "otel_export_total",
        "otel_export_duration_seconds",
        "otel_batch_size",
        "otel_dropped_total",
    ]:
        assert fn in rs, f"MeasuringSpanExporter must bump {fn}"


# ─────────────────────────────────────────────────────────────────────
# Prometheus instruments
# ─────────────────────────────────────────────────────────────────────


def test_d13bis_instruments_exists_with_5_metrics():
    """Audit fix § A15 added `otel_attributes_filtered_total` as the
    5th D13-bis instrument so ops can monitor PII-filter activity."""
    rs = _INSTRUMENTS_RS.read_text(encoding="utf-8")
    once_count = len(re.findall(r"^once_metric!", rs, re.MULTILINE))
    assert once_count == 5, (
        f"expected exactly 5 D13-bis instruments (4 original + audit § A15 "
        f"`otel_attributes_filtered_total`), found {once_count}"
    )


def test_register_all_d13bis_invokes_every_instrument():
    rs = _INSTRUMENTS_RS.read_text(encoding="utf-8")
    register = re.search(
        r"pub fn register_all_d13bis\(\) \{(.*?)\n\}", rs, re.DOTALL
    )
    assert register
    body = register.group(1)
    for fn in [
        "otel_export_total",
        "otel_export_duration_seconds",
        "otel_batch_size",
        "otel_dropped_total",
        # **Audit fix § A15** — PII filter visibility instrument
        # must also be eagerly registered.
        "otel_attributes_filtered_total",
    ]:
        assert fn in body, (
            f"register_all_d13bis must call {fn}() — drift between "
            f"instrument list and registration helper"
        )


def test_metrics_mod_re_exports_register_all_d13bis():
    rs = _METRICS_MOD.read_text(encoding="utf-8")
    assert "pub use d13bis_instruments::register_all_d13bis" in rs


def test_observability_init_invokes_d13bis_register():
    rs = _OBS_INIT_RS.read_text(encoding="utf-8")
    assert "register_all_d13bis" in rs


# ─────────────────────────────────────────────────────────────────────
# Helm
# ─────────────────────────────────────────────────────────────────────


EXPECTED_OTEL_KEYS = {
    "enabled",
    "endpoint",
    "sampleRate",
    "forceSampleOnError",
    "serviceName",
    "serviceVersion",
    "deployEnv",
    "failOnError",
}


def test_values_yaml_otel_block_complete():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    otel = data["config"]["observability"]["otel"]
    actual = set(otel.keys())
    missing = EXPECTED_OTEL_KEYS - actual
    extra = actual - EXPECTED_OTEL_KEYS
    assert not missing, f"missing: {missing}"
    assert not extra, f"unexpected: {extra}"


def test_values_yaml_otel_disabled_by_default():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["config"]["observability"]["otel"]["enabled"] is False


def test_values_yaml_sample_rate_default_is_5_percent():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["config"]["observability"]["otel"]["sampleRate"] == 0.05


def test_configmap_renders_otel_envvars():
    cm = _CONFIGMAP.read_text(encoding="utf-8")
    for var in [
        "EFE_OTEL_ENABLED",
        "EFE_OTEL_ENDPOINT",
        "EFE_OTEL_SAMPLE_RATE",
        "EFE_OTEL_FORCE_SAMPLE_ON_ERROR",
        "EFE_OTEL_SERVICE_NAME",
        "EFE_DEPLOY_ENV",
    ]:
        assert var in cm, f"{var} missing from configmap.yaml"


def test_networkpolicy_for_otel_egress_exists():
    netpol = _NETPOL.read_text(encoding="utf-8")
    assert "NetworkPolicy" in netpol
    assert "opentelemetry-collector" in netpol
    assert "4317" in netpol
    # Conditional on otel.enabled.
    assert ".Values.config.observability.otel.enabled" in netpol


# ─────────────────────────────────────────────────────────────────────
# Architecture pin (versioning)
# ─────────────────────────────────────────────────────────────────────


def test_architecture_doc_exists():
    doc = _REPO / "docs" / "bench" / "d13bis_otel_sdk_real_architecture.md"
    assert doc.exists(), "D13-bis architecture doc must exist"
    content = doc.read_text(encoding="utf-8")
    # Sanity : key sections present.
    for section in [
        "Resource attributes",
        "Sampler design",
        "Trace pipeline",
        "Self-observability",
    ]:
        assert section in content


# ─────────────────────────────────────────────────────────────────────
# Audit fix § A15 — `filter_span_batch` is invoked by MeasuringSpanExporter
# ─────────────────────────────────────────────────────────────────────


def test_audit_a15_filter_span_batch_exists():
    """Pre-audit, `attribute_filter::filter_attributes` was dead
    code — never invoked anywhere in the runtime pipeline. The
    audit fix introduces `filter_span_batch` which mutates the
    batch in place and is invoked by `MeasuringSpanExporter::export`.
    """
    rs = _FILTER_RS.read_text(encoding="utf-8")
    assert "pub fn filter_span_batch" in rs, (
        "audit fix § A15 introduced filter_span_batch — must remain"
    )


def test_audit_a15_measuring_exporter_invokes_filter():
    """The exporter MUST call `filter_span_batch` on every batch
    before forwarding to the inner exporter. Without this call,
    PII reaches Tempo despite the filter existing."""
    rs = _MEASURING_RS.read_text(encoding="utf-8")
    assert "filter_span_batch" in rs, (
        "MeasuringSpanExporter must invoke filter_span_batch — "
        "audit fix § A15 dead-code regression guard"
    )
    # The call must come BEFORE the export call, not after.
    body = rs
    pii_idx = body.find("filter_span_batch")
    export_idx = body.find("self.inner.export")
    assert pii_idx > 0
    assert export_idx > 0
    assert pii_idx < export_idx, (
        "filter_span_batch must run BEFORE inner.export — otherwise "
        "PII still reaches the wire"
    )


def test_audit_a15_pii_filter_metric_registered():
    """The audit introduced `efe_otel_attributes_filtered_total` so
    ops can monitor whether the PII filter is actively stripping
    keys. Steady-state zero ; any non-zero rate is a developer-side
    leak."""
    rs = _INSTRUMENTS_RS.read_text(encoding="utf-8")
    assert "otel_attributes_filtered_total" in rs
    assert "efe_otel_attributes_filtered_total" in rs


# ─────────────────────────────────────────────────────────────────────
# Audit fix § A2 — build_sampler signature
# ─────────────────────────────────────────────────────────────────────


def test_audit_a2_build_sampler_takes_only_rate():
    """Pre-audit, `build_sampler(rate, force_sample_on_error)` had
    a dead second parameter — both branches returned the same
    value. Audit fix § A2 drops the parameter."""
    rs = _SAMPLER_RS.read_text(encoding="utf-8")
    # Match either single-arg or no-arg signature ; refuse the
    # legacy two-arg form.
    assert "pub fn build_sampler(rate: f64) -> Sampler" in rs, (
        "build_sampler must take only `rate: f64` after audit § A2 ; "
        "second parameter was dead"
    )
    assert "force_sample_on_error: bool) -> Sampler" not in rs, (
        "legacy two-arg build_sampler is the audit-§A2 regression"
    )


# ─────────────────────────────────────────────────────────────────────
# Audit fix § A5 / A14 — idempotency latch + AlreadyInitialised variant
# ─────────────────────────────────────────────────────────────────────


def test_audit_a5_already_initialised_variant_present():
    """The audit introduced `OtelInitError::AlreadyInitialised` for
    the idempotency contract."""
    rs = _OTEL_MOD.read_text(encoding="utf-8")
    assert "AlreadyInitialised" in rs


def test_audit_a5_idempotency_latch_atomic_present():
    """The audit introduced a process-wide AtomicBool latch."""
    rs = _OTEL_MOD.read_text(encoding="utf-8")
    assert "OTEL_INITIALISED" in rs
    assert "AtomicBool" in rs


def test_audit_a5_init_uses_compare_exchange():
    """The latch uses CAS so two concurrent calls can't both win."""
    rs = _OTEL_MOD.read_text(encoding="utf-8")
    assert "compare_exchange" in rs


# ─────────────────────────────────────────────────────────────────────
# Audit fix § A4 — force_flush also bumps export_total{result=error}
# ─────────────────────────────────────────────────────────────────────


def test_audit_a4_force_flush_timeout_bumps_export_error():
    """Pre-audit, a force_flush timeout bumped only
    `efe_otel_dropped_total{reason="shutdown_timeout"}`. Post-audit,
    it also bumps `efe_otel_export_total{result="error"}` so SLO
    dashboards surface this alongside other export failures."""
    rs = _OTEL_MOD.read_text(encoding="utf-8")
    # The Err(_elapsed) arm must contain both metrics.
    import re as _re
    elapsed_arm = _re.search(
        r"Err\(_elapsed\) =>\s*\{(.*?)Err\(OtelShutdownError::Timeout",
        rs,
        _re.DOTALL,
    )
    assert elapsed_arm, "force_flush Err(_elapsed) arm not found"
    body = elapsed_arm.group(1)
    assert "otel_dropped_total" in body
    assert "otel_export_total" in body, (
        "audit § A4 : timeout must also bump otel_export_total{result=error}"
    )


# ─────────────────────────────────────────────────────────────────────
# Audit fix § A10 — OtelGuard::Drop logs shutdown failures
# ─────────────────────────────────────────────────────────────────────


def test_audit_a10_drop_warns_on_shutdown_error():
    """Pre-audit, `OtelGuard::Drop` silently swallowed shutdown
    errors via `let _ = self.provider.shutdown();`. Post-audit, it
    surfaces them via `tracing::warn!`."""
    rs = _OTEL_MOD.read_text(encoding="utf-8")
    import re as _re
    drop_block = _re.search(
        r"impl Drop for OtelGuard \{(.*?)\n\}\n", rs, _re.DOTALL
    )
    assert drop_block, "OtelGuard Drop impl not found"
    body = drop_block.group(1)
    assert "warn!" in body, (
        "audit § A10 : Drop must warn on shutdown failure"
    )


# ─────────────────────────────────────────────────────────────────────
# Audit fix § A16 — is_forbidden_attribute is allocation-free
# ─────────────────────────────────────────────────────────────────────


def test_audit_a16_is_forbidden_attribute_no_to_ascii_lowercase():
    """Pre-audit, `is_forbidden_attribute` called
    `key.to_ascii_lowercase()` allocating a String per check.
    Audit fix § A16 replaces it with a byte-level scan."""
    rs = _FILTER_RS.read_text(encoding="utf-8")
    # The allocation-free helper must exist.
    assert "contains_ignore_ascii_case" in rs, (
        "audit § A16 : zero-alloc helper missing"
    )
    # And `is_forbidden_attribute` must NOT call `to_ascii_lowercase`
    # on its input (it now uses the byte-level helper instead).
    import re as _re
    is_forb = _re.search(
        r"pub fn is_forbidden_attribute\(.*?\n\}\n", rs, _re.DOTALL
    )
    assert is_forb
    body = is_forb.group()
    assert "to_ascii_lowercase" not in body, (
        "audit § A16 regression : is_forbidden_attribute must not "
        "call to_ascii_lowercase on its input"
    )


# ─────────────────────────────────────────────────────────────────────
# Audit fix § A20 / A21 / A22 — operational wiring
# ─────────────────────────────────────────────────────────────────────


_INIT_RS = _TELEMETRY / "src" / "init.rs"
_CONFIG_RS = _TELEMETRY / "src" / "config.rs"


def test_audit_a20_init_telemetry_calls_otel_init():
    """**Audit fix § A20 (CRITICAL)** — pre-audit, the legacy
    `init_telemetry` never called `crate::otel::init`. The whole
    D13-bis pipeline was dead in production despite operators
    setting `EFE_OTEL_ENABLED=true`. Post-audit, `init_telemetry`
    delegates to `crate::otel::init` and stores the guard."""
    rs = _INIT_RS.read_text(encoding="utf-8")
    assert "crate::otel::init" in rs, (
        "init_telemetry must call crate::otel::init — "
        "audit § A20 critical wiring regression"
    )
    assert "OtelGuard" in rs


def test_audit_a20_telemetry_handle_owns_otel_guard():
    """The `TelemetryHandle` must own an `Option<OtelGuard>` so the
    shutdown path can force_flush before the process exits."""
    rs = _INIT_RS.read_text(encoding="utf-8")
    assert "otel_guard:" in rs, (
        "TelemetryHandle must have an `otel_guard` field"
    )
    assert "Option<OtelGuard>" in rs


def test_audit_a20_shutdown_calls_force_flush():
    """`TelemetryHandle::shutdown` must call `force_flush(timeout)`
    on the OtelGuard before letting it drop. Without this, the
    `BatchSpanProcessor`'s in-flight batch is stranded on every
    SIGTERM."""
    rs = _INIT_RS.read_text(encoding="utf-8")
    import re as _re
    shutdown_block = _re.search(
        r"pub async fn shutdown\(.*?\n    \}\n", rs, _re.DOTALL
    )
    assert shutdown_block, "shutdown method not found"
    body = shutdown_block.group()
    assert "force_flush" in body, (
        "shutdown must call OtelGuard::force_flush — audit § A20"
    )


def test_audit_a20_telemetry_handle_exposes_otel_active():
    """A `pub fn otel_active(&self) -> bool` getter so /health
    endpoints can surface trace pipeline state."""
    rs = _INIT_RS.read_text(encoding="utf-8")
    assert "pub fn otel_active" in rs


def test_audit_a20_subscriber_composes_otel_layer():
    """`install_subscriber` must compose the
    `tracing-opentelemetry` layer when a guard exists. Without
    this, `tracing` macros in handlers don't auto-export to OTel."""
    rs = _INIT_RS.read_text(encoding="utf-8")
    assert "tracing_opentelemetry::layer" in rs, (
        "install_subscriber must compose tracing-opentelemetry layer "
        "— audit § A20"
    )
    # The subscriber install must include both branches : with and
    # without guard.
    assert "if let Some(g) = guard" in rs


def test_audit_a21_init_telemetry_registers_d13bis():
    """**Audit fix § A21** — `init_telemetry` must call
    `register_all_d13bis()` unconditionally so the 5 D13-bis
    instruments appear in /metrics from the first scrape, even in
    OTel-disabled clusters."""
    rs = _INIT_RS.read_text(encoding="utf-8")
    import re as _re
    init_fn = _re.search(
        r"pub fn init\(cfg: TelemetryConfig\).*?\n\}\n",
        rs,
        _re.DOTALL,
    )
    assert init_fn
    body = init_fn.group()
    assert "register_all_d13bis" in body, (
        "init_telemetry must call register_all_d13bis() — "
        "audit § A21 dashboards-show-no-data regression"
    )


def test_audit_a22_telemetry_config_embeds_otel():
    """**Audit fix § A22** — `TelemetryConfig` must embed an
    `OtelConfig` field so the canonical `from_env()` path produces
    a fully resolved telemetry+OTel config in one call."""
    rs = _CONFIG_RS.read_text(encoding="utf-8")
    assert "pub otel: OtelConfig" in rs, (
        "TelemetryConfig must embed OtelConfig — audit § A22"
    )
    assert "pub fail_on_otel_init_error: bool" in rs


def test_audit_a22_telemetry_config_from_env_calls_otel_from_env():
    """`from_env()` must populate the embedded otel via
    `OtelConfig::from_env()`."""
    rs = _CONFIG_RS.read_text(encoding="utf-8")
    assert "OtelConfig::from_env" in rs, (
        "TelemetryConfig::from_env must call OtelConfig::from_env"
    )
    # And the failOnError env var must be honored.
    assert "EFE_OBSERVABILITY_FAIL_ON_OTEL_ERROR" in rs


def test_audit_a20_telemetry_init_error_has_otel_init_variant():
    """The `TelemetryInitError` enum must include an `OtelInit`
    variant so the wiring can propagate OtelInitError as a fatal
    init failure under fail_on_otel_init_error mode."""
    rs = _INIT_RS.read_text(encoding="utf-8")
    assert "OtelInit(#[from] OtelInitError)" in rs


def test_audit_a20_main_uses_telemetry_init_path():
    """efe-server main MUST call the unified `init_telemetry`
    path that bootstraps OTel — pre-audit it called `init_telemetry`
    but that function didn't bootstrap OTel. Post-audit fix, the
    path is correct AND the function actually does the work."""
    main_rs = (_REPO / "crates" / "efe-server" / "src" / "main.rs").read_text(
        encoding="utf-8"
    )
    # main calls init_telemetry which post-A20 builds the OTel
    # pipeline transparently — no explicit OTel call needed in main.
    assert "init as init_telemetry" in main_rs or "init_telemetry" in main_rs
    # And main calls shutdown which post-A20 force-flushes OTel.
    assert "telemetry.shutdown()" in main_rs


# ─────────────────────────────────────────────────────────────────────
# Architecture § 22 acceptance criteria — audit report file exists
# ─────────────────────────────────────────────────────────────────────


def test_audit_report_exists():
    """Architecture § 22 acceptance criterion : `docs/audits/d13bis_audit_report.md`
    MUST exist before the sprint can be closed. Pattern identical to
    every previous D-X sprint."""
    report = _REPO / "docs" / "audits" / "d13bis_audit_report.md"
    assert report.exists(), (
        "D13-bis audit report missing — architecture § 22 critère "
        "d'acceptation"
    )
    content = report.read_text(encoding="utf-8")
    # Sanity : contains the canonical sections + records both audit
    # passes (Pass #1 internal defects + Pass #2 operational wiring).
    for marker in [
        "A15",  # the Critical PII filter dead-code finding
        "A20",  # the Critical wiring finding from pass #2
        "Pass #1",
        "Pass #2",
        "Conclusion",
        "Dette technique",
    ]:
        assert marker in content, (
            f"audit report missing required section/marker: {marker!r}"
        )


def test_audit_report_documents_all_13_findings():
    """The report must reference all 13 audit IDs (10 from pass #1
    + 3 from pass #2). A future audit pass that adds findings must
    update this count + update the report."""
    report = _REPO / "docs" / "audits" / "d13bis_audit_report.md"
    content = report.read_text(encoding="utf-8")
    expected_ids = [
        # Pass #1
        "A2",
        "A3",
        "A4",
        "A5",
        "A6",
        "A8",
        "A9",
        "A10",
        "A11",
        "A14",
        "A15",
        "A16",
        # Pass #2
        "A20",
        "A21",
        "A22",
    ]
    for fid in expected_ids:
        assert fid in content, (
            f"audit report missing finding ID: {fid}"
        )

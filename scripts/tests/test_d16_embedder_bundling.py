"""Sprint **D16** — Embedder bundling Helm + cross-layer pin tests.

Pin tests that don't require a live Postgres / kind cluster :

* ``values.yaml`` exposes a ``config.embedder`` block with the
  expected fields ;
* ``templates/configmap.yaml`` renders all ``EFE_*`` env vars
  consumed by `config.rs::ServerConfig::from_env` ;
* ``templates/deployment.yaml`` mounts the bundle Secret + the
  trust-store Secret only when ``backend = bundle`` ;
* ``crates/efe-server/migrations/0005_extend_embedder_fingerprint.sql``
  is well-formed + adds the two new columns + the XOR CHECK ;
* the manifest schema constants (CANONICAL_DIMS,
  SUPPORTED_MANIFEST_VERSION) are pinned by the Rust code ;
* Every D16 outcome label appears in BOTH the metric definition
  AND the dashboard query (drift surface).

These tests do NOT require any cluster — chart files + migration
SQL + Rust source only.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import yaml

_REPO = Path(__file__).resolve().parents[2]
_CHART = _REPO / "deploy" / "helm" / "efe"
_VALUES = _CHART / "values.yaml"
_CONFIGMAP = _CHART / "templates" / "configmap.yaml"
_DEPLOYMENT = _CHART / "templates" / "deployment.yaml"
_MIGRATION = (
    _REPO / "crates" / "efe-server" / "migrations"
    / "0005_extend_embedder_fingerprint.sql"
)
_CONFIG_RS = _REPO / "crates" / "efe-server" / "src" / "config.rs"
_MANIFEST_RS = (
    _REPO / "crates" / "efe-server" / "src" / "embedder" / "manifest.rs"
)
_BUNDLE_RS = (
    _REPO / "crates" / "efe-server" / "src" / "embedder" / "bundle.rs"
)
_VERIFY_RS = (
    _REPO / "crates" / "efe-server" / "src" / "embedder" / "verify.rs"
)
_INSTRUMENTS_RS = (
    _REPO / "crates" / "efe-telemetry" / "src" / "metrics"
    / "d16_instruments.rs"
)


# ─────────────────────────────────────────────────────────────────────
# values.yaml — embedder block
# ─────────────────────────────────────────────────────────────────────

EXPECTED_EMBEDDER_TOP_KEYS = {"backend", "bundle"}
EXPECTED_BUNDLE_KEYS = {
    "path",
    "requireSignature",
    "trustStorePath",
    "requireSbom",
    "acknowledgeNonRedistributable",
    "loadTimeoutS",
    "encodeTimeoutMs",
    "existingSecretName",
    "trustStoreSecretName",
}


def test_values_yaml_has_embedder_block():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    emb = data["config"]["embedder"]
    actual_top = set(emb.keys())
    missing = EXPECTED_EMBEDDER_TOP_KEYS - actual_top
    extra = actual_top - EXPECTED_EMBEDDER_TOP_KEYS
    assert not missing, f"missing top keys: {missing}"
    assert not extra, f"unexpected top keys: {extra}"


def test_values_yaml_default_backend_is_legacy():
    """Backwards-compat — D16 must NOT silently activate the bundle
    path on existing deployments."""
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["config"]["embedder"]["backend"] == "legacy"


def test_values_yaml_bundle_keys_complete():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    bundle = data["config"]["embedder"]["bundle"]
    actual = set(bundle.keys())
    missing = EXPECTED_BUNDLE_KEYS - actual
    extra = actual - EXPECTED_BUNDLE_KEYS
    assert not missing, f"missing bundle keys: {missing}"
    assert not extra, f"unexpected bundle keys: {extra}"


def test_values_yaml_default_signature_required():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert (
        data["config"]["embedder"]["bundle"]["requireSignature"] == "required"
    ), "default policy MUST be 'required' for prod-safe boot"


def test_values_yaml_path_consistency_path_starts_at_etc_efe():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    p = data["config"]["embedder"]["bundle"]["path"]
    t = data["config"]["embedder"]["bundle"]["trustStorePath"]
    assert p.startswith("/etc/efe/"), f"unexpected bundle path: {p}"
    assert t.startswith("/etc/efe/"), f"unexpected trust-store path: {t}"


# ─────────────────────────────────────────────────────────────────────
# configmap.yaml — env vars
# ─────────────────────────────────────────────────────────────────────

EXPECTED_D16_ENVVARS = {
    "EFE_EMBEDDER_BACKEND",
    "EFE_BUNDLE_PATH",
    "EFE_BUNDLE_REQUIRE_SIGNATURE",
    "EFE_BUNDLE_TRUST_STORE",
    "EFE_BUNDLE_REQUIRE_SBOM",
    "EFE_BUNDLE_ACK_NONREDIST",
    "EFE_BUNDLE_LOAD_TIMEOUT_S",
    "EFE_EMBEDDER_ENCODE_TIMEOUT_MS",
}


def test_configmap_renders_all_d16_envvars():
    cm = _CONFIGMAP.read_text(encoding="utf-8")
    for var in EXPECTED_D16_ENVVARS:
        assert var in cm, f"{var} not rendered in configmap.yaml"


def test_configmap_envvars_match_config_rs():
    """Every env var rendered by Helm must be parsed by config.rs."""
    config_rs = _CONFIG_RS.read_text(encoding="utf-8")
    for var in EXPECTED_D16_ENVVARS:
        assert var in config_rs, (
            f"{var} appears in configmap.yaml but is not consumed by "
            f"config.rs ; either drop it from the chart or wire it "
            f"into ServerConfig::from_env"
        )


# ─────────────────────────────────────────────────────────────────────
# deployment.yaml — Secret mounts
# ─────────────────────────────────────────────────────────────────────


def test_deployment_mounts_bundle_when_backend_bundle():
    dep = _DEPLOYMENT.read_text(encoding="utf-8")
    assert "embedder-bundle" in dep
    assert "embedder-trust" in dep
    # Both mounts gated on `backend == "bundle"`.
    assert (
        'eq .Values.config.embedder.backend "bundle"' in dep
    ), "bundle volume mount must be gated on backend = bundle"


def test_deployment_bundle_mount_read_only():
    dep = _DEPLOYMENT.read_text(encoding="utf-8")
    # Find the embedder-bundle volumeMount block.
    idx = dep.find("name: embedder-bundle")
    assert idx > 0
    snippet = dep[idx : idx + 250]
    assert "readOnly: true" in snippet


def test_deployment_trust_store_mount_subpath():
    dep = _DEPLOYMENT.read_text(encoding="utf-8")
    idx = dep.find("name: embedder-trust")
    assert idx > 0
    snippet = dep[idx : idx + 400]
    # subPath must be set so the mount projects to a single file
    # (architecture § 12.3).
    assert "subPath:" in snippet
    assert "pubkeys.json" in snippet


# ─────────────────────────────────────────────────────────────────────
# 0005 migration
# ─────────────────────────────────────────────────────────────────────


def test_migration_exists():
    assert _MIGRATION.exists()


def test_migration_adds_two_columns():
    sql = _MIGRATION.read_text(encoding="utf-8")
    assert "bundle_fingerprint" in sql
    assert "legacy_embedder_fingerprint" in sql


def test_migration_xor_constraint_present():
    sql = _MIGRATION.read_text(encoding="utf-8")
    assert "catalogs_fingerprint_xor" in sql, "XOR constraint name missing"
    # Both halves of the XOR formula present.
    assert "bundle_fingerprint IS NOT NULL AND legacy_embedder_fingerprint IS NULL" in sql
    assert "bundle_fingerprint IS NULL     AND legacy_embedder_fingerprint IS NOT NULL" in sql


def test_migration_fingerprint_hex_check():
    sql = _MIGRATION.read_text(encoding="utf-8")
    assert "[0-9a-f]{64}" in sql, "fingerprint hex regex CHECK missing"


def test_migration_partial_index_on_bundle_fingerprint():
    sql = _MIGRATION.read_text(encoding="utf-8")
    assert "catalogs_bundle_fingerprint_idx" in sql
    assert "WHERE bundle_fingerprint IS NOT NULL" in sql


# ─────────────────────────────────────────────────────────────────────
# Manifest constants pin
# ─────────────────────────────────────────────────────────────────────


def test_manifest_supported_version_is_1_0():
    rs = _MANIFEST_RS.read_text(encoding="utf-8")
    assert (
        'pub const SUPPORTED_MANIFEST_VERSION: &str = "1.0";' in rs
    ), "manifest version constant pin"


def test_manifest_canonical_dims_includes_common():
    rs = _MANIFEST_RS.read_text(encoding="utf-8")
    # Architecture § 4.3 INV-16-3 lists exactly these dims. A drift
    # would cause runtime rejection of bundles that previously loaded.
    for dim in [64, 128, 256, 384, 512, 768, 1024, 1536, 2048, 4096]:
        assert f"{dim}," in rs or f"{dim},\n" in rs, f"dim {dim} missing"


def test_manifest_max_seq_len_hard_cap_16384():
    rs = _MANIFEST_RS.read_text(encoding="utf-8")
    assert "MAX_SEQ_LEN_HARD_CAP: usize = 16_384" in rs


# ─────────────────────────────────────────────────────────────────────
# Outcomes pin (architecture § 13.3)
# ─────────────────────────────────────────────────────────────────────


def test_load_outcomes_pinned():
    """Architecture § 13.3 lists exactly 6 LoadOutcome values."""
    rs = _BUNDLE_RS.read_text(encoding="utf-8")
    for label in [
        '"ok"',
        '"manifest_invalid"',
        '"signature_failed"',
        '"hash_mismatch"',
        '"runtime_init_failed"',
        '"policy_violation"',
    ]:
        assert label in rs, f"LoadOutcome label {label} missing"


def test_telemetry_d16_instrument_count():
    rs = _INSTRUMENTS_RS.read_text(encoding="utf-8")
    # Architecture § 13.1 promises exactly 9 instruments.
    once_metric_count = len(re.findall(r"^once_metric!", rs, re.MULTILINE))
    assert once_metric_count == 9, (
        f"expected exactly 9 D16 instruments, found {once_metric_count}"
    )


def test_telemetry_register_all_d16_calls_all_instruments():
    rs = _INSTRUMENTS_RS.read_text(encoding="utf-8")
    register_block = re.search(
        r"pub fn register_all_d16\(\) \{(.*?)\n\}", rs, re.DOTALL
    )
    assert register_block, "register_all_d16 block missing"
    body = register_block.group(1)
    # Must invoke every instrument constructor.
    for fn in [
        "bundle_load_total",
        "bundle_load_duration_seconds",
        "bundle_signature_verifications_total",
        "bundle_hash_verifications_total",
        "embedder_encode_total",
        "embedder_encode_duration_seconds",
        "embedder_tokenizer_truncations_total",
        "embedder_active_bundles",
        "embedder_resident_bytes",
    ]:
        assert fn in body, (
            f"register_all_d16 does not call {fn}() — drift between "
            f"instrument list and registration helper"
        )


# ─────────────────────────────────────────────────────────────────────
# Path-traversal regression guard
# ─────────────────────────────────────────────────────────────────────


def test_verify_rs_rejects_path_traversal():
    rs = _VERIFY_RS.read_text(encoding="utf-8")
    assert "path_has_traversal" in rs
    # Regression : the function must check '..', '/' prefix, and ':'
    # for Windows drives.
    assert '".."' in rs
    assert "starts_with('/')" in rs
    assert "starts_with('\\\\')" in rs


# ─────────────────────────────────────────────────────────────────────
# Cross-layer cardinality budget
# ─────────────────────────────────────────────────────────────────────


def test_cardinality_budget_under_5000():
    """Architecture § 13.2 commits to ~3 150 series at scale (5
    bundles × 100 tenants). Pin against drift above 5 k."""
    rs = _INSTRUMENTS_RS.read_text(encoding="utf-8")
    # Heuristic : count distinct labels on each instrument.
    # The architecture doc has the precise table ; here we just
    # ensure there's no instrument with a `tenant_id` label
    # (would explode cardinality).
    forbidden_label_combo = re.search(
        r'"tenant_id"', rs
    )  # only "tenant" allowed (5 bundles × 100 tenants)
    assert (
        forbidden_label_combo is None
    ), "instrument carries a `tenant_id` label — should be `tenant` only"


# ─────────────────────────────────────────────────────────────────────
# License invariant
# ─────────────────────────────────────────────────────────────────────


def test_loader_refuses_nonredistributable_without_ack():
    rs = _BUNDLE_RS.read_text(encoding="utf-8")
    assert "NonRedistributableLicense" in rs
    assert "ack_non_redistributable" in rs
    assert "redistribution_allowed" in rs


# ─────────────────────────────────────────────────────────────────────
# Audit-fix regression guards
# ─────────────────────────────────────────────────────────────────────


def test_audit_fix_c1_runtime_wires_d16():
    """**Audit fix D16-C1** — runtime.rs MUST call register_all_d16
    AND build_embedder_registry. Pre-audit code never read the
    EFE_EMBEDDER_BACKEND env var at runtime."""
    runtime_rs = (
        _REPO / "crates" / "efe-server" / "src" / "runtime.rs"
    ).read_text(encoding="utf-8")
    assert (
        "register_all_d16()" in runtime_rs
    ), "register_all_d16 must be called at boot (audit fix C1)"
    assert (
        "build_embedder_registry" in runtime_rs
    ), "embedder registry builder must be wired into Server::build"
    assert (
        "embedder_registry" in runtime_rs
    ), "Server struct must hold an embedder_registry field"


def test_audit_fix_c2_singleflight_uses_register_then_check():
    """**Audit fix D16-C2** — the singleflight wait_for_leader
    helper MUST register the Notified future BEFORE checking the
    result, otherwise a notify between check and await is lost."""
    cache_rs = (
        _REPO / "crates" / "efe-server" / "src" / "embedder" / "cache.rs"
    ).read_text(encoding="utf-8")
    assert "wait_for_leader" in cache_rs, "wait_for_leader helper missing"
    # Architecture pin : pre-arm the Notified BEFORE the result lock.
    body_start = cache_rs.find("async fn wait_for_leader")
    body_end = cache_rs.find("\n}\n", body_start)
    body = cache_rs[body_start:body_end]
    notified_pos = body.find("entry.notify.notified()")
    lock_pos = body.find("entry.result.lock()")
    assert notified_pos > 0 and lock_pos > 0
    assert (
        notified_pos < lock_pos
    ), "wait_for_leader must call notify.notified() BEFORE the result lock (audit fix C2)"


def test_audit_fix_c3_no_unwrap_in_mock_session():
    """**Audit fix D16-C3** — MockSession must not use
    `.unwrap()` (violates `deny(clippy::unwrap_used)` in lib build).

    We strip narrative comments before scanning so the historical
    audit-fix-explanation comment doesn't trip the regex.
    """
    mock_rs = (
        _REPO / "crates" / "efe-server" / "src" / "embedder" / "runtime"
        / "mock.rs"
    ).read_text(encoding="utf-8")
    body_start = mock_rs.find("fn encode_inner")
    body_end = mock_rs.find("\n    }\n", body_start)
    body = mock_rs[body_start:body_end]
    # Strip line comments (`// ...`) before checking — the audit-fix
    # narrative legitimately mentions `.unwrap()` in prose.
    code_only = "\n".join(
        line for line in body.splitlines()
        if not line.lstrip().startswith("//")
    )
    assert (
        ".unwrap()" not in code_only
    ), "MockSession::encode_inner uses .unwrap() (audit fix C3 regressed)"


def test_audit_fix_c4_sbom_path_must_be_in_components():
    """**Audit fix D16-C4** — manifest validate refuses sbom_path
    that doesn't appear in components[]."""
    manifest_rs = _MANIFEST_RS.read_text(encoding="utf-8")
    assert "SbomPathNotInComponents" in manifest_rs


def test_audit_fix_c5_xor_constraint_allows_both_null():
    """**Audit fix D16-C5** — migration 0005 XOR constraint must
    accept rows where both bundle_fingerprint and
    legacy_embedder_fingerprint are NULL (pre-D12 back-compat)."""
    sql = _MIGRATION.read_text(encoding="utf-8")
    # Both-NULL branch.
    assert (
        "bundle_fingerprint IS NULL     AND legacy_embedder_fingerprint IS NULL"
        in sql
    ), "audit fix C5 — both-NULL fallback missing"


def test_audit_fix_h1_policy_runs_before_hash_walk():
    """**Audit fix D16-H1** — in load_bundle_dir, policy checks
    (SBOM, license) MUST appear textually BEFORE the
    `verify_components` call so they fail-fast on cheap manifest
    fields without burning CPU on the 500 MiB hash walk."""
    bundle_rs = _BUNDLE_RS.read_text(encoding="utf-8")
    body_start = bundle_rs.find("pub async fn load_bundle_dir")
    body_end = bundle_rs.find("\n// =", body_start)
    body = bundle_rs[body_start:body_end]
    sbom_pos = body.find("SbomRequiredButMissing")
    license_pos = body.find("NonRedistributableLicense")
    verify_pos = body.find("verify_components(&manifest, root)")
    assert sbom_pos > 0 and license_pos > 0 and verify_pos > 0
    assert (
        sbom_pos < verify_pos
    ), "SBOM check must run before verify_components (audit fix H1)"
    assert (
        license_pos < verify_pos
    ), "License check must run before verify_components (audit fix H1)"


def test_audit_fix_h2_load_timeout_wired():
    """**Audit fix D16-H2** — the registry exposes
    `ensure_loaded_with_timeout` and uses tokio::time::timeout
    around `load_bundle_dir`. Pre-audit code had no upper bound."""
    cache_rs = (
        _REPO / "crates" / "efe-server" / "src" / "embedder" / "cache.rs"
    ).read_text(encoding="utf-8")
    assert "ensure_loaded_with_timeout" in cache_rs
    assert "tokio::time::timeout" in cache_rs
    assert "EFE_BUNDLE_LOAD_TIMEOUT_S" in cache_rs


def test_audit_fix_h3_h5_h6_metrics_emitted_in_paths():
    """**Audit fix D16-H3 + H5 + H6** — the runtime hot paths now
    emit the D16 counters/histograms. Pre-audit code only registered
    them ; they were always zero."""
    bundle_rs = _BUNDLE_RS.read_text(encoding="utf-8")
    # H3 (encode metrics)
    assert "embedder_encode_total()" in bundle_rs
    assert "embedder_encode_duration_seconds()" in bundle_rs
    # H5 (hash verify metric)
    assert "bundle_hash_verifications_total()" in bundle_rs
    # H6 (signature verify metric)
    assert "bundle_signature_verifications_total()" in bundle_rs
    # The signature-metric wrapper helper.
    assert "verify_manifest_signature_with_metric" in bundle_rs


def test_audit_fix_h4_path_traversal_normalises_backslash():
    """**Audit fix D16-H4** — path_has_traversal must normalise
    backslashes before splitting, so Windows-style traversal
    `..\\..\\etc\\passwd` is rejected on Linux too."""
    verify_rs = _VERIFY_RS.read_text(encoding="utf-8")
    assert (
        "p.replace('\\\\', \"/\")" in verify_rs
        or 'p.replace(\'\\\\\', "/")' in verify_rs
    ), "path_has_traversal must normalise backslashes (audit fix H4)"


def test_audit_fix_h7_pack_filters_hidden_files():
    """**Audit fix D16-H7** — efe-bundle pack walk_and_hash MUST
    skip hidden files (any segment starting with `.`)."""
    cli_rs = (
        _REPO / "crates" / "efe-server" / "src" / "bin" / "efe_bundle.rs"
    ).read_text(encoding="utf-8")
    assert (
        "name_starts_with_dot" in cli_rs
        or 'starts_with(\'.\')' in cli_rs
    ), "walk_and_hash must skip hidden files (audit fix H7)"


def test_audit_fix_h8_sign_key_validated_early():
    """**Audit fix D16-H8** — sign-key file is validated BEFORE the
    walk + hash + copy step, not after, so operators with a bad
    --sign-key path get instant feedback."""
    cli_rs = (
        _REPO / "crates" / "efe-server" / "src" / "bin" / "efe_bundle.rs"
    ).read_text(encoding="utf-8")
    body_start = cli_rs.find("async fn cmd_pack")
    body_end = cli_rs.find("\n}\n", body_start)
    body = cli_rs[body_start:body_end]
    # The sign-key validation must come before the walk_and_hash call.
    sign_pos = body.find("signing_key_loaded")
    walk_pos = body.find("walk_and_hash(source_dir,")
    assert sign_pos > 0 and walk_pos > 0
    assert (
        sign_pos < walk_pos
    ), "sign-key validation must precede walk_and_hash (audit fix H8)"


def test_audit_fix_h10_skip_policy_logs_present_signature():
    """**Audit fix D16-H10** — Skip mode must log an info line if
    a signature is present but skipped, so ops can debug "why isn't
    my bundle being verified?"."""
    bundle_rs = _BUNDLE_RS.read_text(encoding="utf-8")
    assert "policy=skip" in bundle_rs.lower() or "policy = skip" in bundle_rs.lower()


def test_audit_fix_m1_warn_on_legacy_match_exists():
    """**Audit fix D16-M1** — the public API exposes
    `warn_on_legacy_match` so callers can emit the
    "byte-exact pinning recommended" warning consistently."""
    legacy_rs = (
        _REPO / "crates" / "efe-server" / "src" / "embedder" / "legacy.rs"
    ).read_text(encoding="utf-8")
    assert "pub fn warn_on_legacy_match" in legacy_rs
    assert "byte-exact pinning recommended" in legacy_rs


def test_audit_fix_m2_no_redundant_spawn_in_cache():
    """**Audit fix D16-M2** — leader path no longer spawns a
    redundant tokio task to do an insert that the inline path
    already does."""
    cache_rs = (
        _REPO / "crates" / "efe-server" / "src" / "embedder" / "cache.rs"
    ).read_text(encoding="utf-8")
    body_start = cache_rs.find("ensure_loaded_with_timeout")
    body_end = cache_rs.find("async fn wait_for_leader", body_start)
    body = cache_rs[body_start:body_end]
    assert (
        "tokio::spawn" not in body
    ), "leader path must not spawn redundant inserts (audit fix M2)"


# ─────────────────────────────────────────────────────────────────────
# Architecture acceptance criterion : D16 audit report exists
# ─────────────────────────────────────────────────────────────────────


def test_d16_audit_report_exists():
    """D16 architecture doc § 22 listed `docs/audits/d16_audit_report.md`
    as an explicit acceptance criterion. This pin test guards
    against a future refactor that deletes the report."""
    report = _REPO / "docs" / "audits" / "d16_audit_report.md"
    assert report.exists(), (
        "D16 audit report missing — architecture § 22 acceptance "
        "criterion not satisfied"
    )
    content = report.read_text(encoding="utf-8")
    # Sanity : contains the canonical sections + records the 17
    # findings.
    for marker in [
        "C1",   # NEVER WIRED critical
        "C2",   # lost-wakeup race
        "C4",   # sbom_path orphan
        "H1",   # policy before hash ordering
        "H7",   # hidden-file filter
        "Conclusion",
        "Dette technique",
    ]:
        assert marker in content, (
            f"D16 audit report missing required marker: {marker!r}"
        )


def test_d16_audit_report_documents_17_findings():
    """The report must reference the 16 finding IDs (C1-C5, H1-H10
    excluding H9, M1-M2 = 16 total ; C1 + 4 + 9 + 2 = 16)."""
    report = _REPO / "docs" / "audits" / "d16_audit_report.md"
    content = report.read_text(encoding="utf-8")
    expected_ids = [
        "C1", "C2", "C3", "C4", "C5",
        "H1", "H2", "H3", "H4", "H5", "H6", "H7", "H8", "H10",
        "M1", "M2",
    ]
    for fid in expected_ids:
        assert fid in content, (
            f"D16 audit report missing finding ID: {fid}"
        )


def test_d16_audit_report_documents_c1_wiring_class():
    """**Critical pattern guard** : the C1 finding (D16 was NEVER
    WIRED into runtime) is the same defect class as D13bis-A20.
    The D16 audit report must explicitly mention the cross-sprint
    relationship so future maintainers learn from it."""
    report = _REPO / "docs" / "audits" / "d16_audit_report.md"
    content = report.read_text(encoding="utf-8")
    # The doc must reference D13-bis A20 as the same defect class.
    assert "A20" in content, (
        "D16 audit report should reference D13-bis A20 as the "
        "recurring 'wiring' defect class"
    )
    # And must articulate the institutional learning.
    assert "Python pin test" in content
    assert "binaire principal" in content

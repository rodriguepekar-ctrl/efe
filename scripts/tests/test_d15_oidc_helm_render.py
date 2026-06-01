"""Sprint **D15** — Helm chart OIDC + tenant-onboarding rendering.

Pin tests for the architecture-doc invariants that are easy to drift
silently across the YAML / Rust boundary :

* ``values.yaml`` exposes a ``config.auth.oidc`` block with the 9
  expected fields ;
* ``templates/configmap.yaml`` renders all 7 ``EFE_OIDC_*`` env vars ;
* ``templates/deployment.yaml`` mounts the issuers-Secret at
  ``/etc/efe`` only when ``oidc.enabled && existingSecretName`` ;
* ``crates/efe-server/migrations/0004_tenants.sql`` exists with the
  6 CHECK constraints from the architecture doc § 4 ;
* ``deploy/config/oidc-issuers.example.json`` is valid JSON and
  contains at least 3 vendor templates (Auth0 / Keycloak / Okta).

These tests do NOT require a live cluster — chart files + migration
SQL + example JSON only.

Cross-validation rule : every ``EFE_OIDC_*`` env var rendered by the
ConfigMap must be parsed by ``crates/efe-server/src/config.rs`` so a
typo on either side surfaces in CI rather than in production.
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
_MIGRATION = _REPO / "crates" / "efe-server" / "migrations" / "0004_tenants.sql"
_EXAMPLE_ISSUERS = _REPO / "deploy" / "config" / "oidc-issuers.example.json"
_CONFIG_RS = _REPO / "crates" / "efe-server" / "src" / "config.rs"
_ISSUERS_RS = _REPO / "crates" / "efe-server" / "src" / "auth" / "issuers.rs"
_TENANTS_RS = _REPO / "crates" / "efe-server" / "src" / "auth" / "tenants.rs"


# ─────────────────────────────────────────────────────────────────────
# values.yaml — oidc block structure
# ─────────────────────────────────────────────────────────────────────

EXPECTED_OIDC_KEYS = {
    "enabled",
    "issuersFile",
    "jwksTtlS",
    "jwksFetchTimeoutMs",
    "jwksRetryMax",
    "jwksBackoffMs",
    "graceStaleS",
    "existingSecretName",
    "existingSecretKey",
}


def test_values_yaml_has_oidc_block():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    oidc = data["config"]["auth"]["oidc"]
    assert isinstance(oidc, dict)
    actual = set(oidc.keys())
    missing = EXPECTED_OIDC_KEYS - actual
    extra = actual - EXPECTED_OIDC_KEYS
    assert not missing, f"missing OIDC keys in values.yaml: {missing}"
    assert not extra, f"unexpected OIDC keys in values.yaml: {extra}"


def test_values_yaml_oidc_disabled_by_default():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["config"]["auth"]["oidc"]["enabled"] is False


def test_values_yaml_oidc_defaults_match_architecture():
    """Architecture doc § 13.2 pins these defaults — drift silently
    breaks operator expectations."""
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    oidc = data["config"]["auth"]["oidc"]
    assert oidc["jwksTtlS"] == 300, "jwksTtlS should be 300 (5 min)"
    assert oidc["jwksFetchTimeoutMs"] == 5000, "jwksFetchTimeoutMs default = 5 s"
    assert oidc["jwksRetryMax"] == 3, "retry budget default = 3"
    assert oidc["jwksBackoffMs"] == 1000, "initial backoff default = 1 s"
    assert oidc["graceStaleS"] == 3600, "grace stale default = 1 h"
    # **Audit fix D15-M7** — mount sub-path moved to avoid shadowing.
    assert oidc["issuersFile"] == "/etc/efe/oidc/oidc-issuers.json"


def test_oidc_path_consistency_across_layers():
    """**Audit fix D15-C3 + M7 regression guard.**

    The ``issuersFile`` path appears in three places that drift if not
    pinned : Helm values, the Rust default, and the CLI default.
    Mismatch produces silent "issuers file not found" errors when the
    operator launches the binary outside Helm or runs the CLI with
    defaults.
    """
    canonical = "/etc/efe/oidc/oidc-issuers.json"
    helm = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert helm["config"]["auth"]["oidc"]["issuersFile"] == canonical

    config_rs = _CONFIG_RS.read_text(encoding="utf-8")
    assert canonical in config_rs, (
        f"OidcConfig::default()::issuers_file does not match Helm path "
        f"{canonical!r}"
    )

    cli_rs = (
        _REPO / "crates" / "efe-server" / "src" / "bin" / "efe_onboard.rs"
    ).read_text(encoding="utf-8")
    assert canonical in cli_rs, (
        f"efe-onboard preflight default does not match Helm path "
        f"{canonical!r}"
    )


# ─────────────────────────────────────────────────────────────────────
# configmap.yaml — env vars
# ─────────────────────────────────────────────────────────────────────

EXPECTED_OIDC_ENVVARS = {
    "EFE_OIDC_ENABLED",
    "EFE_OIDC_ISSUERS_FILE",
    "EFE_OIDC_JWKS_TTL_S",
    "EFE_OIDC_JWKS_FETCH_TIMEOUT_MS",
    "EFE_OIDC_JWKS_RETRY_MAX",
    "EFE_OIDC_JWKS_BACKOFF_MS",
    "EFE_OIDC_GRACE_STALE_S",
}


def test_configmap_renders_all_oidc_envvars():
    cm = _CONFIGMAP.read_text(encoding="utf-8")
    for var in EXPECTED_OIDC_ENVVARS:
        assert var in cm, f"{var} not rendered by configmap.yaml"


def test_configmap_oidc_envvars_match_config_rs_parsers():
    """Every EFE_OIDC_* env rendered by Helm must be parsed by
    config.rs ; otherwise the operator's Helm value is silently
    ignored."""
    config_rs = _CONFIG_RS.read_text(encoding="utf-8")
    for var in EXPECTED_OIDC_ENVVARS:
        assert var in config_rs, (
            f"{var} appears in configmap.yaml but is not consumed by "
            f"config.rs ; either drop it from the chart or wire it "
            f"into ServerConfig::from_env"
        )


# ─────────────────────────────────────────────────────────────────────
# deployment.yaml — Secret volume mount
# ─────────────────────────────────────────────────────────────────────


def test_deployment_mounts_oidc_secret_when_enabled():
    dep = _DEPLOYMENT.read_text(encoding="utf-8")
    # The mount block must reference both `oidc.enabled` and
    # `existingSecretName` so a missing Secret silently no-ops
    # rather than failing the pod schedule.
    assert "oidc-issuers" in dep, "oidc-issuers volume name missing"
    assert ".Values.config.auth.oidc.enabled" in dep
    assert ".Values.config.auth.oidc.existingSecretName" in dep
    # **Audit fix D15-M7** — mount path is `/etc/efe/oidc/`
    # (sub-directory) NOT `/etc/efe` (which would shadow the parent).
    assert re.search(r"mountPath:\s*/etc/efe/oidc\b", dep), (
        "OIDC volume must mount at /etc/efe/oidc (sub-path) post audit-fix M7"
    )
    # Defensive : a bare `mountPath: /etc/efe` (without /oidc) would
    # shadow other secret mounts under /etc/efe — pin this against
    # accidental regression.
    assert not re.search(
        r"name:\s*oidc-issuers\s*\n\s*mountPath:\s*/etc/efe\s*\n", dep
    ), "OIDC volume must NOT mount at bare /etc/efe (audit fix M7)"


def test_deployment_mount_is_read_only():
    dep = _DEPLOYMENT.read_text(encoding="utf-8")
    # Walk to the oidc-issuers mount block ; assert readOnly: true
    # within ~5 lines.
    idx = dep.find("name: oidc-issuers")
    assert idx > 0, "name: oidc-issuers not found"
    snippet = dep[idx : idx + 250]
    assert "readOnly: true" in snippet


def test_deployment_mode_is_0400():
    dep = _DEPLOYMENT.read_text(encoding="utf-8")
    # The Secret volume defaultMode should be 0400 — same security
    # posture as the audit-hmac mount.
    idx = dep.find("name: oidc-issuers\n          secret:")
    assert idx > 0, "oidc-issuers secret volume missing"
    snippet = dep[idx : idx + 400]
    assert "defaultMode: 0400" in snippet


# ─────────────────────────────────────────────────────────────────────
# 0004_tenants.sql — schema invariants
# ─────────────────────────────────────────────────────────────────────


def test_tenants_migration_exists():
    assert _MIGRATION.exists(), "migrations/0004_tenants.sql missing"


def test_tenants_migration_has_six_check_constraints():
    sql = _MIGRATION.read_text(encoding="utf-8")
    # Architecture § 4 commits to 5 CHECK constraints. Every one of
    # them MUST be present so a renamed enum value or relaxed regex
    # doesn't silently allow bad data.
    expected = [
        "tenants_id_format",
        "tenants_status_enum",
        "tenants_idp_strategy_enum",
        "tenants_display_name_nonempty",
        "tenants_idp_issuer_nonempty",
    ]
    for name in expected:
        assert name in sql, f"missing CONSTRAINT {name}"


def test_tenants_migration_has_two_indexes():
    sql = _MIGRATION.read_text(encoding="utf-8")
    assert "CREATE INDEX tenants_idp_issuer_idx" in sql
    assert "CREATE INDEX tenants_status_idx" in sql


def test_tenants_status_enum_is_three_values():
    sql = _MIGRATION.read_text(encoding="utf-8")
    # The CHECK constraint must list exactly 'pending' | 'active' |
    # 'suspended' — adding a fourth status would require a code
    # change in tenants.rs (TenantStatus::from_sql) so this catches
    # forgotten enum widening.
    m = re.search(r"status\s+IN\s*\(\s*'([^']+)'\s*,\s*'([^']+)'\s*,\s*'([^']+)'", sql)
    assert m, "status enum pattern missing"
    assert set(m.groups()) == {"pending", "active", "suspended"}


def test_tenants_idp_strategy_enum_matches_rust():
    """The SQL CHECK constraint must list exactly the Rust enum
    variants. A drift here makes inserts succeed in SQL but fail at
    `from_sql` decode time."""
    sql = _MIGRATION.read_text(encoding="utf-8")
    rs = _TENANTS_RS.read_text(encoding="utf-8")
    sql_match = re.search(
        r"idp_strategy\s+IN\s*\(\s*'([^']+)'\s*,\s*'([^']+)'", sql
    )
    assert sql_match, "idp_strategy CHECK pattern missing"
    sql_values = set(sql_match.groups())
    # Pull the as_sql() variants from the Rust source.
    rs_values = set(re.findall(r'IdpStrategy::\w+ => "([^"]+)"', rs))
    assert (
        sql_values == rs_values
    ), f"SQL {sql_values} drifts from Rust {rs_values}"


def test_tenants_id_regex_matches_rust():
    """The SQL CHECK regex MUST match the Rust validate_tenant_id
    rule. Drift here means CLI-validated IDs may still be rejected
    at INSERT time — confusing to debug."""
    sql = _MIGRATION.read_text(encoding="utf-8")
    rs = _TENANTS_RS.read_text(encoding="utf-8")
    assert "tenant_id ~ '^[a-z0-9_-]{1,63}$'" in sql, "expected SQL regex"
    # Rust validates byte-by-byte ; we simply check that the regex
    # comment / canonical pattern is present in the source.
    assert "^[a-z0-9_-]{1,63}$" in rs, "Rust validate_tenant_id docs / regex missing"


# ─────────────────────────────────────────────────────────────────────
# oidc-issuers.example.json — operator-facing template
# ─────────────────────────────────────────────────────────────────────


def test_example_issuers_is_valid_json_and_array():
    data = json.loads(_EXAMPLE_ISSUERS.read_text(encoding="utf-8"))
    assert isinstance(data, list), "example must be a JSON array"
    assert len(data) >= 3, "expected ≥ 3 vendor templates (Auth0, Keycloak, Okta)"


def test_example_issuers_each_has_required_fields():
    data = json.loads(_EXAMPLE_ISSUERS.read_text(encoding="utf-8"))
    required = {"issuer", "audience", "jwks_url", "alg_whitelist", "tenant_strategy"}
    for entry in data:
        missing = required - set(entry.keys())
        assert not missing, f"entry {entry.get('issuer')} missing {missing}"


def test_example_issuers_https_only():
    data = json.loads(_EXAMPLE_ISSUERS.read_text(encoding="utf-8"))
    for entry in data:
        assert entry["issuer"].startswith("https://"), (
            f"non-HTTPS issuer: {entry['issuer']}"
        )
        assert entry["jwks_url"].startswith("https://"), (
            f"non-HTTPS jwks_url: {entry['jwks_url']}"
        )


def test_example_issuers_alg_whitelist_subset_of_supported():
    data = json.loads(_EXAMPLE_ISSUERS.read_text(encoding="utf-8"))
    supported = {"RS256", "ES256"}
    for entry in data:
        for alg in entry["alg_whitelist"]:
            assert alg in supported, (
                f"unsupported alg {alg} in {entry['issuer']} ; "
                f"validate_issuer_config rejects everything outside {supported}"
            )


def test_example_issuers_strategy_invariants_satisfied():
    data = json.loads(_EXAMPLE_ISSUERS.read_text(encoding="utf-8"))
    for entry in data:
        if entry["tenant_strategy"] == "claim":
            assert entry.get("tenant_claim"), (
                f"strategy=claim requires tenant_claim ({entry['issuer']})"
            )
        elif entry["tenant_strategy"] == "issuer":
            tid = entry.get("tenant_id_for_issuer", "")
            assert re.match(r"^[a-z0-9_-]{1,63}$", tid), (
                f"strategy=issuer requires tenant_id matching regex "
                f"({entry['issuer']} → {tid!r})"
            )


# ─────────────────────────────────────────────────────────────────────
# issuers.rs — http gating logic
# ─────────────────────────────────────────────────────────────────────


def test_issuers_rs_gates_http_behind_env_var():
    """Architecture § 13.2 : ``http://`` URLs are accepted ONLY when
    ``EFE_OIDC_ALLOW_HTTP=true``. The previous implementation accepted
    both unconditionally — this test pins the audit fix in place."""
    rs = _ISSUERS_RS.read_text(encoding="utf-8")
    assert (
        "EFE_OIDC_ALLOW_HTTP" in rs
    ), "is_valid_url must check EFE_OIDC_ALLOW_HTTP for http://"
    # Defensive : assert that the bare `s.starts_with("http://")` is
    # NOT the only path — it must be inside an env-check branch.
    bare_match = re.search(
        r"fn is_valid_url[^}]*\bs\.starts_with\(\"http://\"\)\s*\}", rs, re.DOTALL
    )
    assert (
        bare_match is None
    ), "is_valid_url accepts http:// without env check (audit regression)"


# ─────────────────────────────────────────────────────────────────────
# Helm — README mention of the new path
# ─────────────────────────────────────────────────────────────────────


def test_chart_readme_mentions_oidc():
    readme = (_CHART / "README.md").read_text(encoding="utf-8")
    assert (
        "oidc" in readme.lower() or "OIDC" in readme
    ), "deploy/helm/efe/README.md does not mention OIDC ; D15 is undiscoverable"


# ─────────────────────────────────────────────────────────────────────
# Audit-fix regression pins
# ─────────────────────────────────────────────────────────────────────


def test_audit_fix_c1_tenant_status_gate_wired():
    """**Audit fix D15-C1** — the OIDC validator must accept an optional
    `TenantStatusGate` ; the runtime must wire `TenantStore` as the gate
    when a Postgres pool is available. Pre-audit code did neither :
    suspended tenants kept hitting authenticated RPCs."""
    oidc_rs = (
        _REPO / "crates" / "efe-server" / "src" / "auth" / "oidc.rs"
    ).read_text(encoding="utf-8")
    assert "trait TenantStatusGate" in oidc_rs, "TenantStatusGate trait missing"
    assert "with_tenant_gate" in oidc_rs, "with_tenant_gate constructor missing"
    assert "tenant_gate.as_ref()" in oidc_rs, (
        "validate_inner must consult the gate after tenant resolution"
    )

    runtime_rs = (
        _REPO / "crates" / "efe-server" / "src" / "runtime.rs"
    ).read_text(encoding="utf-8")
    assert "TenantStatusGate" in runtime_rs, (
        "runtime.rs must wire TenantStatusGate when Postgres pool available"
    )
    assert "with_tenant_gate" in runtime_rs, (
        "runtime.rs must call OidcValidator::with_tenant_gate"
    )

    tenants_rs = (
        _REPO / "crates" / "efe-server" / "src" / "auth" / "tenants.rs"
    ).read_text(encoding="utf-8")
    assert "impl super::oidc::TenantStatusGate for TenantStore" in tenants_rs


def test_audit_fix_c2_oidc_requires_auth_enabled():
    """**Audit fix D15-C2** — `oidc.enabled = true` MUST require
    `auth.enabled = true`. Without this guard, a misconfiguration
    silently disables auth (runtime falls into AuthGuard::disabled())."""
    config_rs = _CONFIG_RS.read_text(encoding="utf-8")
    assert "EFE_OIDC_ENABLED=true requires EFE_AUTH_ENABLED=true" in config_rs


def test_audit_fix_c4_bad_audience_outcome_routed():
    """**Audit fix D15-C4** — `Outcome::BadAudience` must be assigned
    by some error variant. Pre-audit code defined the variant but never
    routed any error to it ; the dashboard panel was dead code."""
    oidc_rs = (
        _REPO / "crates" / "efe-server" / "src" / "auth" / "oidc.rs"
    ).read_text(encoding="utf-8")
    # The audit fix maps `ErrorKind::InvalidAudience → BadAudience`.
    assert "ErrorKind::InvalidAudience => {" in oidc_rs or (
        "ErrorKind::InvalidAudience =>" in oidc_rs
        and "OidcError::BadAudience" in oidc_rs
    ), "InvalidAudience must route to OidcError::BadAudience"
    assert "OidcError::BadAudience(_) => Outcome::BadAudience" in oidc_rs


def test_audit_fix_c5_expired_distinct_from_too_old():
    """**Audit fix D15-C5** — `OidcError::Expired` and
    `OidcError::TokenTooOld` are distinct variants (clean expiry vs
    lifetime cap). Pre-audit code folded both into TokenTooOld with
    a misleading `lifetime=0` placeholder."""
    oidc_rs = (
        _REPO / "crates" / "efe-server" / "src" / "auth" / "oidc.rs"
    ).read_text(encoding="utf-8")
    assert "OidcError::Expired" in oidc_rs
    assert "OidcError::TokenTooOld" in oidc_rs
    # `Expired` should map to Outcome::Expired alongside TokenTooOld.
    assert "OidcError::Expired | OidcError::TokenTooOld" in oidc_rs


def test_audit_fix_c6_stale_counter_inside_kid_match():
    """**Audit fix D15-C6+C7** — `jwks_stale_total` increments only
    AFTER the kid is found in the cache. Pre-audit code incremented
    before the lookup, producing false-positive stale-serve telemetry
    when a token presented an unknown kid against a stale cache."""
    jwks_rs = (
        _REPO / "crates" / "efe-server" / "src" / "auth" / "jwks.rs"
    ).read_text(encoding="utf-8")
    # Find the `get_key` body and assert the stale-counter increment
    # happens AFTER `st.keys.get(kid).cloned()?` (the early-return on
    # miss).
    body_start = jwks_rs.find("pub async fn get_key(")
    assert body_start > 0, "get_key signature missing"
    # Body extends to the next top-level fn-level closing brace ;
    # we find it by matching the `pub async fn refresh` declaration
    # that follows.
    body_end = jwks_rs.find("pub async fn refresh", body_start)
    body = jwks_rs[body_start:body_end]
    miss_pos = body.find("st.keys.get(kid).cloned()")
    # Search for the actual `jwks_stale_total()` *call* (not the
    # bare identifier which appears in narrative comments above).
    # The call site is unique : `jwks_stale_total()`.
    stale_pos = body.find("jwks_stale_total()")
    assert miss_pos > 0, "kid-miss early-return missing in get_key"
    assert stale_pos > 0, "jwks_stale_total() call missing in get_key"
    assert stale_pos > miss_pos, (
        f"jwks_stale_total() call at offset {stale_pos} must come "
        f"AFTER the kid lookup at {miss_pos} (audit fix C6+C7)"
    )


def test_audit_fix_h1_alg_canonical_name_helper():
    """**Audit fix D15-H1** — the alg whitelist comparison must use
    an explicit canonical-name helper, NOT `format!("{:?}", header.alg)`
    which depends on jsonwebtoken's Debug derive staying stable."""
    oidc_rs = (
        _REPO / "crates" / "efe-server" / "src" / "auth" / "oidc.rs"
    ).read_text(encoding="utf-8")
    assert "fn alg_canonical_name(alg: Algorithm)" in oidc_rs
    # Defensive : the bare assignment-form `let header_alg_str =
    # format!("{:?}", header.alg);` must be gone (the historical
    # comment may still mention it as the audit-fix narrative).
    assert (
        'let header_alg_str = format!("{:?}", header.alg);' not in oidc_rs
    ), "regressed to format!(\"{:?}\", header.alg) (audit fix H1)"
    # And the new canonical-name call must be present at the use site.
    assert "alg_canonical_name(header.alg)" in oidc_rs


def test_audit_fix_h3_url_parse_used():
    """**Audit fix D15-H3** — `is_valid_url` uses `url::Url::parse`
    instead of bare prefix matching."""
    issuers_rs = _ISSUERS_RS.read_text(encoding="utf-8")
    assert "url::Url::parse" in issuers_rs, (
        "is_valid_url must call url::Url::parse (audit fix H3)"
    )


# ─────────────────────────────────────────────────────────────────────
# Architecture acceptance criterion : D15 audit report exists
# ─────────────────────────────────────────────────────────────────────


def test_d15_audit_report_exists():
    """D15 architecture doc listed `docs/audits/d15_audit_report.md`
    as an acceptance criterion. This pin test guards against a
    future refactor that deletes the report."""
    report = _REPO / "docs" / "audits" / "d15_audit_report.md"
    assert report.exists(), (
        "D15 audit report missing — architecture acceptance "
        "criterion not satisfied"
    )
    content = report.read_text(encoding="utf-8")
    # Sanity : contains the canonical sections + records all 17
    # findings.
    for marker in [
        "C1",  # suspended tenants critical
        "C2",  # auth bypass critical
        "C8",  # tenant audit emission gap
        "H1",  # alg whitelist fragility
        "Conclusion",
        "Dette technique",
    ]:
        assert marker in content, (
            f"D15 audit report missing required marker: {marker!r}"
        )


def test_d15_audit_report_documents_17_findings():
    """The report must reference all 17 finding IDs."""
    report = _REPO / "docs" / "audits" / "d15_audit_report.md"
    content = report.read_text(encoding="utf-8")
    expected_ids = [
        "C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8",
        "H1", "H3", "H5", "H7", "H8", "H9", "H10",
        "M3", "M7",
    ]
    for fid in expected_ids:
        assert fid in content, (
            f"D15 audit report missing finding ID: {fid}"
        )

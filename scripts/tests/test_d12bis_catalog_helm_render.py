"""Sprint D12-bis — Helm chart catalog wiring smoke.

Pin tests :

* ``values.yaml`` exposes a ``config.catalog`` block with
  ``backend`` + 4 quotas.
* ``templates/configmap.yaml`` renders the 5
  ``EFE_CATALOG_*`` env vars.
* ``templates/tests/test-catalog-reachable.yaml`` exists and uses
  ``grpc_health_probe -service=efe.v1.catalog.CatalogService``.

These tests do NOT require a live cluster — they read the chart
files and assert structural properties. They catch the most
common regressions :

  * "I bumped a quota in values.yaml but forgot to render it
    in configmap.yaml".
  * "I dropped the catalog reachability test thinking it's
    redundant with the generic gRPC health probe — it isn't".
"""

from __future__ import annotations

from pathlib import Path

import yaml

_REPO = Path(__file__).resolve().parents[2]
_CHART = _REPO / "deploy" / "helm" / "efe"
_VALUES = _CHART / "values.yaml"
_CONFIGMAP = _CHART / "templates" / "configmap.yaml"
_TEST_REACHABLE = _CHART / "templates" / "tests" / "test-catalog-reachable.yaml"


# ─────────────────────────────────────────────────────────────────────
# values.yaml — the structural source of truth.
# ─────────────────────────────────────────────────────────────────────


def test_values_yaml_has_config_catalog_block():
    """The ``config.catalog`` block is the single authoritative
    place for choosing the backend + quotas. If it's missing, the
    chart user cannot tune catalog limits per environment."""
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    cat = data["config"]["catalog"]
    assert "backend" in cat
    assert cat["backend"] in {"memory", "postgres"}, (
        f"unexpected backend value: {cat['backend']!r}"
    )
    for key in (
        "maxNamesPerTenant",
        "maxVersionsPerName",
        "maxClassesPerCatalog",
        "maxSentencesPerClass",
    ):
        assert key in cat, f"missing quota {key} in config.catalog"
        assert isinstance(cat[key], int), (
            f"config.catalog.{key} must be an int, got {type(cat[key])}"
        )
        assert cat[key] > 0, (
            f"config.catalog.{key} must be > 0, got {cat[key]}"
        )


def test_values_yaml_default_backend_is_memory():
    """Until D14 lands the postgres backend, the chart default
    MUST be ``memory`` ; ``postgres`` would make the binary fail
    fast at boot.  Catching this drift early avoids painful
    helm-upgrade-then-CrashLoopBackOff incidents."""
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["config"]["catalog"]["backend"] == "memory"


def test_values_yaml_quotas_match_efe_server_defaults():
    """The chart-side defaults MUST match the in-binary defaults in
    ``crates/efe-server/src/catalog/policy.rs::DEFAULT_*`` so a
    bare-bones ``helm install`` behaves identically to ``cargo run``.

    Audit D12-bis fix C2 : the previous values (64/32/64/32) were
    invented chart-side and didn't match any of the in-binary
    constants (100/1000/256/64), creating silent drift between
    Tier-1 deploys and local dev.  The drift was undetectable
    because :

      * ``runtime.rs::build`` ignored config.catalog.policy entirely
        (audit fix C1) — quotas were *always* the binary defaults.
      * This very test hardcoded the chart's invented values as
        "in-binary defaults" — it passed for the wrong reason.

    To prevent this from regressing, the test now reads the actual
    Rust constants from ``policy.rs`` and asserts equality.  If
    someone bumps a binary default without updating values.yaml,
    or vice-versa, the test fails.
    """
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    cat = data["config"]["catalog"]

    rust_defaults = _read_policy_defaults_from_rust()
    expected_chart_keys = {
        "maxNamesPerTenant": rust_defaults["DEFAULT_MAX_NAMES_PER_TENANT"],
        "maxVersionsPerName": rust_defaults["DEFAULT_MAX_VERSIONS_PER_NAME"],
        "maxClassesPerCatalog": rust_defaults["DEFAULT_MAX_CLASSES_PER_CATALOG"],
        "maxSentencesPerClass": rust_defaults["DEFAULT_MAX_SENTENCES_PER_CLASS"],
    }
    for key, val in expected_chart_keys.items():
        assert cat[key] == val, (
            f"config.catalog.{key} drift : chart={cat[key]} vs "
            f"in-binary default={val} ; either the chart needs to be "
            f"updated to match policy.rs::DEFAULT_*, or the policy.rs "
            f"constants were changed without a chart update."
        )


def _read_policy_defaults_from_rust() -> dict[str, int]:
    """Parse the four ``pub const DEFAULT_*`` declarations out of
    ``crates/efe-server/src/catalog/policy.rs``.

    We grep the source rather than calling ``cargo`` so this test
    has no Rust toolchain dependency in CI ; the parser is permissive
    enough to survive whitespace tweaks but strict enough to refuse
    silent renames (it requires the exact constant names).
    """
    import re

    policy_rs = _REPO / "crates" / "efe-server" / "src" / "catalog" / "policy.rs"
    text = policy_rs.read_text(encoding="utf-8")
    pattern = re.compile(
        r"pub\s+const\s+(DEFAULT_MAX_[A-Z_]+):\s*u32\s*=\s*(\d+)\s*;",
    )
    found = {m.group(1): int(m.group(2)) for m in pattern.finditer(text)}
    required = {
        "DEFAULT_MAX_NAMES_PER_TENANT",
        "DEFAULT_MAX_VERSIONS_PER_NAME",
        "DEFAULT_MAX_CLASSES_PER_CATALOG",
        "DEFAULT_MAX_SENTENCES_PER_CLASS",
    }
    missing = required - set(found)
    assert not missing, (
        f"policy.rs is missing the expected constants : {sorted(missing)} "
        f"; this test must be updated, or the constants were renamed."
    )
    return found


# ─────────────────────────────────────────────────────────────────────
# configmap.yaml — the env var rendering.
# ─────────────────────────────────────────────────────────────────────


def test_configmap_renders_5_efe_catalog_env_vars():
    """Every ``config.catalog.*`` value MUST be rendered as an
    ``EFE_CATALOG_*`` env var, otherwise the binary won't see
    the chart-supplied value."""
    text = _CONFIGMAP.read_text(encoding="utf-8")
    expected_keys = {
        "EFE_CATALOG_BACKEND",
        "EFE_CATALOG_MAX_NAMES_PER_TENANT",
        "EFE_CATALOG_MAX_VERSIONS_PER_NAME",
        "EFE_CATALOG_MAX_CLASSES_PER_CATALOG",
        "EFE_CATALOG_MAX_SENTENCES_PER_CLASS",
    }
    for key in expected_keys:
        assert f"{key}:" in text, (
            f"configmap.yaml missing env var {key} ; "
            f"is the catalog block rendered ?"
        )


def test_configmap_catalog_block_uses_values_indirection():
    """Each catalog env var should be sourced from
    ``.Values.config.catalog.*`` ; hard-coding values would
    silently override operator overrides."""
    text = _CONFIGMAP.read_text(encoding="utf-8")
    assert "{{ .Values.config.catalog.backend" in text
    assert "{{ .Values.config.catalog.maxNamesPerTenant" in text
    assert "{{ .Values.config.catalog.maxVersionsPerName" in text
    assert "{{ .Values.config.catalog.maxClassesPerCatalog" in text
    assert "{{ .Values.config.catalog.maxSentencesPerClass" in text


# ─────────────────────────────────────────────────────────────────────
# test-catalog-reachable.yaml — the helm test pod.
# ─────────────────────────────────────────────────────────────────────


def test_catalog_reachable_test_template_exists():
    """The chart MUST ship a helm test pod that probes the
    catalog service health entry. Without this, ``helm install``
    succeeding ⇒ catalog wired is NOT a verifiable invariant."""
    assert _TEST_REACHABLE.exists(), (
        f"missing helm test : {_TEST_REACHABLE.relative_to(_REPO)}"
    )


def test_catalog_reachable_uses_per_service_health_probe():
    """The probe MUST target the *catalog-specific* service entry
    in the gRPC health reporter ; the generic
    ``grpc.health.v1.Health/Check`` for the whole pod would be
    SERVING regardless of whether catalog is attached or not."""
    text = _TEST_REACHABLE.read_text(encoding="utf-8")
    assert "/usr/local/bin/grpc_health_probe" in text
    assert "-service=efe.v1.catalog.CatalogService" in text, (
        "the per-service flag MUST target efe.v1.catalog.CatalogService "
        "exactly ; otherwise the test passes trivially."
    )


def test_catalog_reachable_test_is_opt_in_via_values_toggle():
    """The template should be guarded by
    ``.Values.tests.catalogReachable.enabled`` so operators on
    air-gapped clusters can opt out."""
    text = _TEST_REACHABLE.read_text(encoding="utf-8")
    assert ".Values.tests.catalogReachable.enabled" in text


def test_catalog_reachable_test_default_enabled_true():
    """Unlike the bench-image-dependent helm tests
    (``semanticSmoke``, ``crossTenantDenial``) which default to
    ``false`` because they require a published bench image,
    the catalog reachability test uses the bundled
    ``grpc_health_probe`` already in the server image and SHOULD
    default to ``true`` so every ``helm install`` exercises it."""
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["tests"]["catalogReachable"]["enabled"] is True

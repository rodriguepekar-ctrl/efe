"""efe_check_bundle_compatibility pin tests (M18 D47)."""
from __future__ import annotations

import pytest

from mcp_server.bundle.compat import _parse_semver, _semver_le, check_compatibility
from mcp_server.bundle.manifest import EFEBundleManifest


def _manifest(**kwargs) -> EFEBundleManifest:
    base = dict(
        schema_version=2,
        media_type="application/vnd.oci.image.manifest.v1+json",
        config_digest="sha256:" + "0" * 64,
        layer_count=0,
        total_layer_size=0,   # M18-D47-A2 fix added this field.
        slug="x",
        version="v1.0.0",
        framework="hash",
        model_name=None,
        dim=384,
        is_dev_unsigned=False,
        bundle_schema_version="1.0",
        produced_at=None,
        produced_by=None,
        min_efe_server_version=None,
        max_efe_server_version=None,
        required_features=tuple(),
        sbom_digest=None,
        manifest_sha256="a" * 64,
    )
    base.update(kwargs)
    return EFEBundleManifest(**base)


def test_semver_parse_basic():
    assert _parse_semver("1.0.0") == (1, 0, 0, "")
    assert _parse_semver("v1.2.3") == (1, 2, 3, "")
    assert _parse_semver("v1.2.3-rc1") == (1, 2, 3, "rc1")
    assert _parse_semver("not-a-semver") is None


def test_semver_le_orders_correctly():
    assert _semver_le("1.0.0", "1.0.1") is True
    assert _semver_le("1.0.1", "1.0.0") is False
    assert _semver_le("1.0.0", "1.0.0") is True
    assert _semver_le("1.0.0-rc1", "1.0.0") is True


def test_compat_compatible_when_all_match():
    m = _manifest(
        min_efe_server_version="1.0.0",
        max_efe_server_version=None,
        dim=384,
        required_features=("multilingual",),
    )
    result = check_compatibility(
        manifest=m,
        efe_server_version="1.2.0",
        efe_server_features=["multilingual", "drift_detection"],
        efe_server_dim=384,
    )
    assert result.compatible is True
    assert result.blocking_issues == []


def test_compat_incompatible_when_dim_mismatch():
    m = _manifest(dim=384)
    result = check_compatibility(
        manifest=m,
        efe_server_version="1.2.0",
        efe_server_dim=768,
    )
    assert result.compatible is False
    assert any("dim" in b for b in result.blocking_issues)


def test_compat_blocking_when_min_version_too_low():
    m = _manifest(min_efe_server_version="2.0.0")
    result = check_compatibility(
        manifest=m,
        efe_server_version="1.2.0",
        efe_server_dim=384,
    )
    assert result.compatible is False


def test_compat_blocking_when_features_missing():
    m = _manifest(required_features=("rare-feature",))
    result = check_compatibility(
        manifest=m,
        efe_server_version="1.2.0",
        efe_server_features=[],
    )
    assert result.compatible is False
    assert any("rare-feature" in b for b in result.blocking_issues)


def test_compat_warns_on_dev_unsigned():
    m = _manifest(is_dev_unsigned=True)
    result = check_compatibility(
        manifest=m,
        efe_server_version="1.2.0",
        efe_server_dim=384,
    )
    assert any("is_dev_unsigned" in w for w in result.warnings)

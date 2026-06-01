"""Bundle vs EFE-server compatibility checks (M18 D47).

Cross-checks the manifest annotations
  - min_efe_server_version
  - max_efe_server_version
  - dim
  - required_features

against the live `/info` payload of the EFE backend.
"""
from __future__ import annotations

import re
from dataclasses import dataclass

from .manifest import EFEBundleManifest


@dataclass(frozen=True)
class CompatibilityCheckItem:
    name: str
    required: object | None
    actual: object | None
    ok: bool
    message: str = ""


@dataclass(frozen=True)
class CompatibilityCheck:
    compatible: bool
    checks: list[CompatibilityCheckItem]
    warnings: list[str]
    blocking_issues: list[str]


# ─────────────────────────────────────────────────────────────────────
# Semver helpers — minimal subset (no pre-release ordering edge cases)
# ─────────────────────────────────────────────────────────────────────


_SEMVER_RE = re.compile(
    r"^v?(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)"
    r"(?:-(?P<pre>[A-Za-z0-9\-.]+))?$"
)


def _parse_semver(value: str) -> tuple[int, int, int, str] | None:
    m = _SEMVER_RE.match(value.strip())
    if not m:
        return None
    return (
        int(m.group("major")),
        int(m.group("minor")),
        int(m.group("patch")),
        m.group("pre") or "",
    )


def _semver_le(a: str, b: str) -> bool | None:
    """Return True if `a <= b`, False if a > b, None if unparseable."""
    pa = _parse_semver(a)
    pb = _parse_semver(b)
    if pa is None or pb is None:
        return None
    if pa[:3] != pb[:3]:
        return pa[:3] <= pb[:3]
    # Pre-release strings : `a-rc1` < `a` (per semver).
    if pa[3] == "" and pb[3] != "":
        return False
    if pa[3] != "" and pb[3] == "":
        return True
    return pa[3] <= pb[3]


# ─────────────────────────────────────────────────────────────────────
# Public API
# ─────────────────────────────────────────────────────────────────────


def check_compatibility(
    *,
    manifest: EFEBundleManifest,
    efe_server_version: str,
    efe_server_features: list[str] | tuple[str, ...] | None = None,
    efe_server_dim: int | None = None,
) -> CompatibilityCheck:
    """Cross-check a bundle manifest against the EFE backend `/info`."""
    checks: list[CompatibilityCheckItem] = []
    warnings: list[str] = []
    blocking: list[str] = []
    server_features = list(efe_server_features or [])

    # min_efe_server_version
    min_v = manifest.min_efe_server_version
    if min_v:
        cmp = _semver_le(min_v, efe_server_version)
        ok = cmp is True
        msg = (
            ""
            if ok
            else (
                f"server version {efe_server_version!r} is below the "
                f"required minimum {min_v!r}"
                if cmp is False
                else f"could not parse semver: required={min_v!r} or actual={efe_server_version!r}"
            )
        )
        checks.append(
            CompatibilityCheckItem(
                name="min_efe_server_version",
                required=min_v,
                actual=efe_server_version,
                ok=ok,
                message=msg,
            )
        )
        if not ok:
            blocking.append(msg or "min_efe_server_version not satisfied")

    # max_efe_server_version
    max_v = manifest.max_efe_server_version
    if max_v:
        cmp = _semver_le(efe_server_version, max_v)
        ok = cmp is True
        msg = (
            ""
            if ok
            else (
                f"server version {efe_server_version!r} exceeds max {max_v!r}"
                if cmp is False
                else f"could not parse semver: required={max_v!r} or actual={efe_server_version!r}"
            )
        )
        checks.append(
            CompatibilityCheckItem(
                name="max_efe_server_version",
                required=max_v,
                actual=efe_server_version,
                ok=ok,
                message=msg,
            )
        )
        if not ok:
            blocking.append(msg or "max_efe_server_version not satisfied")

    # required_dim
    if manifest.dim is not None and efe_server_dim is not None:
        ok = manifest.dim == efe_server_dim
        checks.append(
            CompatibilityCheckItem(
                name="required_dim",
                required=manifest.dim,
                actual=efe_server_dim,
                ok=ok,
                message=(
                    "" if ok else
                    f"bundle dim={manifest.dim} != server dim={efe_server_dim}"
                ),
            )
        )
        if not ok:
            blocking.append(
                f"dim mismatch: bundle={manifest.dim}, server={efe_server_dim}"
            )

    # required_features
    if manifest.required_features:
        missing = [f for f in manifest.required_features if f not in server_features]
        ok = not missing
        checks.append(
            CompatibilityCheckItem(
                name="required_features",
                required=list(manifest.required_features),
                actual=server_features,
                ok=ok,
                message=(
                    "" if ok else
                    f"server missing features: {missing}"
                ),
            )
        )
        if not ok:
            blocking.append(f"server missing features: {missing}")

    # bundle_schema_version
    if manifest.bundle_schema_version:
        ok = manifest.bundle_schema_version in ("1.0", "1.1")
        checks.append(
            CompatibilityCheckItem(
                name="schema_version",
                required="1.0|1.1",
                actual=manifest.bundle_schema_version,
                ok=ok,
                message=(
                    "" if ok else
                    f"bundle schema version {manifest.bundle_schema_version!r} not in supported set"
                ),
            )
        )
        if not ok:
            warnings.append(
                f"unfamiliar bundle schema version {manifest.bundle_schema_version!r}"
            )

    # is_dev_unsigned
    if manifest.is_dev_unsigned:
        warnings.append(
            "bundle is marked is_dev_unsigned=true — production deployment is discouraged"
        )

    return CompatibilityCheck(
        compatible=len(blocking) == 0,
        checks=checks,
        warnings=warnings,
        blocking_issues=blocking,
    )

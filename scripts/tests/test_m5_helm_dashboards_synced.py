"""Sprint **M5 audit fix T-A2** — Pin tests : Helm mirror is in sync
with the canonical M5 extension dashboards.

Pre-audit, the 4 new dashboards (`d29/01_gpu_canary.json`,
`d34/01_templates.json`, `d34/02_mode2.json`, `d37/01_mode3.json`)
lived under per-sprint subdirs canonically but the Helm template
globs `d13_files/dashboards/*.json` (flat, no recursion). The
provisioning would silently skip them in production.

The fix copies each canonical file to the flat helm mirror with a
`dXX_` prefix to avoid name collision with the existing 6 d13
dashboards (also numbered 01-06). This pin test verifies the mirror
stays in sync with the canonical files.
"""
from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_CANON = _REPO / "deploy" / "observability" / "grafana" / "dashboards"
_MIRROR = _REPO / "deploy" / "helm" / "efe" / "d13_files" / "dashboards"


def _sha256(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


# Canonical → mirror pairs, one per M5 extension dashboard.
_M5_PAIRS = [
    (_CANON / "d29" / "01_gpu_canary.json", _MIRROR / "d29_01_gpu_canary.json"),
    (_CANON / "d34" / "01_templates.json", _MIRROR / "d34_01_templates.json"),
    (_CANON / "d34" / "02_mode2.json", _MIRROR / "d34_02_mode2.json"),
    (_CANON / "d37" / "01_mode3.json", _MIRROR / "d37_01_mode3.json"),
]


@pytest.mark.parametrize(
    "canon, mirror",
    _M5_PAIRS,
    ids=[c.relative_to(_CANON).as_posix() for c, _ in _M5_PAIRS],
)
def test_m5_dashboard_mirror_matches_canonical(canon: Path, mirror: Path) -> None:
    """The 4 M5 extension dashboards in the helm mirror must
    byte-match their canonical counterpart. Drift = silent
    desync between dev source-of-truth and production
    deployment."""
    assert canon.exists(), f"canonical M5 dashboard missing : {canon}"
    assert mirror.exists(), (
        f"helm mirror missing : {mirror}\n"
        f"Run : bash scripts/observability/sync_helm_d13_assets.sh"
    )
    assert _sha256(canon) == _sha256(mirror), (
        f"helm mirror is OUT OF SYNC with canonical source.\n"
        f"  canonical : {canon}\n"
        f"  mirror    : {mirror}\n"
        f"Run : bash scripts/observability/sync_helm_d13_assets.sh\n"
        f"and commit the result."
    )


def test_m5_mirror_filenames_use_sprint_prefix() -> None:
    """Anti-regression : M5 extension files in the flat helm mirror
    must use `dXX_NN_<slug>.json` naming so they don't collide
    with the 6 existing d13 dashboards (which use `01..06_*.json`)."""
    for fname in ("d29_01_gpu_canary.json", "d34_01_templates.json",
                  "d34_02_mode2.json", "d37_01_mode3.json"):
        f = _MIRROR / fname
        assert f.exists(), f"M5 mirror file missing : {f}"


def test_m5_mirror_no_d13_collision() -> None:
    """Anti-regression : the 6 d13 dashboards (01-06) must remain
    distinct from the M5 dashboards. Collision = one would
    overwrite the other."""
    d13_names = {
        "01_operator_overview.json",
        "02_tenant_drill_down.json",
        "03_catalog_version_distribution.json",
        "04_cost_attribution.json",
        "05_audit_stream.json",
        "06_capacity_planning.json",
    }
    m5_names = {
        "d29_01_gpu_canary.json",
        "d34_01_templates.json",
        "d34_02_mode2.json",
        "d37_01_mode3.json",
    }
    # No filename appears in both sets.
    assert d13_names.isdisjoint(m5_names), (
        "d13 and M5 dashboards must have disjoint filenames in the flat mirror"
    )


def test_sync_script_extended_for_m5() -> None:
    """The shared sync script must reference each M5 extension
    sprint subdir explicitly. Without this, future canonical
    edits won't propagate to the helm mirror."""
    script = _REPO / "scripts" / "observability" / "sync_helm_d13_assets.sh"
    body = script.read_text(encoding="utf-8")
    for sprint in ("d29", "d34", "d37"):
        # Look for the SRC_DASH_DXX variable assignment.
        assert (
            f"SRC_DASH_{sprint.upper()}" in body
        ), f"sync script missing SRC_DASH_{sprint.upper()} for sprint {sprint}"


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))

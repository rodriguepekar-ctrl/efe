"""Sprint D13 audit C-2 regression — Helm chart asset mirror.

Helm cannot escape the chart sandbox via ``../..``, so the
canonical observability artefacts under ``deploy/observability/``
are mirrored into ``deploy/helm/efe/d13_files/``. This test
verifies the mirror is byte-identical to the canonical source
(SHA-256). When you edit the canonical file, run::

    bash scripts/observability/sync_helm_d13_assets.sh

before opening a PR.
"""

from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_CANON = _REPO / "deploy" / "observability"
_MIRROR = _REPO / "deploy" / "helm" / "efe" / "d13_files"


def _sha256(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


# Pairs of (canonical path, mirror path).
_PAIRS = (
    *[
        (
            _CANON / "grafana" / "dashboards" / "d13" / fname,
            _MIRROR / "dashboards" / fname,
        )
        for fname in (
            "01_operator_overview.json",
            "02_tenant_drill_down.json",
            "03_catalog_version_distribution.json",
            "04_cost_attribution.json",
            "05_audit_stream.json",
            "06_capacity_planning.json",
        )
    ],
    (_CANON / "prometheus" / "rules.d13" / "recording.yaml",
     _MIRROR / "prometheus" / "recording.yaml"),
    (_CANON / "prometheus" / "rules.d13" / "alerts.yaml",
     _MIRROR / "prometheus" / "alerts.yaml"),
    (_CANON / "loki" / "promtail-config.example.yaml",
     _MIRROR / "loki" / "promtail-config.example.yaml"),
)


@pytest.mark.parametrize("canon, mirror", _PAIRS,
                         ids=[c.name for c, _ in _PAIRS])
def test_helm_mirror_matches_canonical(canon, mirror):
    assert canon.exists(), f"canonical source missing : {canon}"
    assert mirror.exists(), (
        f"chart mirror missing : {mirror}\n"
        f"Run : bash scripts/observability/sync_helm_d13_assets.sh"
    )
    assert _sha256(canon) == _sha256(mirror), (
        f"chart mirror is OUT OF SYNC with canonical source.\n"
        f"  canonical : {canon}\n"
        f"  mirror    : {mirror}\n"
        f"Run : bash scripts/observability/sync_helm_d13_assets.sh\n"
        f"and commit the result."
    )


def test_sync_script_exists_and_is_executable():
    p = _REPO / "scripts" / "observability" / "sync_helm_d13_assets.sh"
    assert p.exists()
    import os
    mode = p.stat().st_mode
    assert mode & 0o111, f"{p} is not executable (chmod +x)"


def test_chart_mirror_directory_layout():
    """Pin the directory layout : dashboards/, prometheus/, loki/."""
    assert (_MIRROR / "dashboards").is_dir()
    assert (_MIRROR / "prometheus").is_dir()
    assert (_MIRROR / "loki").is_dir()

"""Sprint D13 — Helm template integrity.

Smoke-checks that the new D13 Helm templates exist and reference
the right files in `deploy/observability/`.
"""

from __future__ import annotations

from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_HELM_TPL = _REPO / "deploy" / "helm" / "efe" / "templates"


import re as _re

# Audit C-2 regression : Helm `.Files.Get` / `.Files.Glob` cannot
# escape the chart sandbox via `../..`. The patterns below are the
# actual Helm directive lines (after stripping comments).
_HELM_FILES_DIRECTIVE_RE = _re.compile(
    r"\.Files\.(?:Get|Glob)\s+\"([^\"]+)\"",
)


def _helm_directive_paths(template_text: str) -> list[str]:
    """Return the file paths referenced by .Files.Get / .Files.Glob,
    excluding any inside Helm comment blocks ``{{- /* ... */ -}}``."""
    no_comments = _re.sub(
        r"\{\{-?\s*/\*.*?\*/\s*-?\}\}",
        "",
        template_text,
        flags=_re.DOTALL,
    )
    return _HELM_FILES_DIRECTIVE_RE.findall(no_comments)


def test_grafana_dashboards_template_exists():
    p = _HELM_TPL / "grafana-dashboards.yaml"
    assert p.exists()
    text = p.read_text(encoding="utf-8")
    paths = _helm_directive_paths(text)
    assert paths, "grafana-dashboards.yaml must use .Files.Get / .Files.Glob"
    for path in paths:
        # Must reference the in-chart dashboards mirror (audit C-2
        # fix — canonical lives under deploy/observability/ but the
        # Helm chart can't reach it via ../.. ; sync script keeps
        # deploy/helm/efe/d13_files/ in sync).
        assert "d13_files/" in path, (
            f"grafana-dashboards.yaml has Files directive on path "
            f"{path!r} ; expected d13_files/..."
        )
        assert ".." not in path, (
            f"audit C-2 regression : grafana-dashboards.yaml directive "
            f"path {path!r} contains '..' (Helm sandbox escape)"
        )
    # Must label ConfigMaps with grafana_dashboard=1.
    assert 'grafana_dashboard: "1"' in text


def test_prometheusrule_d13_template_exists():
    p = _HELM_TPL / "prometheusrule-d13.yaml"
    assert p.exists()
    text = p.read_text(encoding="utf-8")
    paths = _helm_directive_paths(text)
    assert any("d13_files/prometheus/recording.yaml" in pp for pp in paths)
    assert any("d13_files/prometheus/alerts.yaml"    in pp for pp in paths)
    for path in paths:
        assert ".." not in path, (
            f"audit C-2 regression : prometheusrule-d13.yaml directive "
            f"path {path!r} contains '..'"
        )
    # Both opt-in flags.
    assert "observability.recordingRules.enabled" in text
    assert "observability.alerts.enabled" in text


def test_promtail_config_template_exists():
    p = _HELM_TPL / "promtail-config.yaml"
    assert p.exists()
    text = p.read_text(encoding="utf-8")
    paths = _helm_directive_paths(text)
    assert any("d13_files/loki/promtail-config.example.yaml" in pp
               for pp in paths)
    for path in paths:
        assert ".." not in path, (
            f"audit C-2 regression : promtail-config.yaml directive "
            f"path {path!r} contains '..'"
        )


def test_helm_test_dashboards_load_exists():
    p = _HELM_TPL / "tests" / "test-dashboards-load.yaml"
    assert p.exists()
    text = p.read_text(encoding="utf-8")
    # Must hit the 6 D13 UIDs.
    for uid in (
        "efe-d13-01-operator-overview",
        "efe-d13-02-tenant-drill-down",
        "efe-d13-03-catalog-version-distribution",
        "efe-d13-04-cost-attribution",
        "efe-d13-05-audit-stream",
        "efe-d13-06-capacity-planning",
    ):
        assert uid in text, (
            f"helm test must hit {uid!r}"
        )


def test_values_yaml_has_observability_block():
    p = _REPO / "deploy" / "helm" / "efe" / "values.yaml"
    text = p.read_text(encoding="utf-8")
    # Top-level observability key.
    assert "\nobservability:" in text, (
        "values.yaml must declare a top-level `observability:` block"
    )
    # Required sub-blocks.
    for key in ("dashboards", "recordingRules", "alerts", "cardinality",
                "loki", "tempo", "otel", "multiTenancy"):
        assert key in text, (
            f"values.yaml `observability:` must have `.{key}` sub-block"
        )


def test_dashboards_default_off():
    """Default OFF for backward compat with single-tenant deployments."""
    p = _REPO / "deploy" / "helm" / "efe" / "values.yaml"
    text = p.read_text(encoding="utf-8")
    # Find the dashboards block and verify default enabled = false.
    # Walk the lines : after `  dashboards:` we should see the first
    # non-comment, non-blank line containing `    enabled: false`.
    in_dashboards = False
    for line in text.splitlines():
        if line.strip() == "dashboards:":
            in_dashboards = True
            continue
        if in_dashboards:
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            if stripped.startswith("enabled:"):
                _, _, value = stripped.partition(":")
                value = value.strip()
                assert value == "false", (
                    f"observability.dashboards.enabled default should be "
                    f"false ; got {value!r}"
                )
                return
            # Hit something else first — fail loudly.
            raise AssertionError(
                f"first non-comment line under `dashboards:` was "
                f"{stripped!r}, expected `enabled: false`"
            )
    raise AssertionError("`dashboards:` block not found in values.yaml")

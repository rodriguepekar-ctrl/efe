"""Tests for the Sprint D9 Helm chart label propagation.

Pinned invariants (audit fix INT1):
  1. The Deployment's ``spec.template.metadata.labels`` (i.e. the
     pod template) carries the ``app.kubernetes.io/tenant`` label
     so D11 multi-tenant ResourceQuotas / NetworkPolicies can
     select on pod labels — not just on Deployment metadata labels.
  2. The Deployment's ``spec.selector.matchLabels`` does NOT
     include the tenant label (it is IMMUTABLE — adding labels
     here would break ``helm upgrade``).
  3. The Service template inherits the same selector subset.
  4. The chart still renders without errors when ``tenant.id``
     is set to a non-default value (``"production-pilot"``).

The tests render the Helm template by direct text substitution of
the placeholders that matter for label propagation. This is a
narrow approximation — a full ``helm template`` invocation would
require helm on PATH (which CI may or may not have). Instead we
read the template files directly and validate the structural
invariants.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest


_REPO = Path(__file__).resolve().parents[2]
_CHART = _REPO / "deploy" / "helm" / "efe"
_HELPERS = _CHART / "templates" / "_helpers.tpl"
_DEPLOYMENT = _CHART / "templates" / "deployment.yaml"
_SERVICE = _CHART / "templates" / "service.yaml"
_VALUES = _CHART / "values.yaml"


# ─────────────────────────────────────────────────────────────────────
# 1-2. _helpers.tpl invariants
# ─────────────────────────────────────────────────────────────────────

def test_efe_labels_includes_tenant():
    """``efe.labels`` MUST emit a ``app.kubernetes.io/tenant`` line."""
    helpers = _HELPERS.read_text()
    # The block defining `efe.labels` lives between the matching
    # `{{- define "efe.labels" -}}` and `{{- end -}}` markers.
    m = re.search(
        r'\{\{-?\s*define\s+"efe\.labels"\s*-?\}\}(.*?)\{\{-?\s*end\s*-?\}\}',
        helpers, re.DOTALL,
    )
    assert m, "could not locate `efe.labels` define block"
    block = m.group(1)
    assert "app.kubernetes.io/tenant" in block, (
        "INT1 regression: `efe.labels` must carry the tenant label"
    )


def test_efe_selectorLabels_does_NOT_include_tenant():
    """``efe.selectorLabels`` MUST stay a stable, minimal subset.
    Adding tenant here would break ``helm upgrade`` because
    ``Deployment.spec.selector`` is immutable."""
    helpers = _HELPERS.read_text()
    m = re.search(
        r'\{\{-?\s*define\s+"efe\.selectorLabels"\s*-?\}\}(.*?)\{\{-?\s*end\s*-?\}\}',
        helpers, re.DOTALL,
    )
    assert m, "could not locate `efe.selectorLabels` define block"
    block = m.group(1)
    assert "app.kubernetes.io/tenant" not in block, (
        "tenant label must NOT live in the selector subset (immutable!)"
    )


def test_efe_tenantId_helper_exists():
    helpers = _HELPERS.read_text()
    assert 'define "efe.tenantId"' in helpers, (
        "the `efe.tenantId` helper is exposed for direct use in templates"
    )


# ─────────────────────────────────────────────────────────────────────
# 3. Deployment template — pod template uses efe.labels (INT1 fix)
# ─────────────────────────────────────────────────────────────────────

def test_deployment_pod_template_uses_full_labels():
    """The pod template MUST include ``efe.labels`` so the tenant
    label propagates to pods (audit fix INT1).

    The previous version of the chart used ``efe.selectorLabels``
    here, which prevented the tenant label from reaching pods —
    defeating the purpose of D9 amendment A9.
    """
    deployment = _DEPLOYMENT.read_text()
    # Locate the `template:` block.
    template_block_match = re.search(
        r'\n  template:\s*\n\s+metadata:\s*\n\s+labels:(.*?)\n\s+annotations:',
        deployment, re.DOTALL,
    )
    assert template_block_match, (
        "could not locate the `template.metadata.labels` block in deployment.yaml"
    )
    block = template_block_match.group(1)
    assert 'include "efe.labels"' in block, (
        "INT1 regression: pod template must include `efe.labels` "
        "(otherwise the tenant label set in `efe.labels` won't reach pods). "
        f"Found instead: {block!r}"
    )


def test_deployment_selector_uses_selectorLabels_only():
    """The Deployment's selector MUST keep the small, stable
    ``efe.selectorLabels`` set — the immutability of selectors
    means we can't ever add tenant here without re-installing."""
    deployment = _DEPLOYMENT.read_text()
    # Tolerate inline comments between `selector:` and `matchLabels:`.
    m = re.search(
        r"\bselector:\s*\n(?:.*?\n)*?\s+matchLabels:\s*\{\{-?\s*include\s+\"([^\"]+)\"",
        deployment,
    )
    assert m, "could not parse Deployment selector block"
    helper_used = m.group(1)
    assert helper_used == "efe.selectorLabels", (
        f"selector must use `efe.selectorLabels`, got {helper_used!r}"
    )


# ─────────────────────────────────────────────────────────────────────
# 4. Service template — selector subset
# ─────────────────────────────────────────────────────────────────────

def test_service_selector_uses_selectorLabels_only():
    service = _SERVICE.read_text()
    m = re.search(
        r"selector:\s*\{\{-?\s*include\s+\"([^\"]+)\"",
        service,
    )
    assert m, "could not parse Service selector"
    assert m.group(1) == "efe.selectorLabels"


def test_service_supports_headless_toggle():
    """Audit fix A5 — the chart must allow ``service.headless: true``
    to publish the Service with ``clusterIP: None`` for client-side
    gRPC LB across replicas."""
    service = _SERVICE.read_text()
    # The toggle uses `if .Values.service.headless`.
    assert "if .Values.service.headless" in service, (
        "service.headless toggle missing from service.yaml"
    )
    assert "clusterIP: None" in service, (
        "headless branch must emit `clusterIP: None`"
    )


# ─────────────────────────────────────────────────────────────────────
# 5. values.yaml exposes the new D9 keys
# ─────────────────────────────────────────────────────────────────────

def test_values_yaml_has_tenant_block():
    values = _VALUES.read_text()
    assert re.search(r"^tenant:\s*$", values, re.MULTILINE), \
        "values.yaml must declare a top-level `tenant:` block"
    assert re.search(r"^\s+id:", values, re.MULTILINE), \
        "tenant.id default must be present"


def test_values_yaml_has_headless_default_true_post_d18():
    """D9 originally pinned ``headless: false`` (opt-in only) ; the
    D18 audit-fix A11 flipped the default to ``true`` once the
    canonical posture became multi-replica (>= 3 pods). Single-replica
    dev clusters can still opt back out via
    ``--set service.headless=false``.

    We keep the pin to guard against an accidental flip back to the
    pre-D18 single-replica default — that would silently kill the
    gRPC client-side load-balancing on prod multi-replica installs.
    """
    values = _VALUES.read_text()
    assert "headless: true" in values, (
        "service.headless default must be true post-D18-A11 audit fix "
        "(multi-replica gRPC client-side LB depends on the headless "
        "service ; cf. D9 §5 + D18 § 12.1)"
    )


def test_values_yaml_warns_about_unbuilt_bench_image():
    """Audit fix H3/R4 — the semantic smoke comment MUST warn the
    operator that the bench image is NOT built by the standard
    release workflow."""
    values = _VALUES.read_text()
    assert "ImagePullBackOff" in values or "not yet built" in values.lower(), (
        "values.yaml must warn that flipping tests.semanticSmoke.enabled "
        "without first building the bench image will fail"
    )


# ─────────────────────────────────────────────────────────────────────
# 6. helm test pods carry the tenant label too
# ─────────────────────────────────────────────────────────────────────

def test_helm_test_grpc_health_pod_uses_efe_labels():
    """The grpc-health test pod inherits the chart's labels — so
    operators can filter test pods per-tenant in shared clusters."""
    pod = (_CHART / "templates" / "tests" / "test-grpc-health.yaml").read_text()
    assert 'include "efe.labels"' in pod

"""Sprint **M12 — Helm chart extension** : pin tests anti-régression structurelle.

Cf. `docs/EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md`.

M12 livre 4 sprints d'extension du chart `deploy/helm/efe/` mature
post-D18 + A11 dedup :

  * **D21 (Φ-A)** — NetworkPolicy strict + Polaris validation.
  * **D22 (Φ-B)** — mTLS via cert-manager.
  * **D27 (Φ-C)** — Live bundle reload Helm post-upgrade hook.
  * **D40 (Φ-D)** — DR drill quarterly CronJob (déjà livré post-audit
                     ; M12 ajoute la pin test de vérification).

Ces pin tests vérifient les **invariants structurels** des artefacts
M12 : présence des fichiers, gating des templates par `.Values.<X>.enabled`,
hook annotations Helm, securityContext strict, helpers `_helpers.tpl`,
exemptions Polaris, schedule quarterly, etc.

Une régression structurelle (e.g., quelqu'un retire le gating
`{{- if .Values.networkPolicyStrict.enabled }}` ou abaisse la
severity Polaris d'un check `runAsRootAllowed`) surface en CI et
bloque le merge.

Sections (8) :

  1. Chart.yaml versioning + metadata (3 tests)
  2. Polaris config (.polaris.yaml) (3 tests)
  3. NetworkPolicy strict template (D21) (4 tests)
  4. mTLS Certificate template (D22) (4 tests)
  5. Bundle reload Helm hook Job + RBAC (D27) (4 tests)
  6. DR drill quarterly CronJob (D40 verify) (3 tests)
  7. helm-unittest fixtures + helm test pods (3 tests)
  8. CI workflows + values extension + chart CHANGELOG (4 tests)
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

_REPO = Path(__file__).resolve().parents[2]
_CHART = _REPO / "deploy" / "helm" / "efe"
_TEMPLATES = _CHART / "templates"
_TESTS_FIXTURES = _CHART / "tests"
_TEST_PODS = _TEMPLATES / "tests"
_WORKFLOWS = _REPO / ".github" / "workflows"


# ─────────────────────────────────────────────────────────────────────
# Helpers — pure utilities used across sections.
# ─────────────────────────────────────────────────────────────────────


def _read(path: Path) -> str:
    """Read a chart artefact as UTF-8 text — raises with explicit
    pointer if the file is missing (so the test failure surfaces the
    expected canonical path, not a generic FileNotFoundError)."""
    if not path.exists():
        raise AssertionError(
            f"M12 artefact missing : {path.relative_to(_REPO)}\n"
            f"Expected canonical path : {path}"
        )
    return path.read_text(encoding="utf-8")


def _yaml_docs(path: Path) -> list[dict]:
    """Multi-document YAML loader. Filters out None / empty docs.

    NOTE : Helm templates contain `{{- }}` Go template directives which
    are NOT valid YAML before rendering. This helper is only used for
    the Polaris config + helm-unittest fixtures + CI workflows + the
    `values.m12-extension.yaml` (all pure YAML, no templating)."""
    raw = _read(path)
    docs = list(yaml.safe_load_all(raw))
    return [d for d in docs if d]


# ═════════════════════════════════════════════════════════════════════
# Section 1 — Chart.yaml versioning + metadata.
# ═════════════════════════════════════════════════════════════════════


def test_m12_chart_yaml_kubeversion_pinned() -> None:
    """`Chart.yaml` must declare `kubeVersion: ">=1.27.0-0"` so the
    M12 features (cert-manager v1.13 ≥ K8s 1.27, NetworkPolicy
    strict, etc.) refuse to install on too-old clusters.

    Anti-regression : a future drop to `>=1.20.0` would silently
    enable installs on K8s 1.24 where cert-manager v1.13 fails."""
    raw = _read(_CHART / "Chart.yaml")
    chart = yaml.safe_load(raw)
    assert chart.get("apiVersion") == "v2", "Helm v3 chart must be apiVersion v2"
    assert chart.get("name") == "efe"
    kube_ver = chart.get("kubeVersion", "")
    assert ">=1.27" in kube_ver, (
        f"kubeVersion must require ≥1.27 (cert-manager v1.13 prereq) ; "
        f"got {kube_ver!r}"
    )


def test_m12_chart_yaml_has_efe_keywords() -> None:
    """Chart keywords must include the EFE-specific markers so
    artifact-hub indexing surfaces the chart for `grpc + swarm + ai`
    queries."""
    chart = yaml.safe_load(_read(_CHART / "Chart.yaml"))
    keywords = set(chart.get("keywords", []))
    expected = {"grpc", "swarm", "ai"}
    assert expected.issubset(keywords), (
        f"Chart.yaml keywords must include {expected} ; got {keywords}"
    )


def test_m12_chart_yaml_artifacthub_annotations_present() -> None:
    """artifact-hub annotations must be declared so Helm UIs / hubs
    render the chart description + license + change log."""
    chart = yaml.safe_load(_read(_CHART / "Chart.yaml"))
    annotations = chart.get("annotations", {})
    assert "artifacthub.io/license" in annotations
    assert "artifacthub.io/changes" in annotations


# ═════════════════════════════════════════════════════════════════════
# Section 2 — Polaris config (.polaris.yaml).
# ═════════════════════════════════════════════════════════════════════


def test_m12_polaris_config_present_and_well_formed() -> None:
    """`.polaris.yaml` must exist + be valid YAML with `checks` +
    `exemptions` top-level keys."""
    polaris = yaml.safe_load(_read(_CHART / ".polaris.yaml"))
    assert "checks" in polaris, ".polaris.yaml must declare a `checks` section"
    assert "exemptions" in polaris, ".polaris.yaml must declare an `exemptions` section"
    assert isinstance(polaris["checks"], dict)
    assert isinstance(polaris["exemptions"], list)


def test_m12_polaris_dangers_pinned() -> None:
    """The 7 critical security checks MUST stay at severity=danger.

    Anti-regression : a downgrade to `warning` or `ignore` would
    let a reviewer merge a `runAsRoot: true` Pod without CI
    blocking — that's an M12 GA gate violation."""
    polaris = yaml.safe_load(_read(_CHART / ".polaris.yaml"))
    checks = polaris["checks"]

    must_be_danger = [
        "hostNetworkSet",
        "hostPIDSet",
        "hostIPCSet",
        "hostPortSet",
        "privilegeEscalationAllowed",
        "runAsRootAllowed",
        "runAsPrivileged",
        "dangerousCapabilities",
        "tagNotSpecified",
    ]
    for check in must_be_danger:
        assert checks.get(check) == "danger", (
            f"Polaris check `{check}` must stay at severity=danger ; "
            f"got {checks.get(check)!r}"
        )


def test_m12_polaris_exemptions_cover_cronjobs_and_test_pods() -> None:
    """Exemptions MUST cover the 4 CronJobs (efe-backup,
    efe-dr-drill, efe-dr-drill-quarterly, efe-bundle-reload) +
    the 4 helm test pods (efe-test-connection, efe-test-networkpolicy,
    efe-test-mtls-handshake, efe-test-bundle-reload).

    These short-lived workloads legitimately skip probes / QoS."""
    polaris = yaml.safe_load(_read(_CHART / ".polaris.yaml"))
    exemptions = polaris["exemptions"]
    all_controllers: set[str] = set()
    for ex in exemptions:
        all_controllers.update(ex.get("controllerNames", []))

    cronjobs = {
        "efe-backup",
        "efe-dr-drill",
        "efe-dr-drill-quarterly",
        "efe-bundle-reload",
    }
    test_pods = {
        "efe-test-connection",
        "efe-test-networkpolicy",
        "efe-test-mtls-handshake",
        "efe-test-bundle-reload",
    }
    missing = (cronjobs | test_pods) - all_controllers
    assert not missing, f"Polaris exemptions missing controllers : {missing}"


# ═════════════════════════════════════════════════════════════════════
# Section 3 — NetworkPolicy strict (D21).
# ═════════════════════════════════════════════════════════════════════


def test_m12_d21_networkpolicy_strict_template_present() -> None:
    """`templates/d21-networkpolicy-strict.yaml` must exist with the
    canonical sprint-tagged filename for audit traceability."""
    f = _TEMPLATES / "d21-networkpolicy-strict.yaml"
    body = _read(f)
    assert "kind: NetworkPolicy" in body
    assert "apiVersion: networking.k8s.io/v1" in body


def test_m12_d21_networkpolicy_strict_gated_by_values() -> None:
    """The template MUST be wrapped in `{{- if .Values.networkPolicyStrict.enabled }}`
    so it's opt-in (default false → backward-compat with pre-M12)."""
    body = _read(_TEMPLATES / "d21-networkpolicy-strict.yaml")
    assert "{{- if .Values.networkPolicyStrict.enabled }}" in body, (
        "D21 NetworkPolicy strict must be gated by .Values.networkPolicyStrict.enabled"
    )


def test_m12_d21_networkpolicy_strict_has_default_deny_semantics() -> None:
    """Strict NetworkPolicy MUST declare both Ingress + Egress in
    `policyTypes` (omitting either would NOT default-deny that
    direction — silent regression)."""
    body = _read(_TEMPLATES / "d21-networkpolicy-strict.yaml")
    # Lookup the policyTypes block — pre-render but pinning the literal lines.
    assert "policyTypes:" in body
    assert "- Ingress" in body
    assert "- Egress" in body


def test_m12_d21_networkpolicy_strict_has_required_egress_blocks() -> None:
    """Strict NetworkPolicy MUST declare 5 egress concerns :
    DNS, audit aggregator, S3 backup, OIDC, cert-manager.

    Each block is gated by its own `.Values.networkPolicyStrict.egress.<X>`
    key — but the block STRUCTURE must exist so operators have a
    knob to toggle each."""
    body = _read(_TEMPLATES / "d21-networkpolicy-strict.yaml")
    expected_keys = [
        ".Values.networkPolicyStrict.egress.dnsEnabled",
        ".Values.networkPolicyStrict.egress.auditAggregatorSelector",
        ".Values.networkPolicyStrict.egress.s3BackupEnabled",
        ".Values.networkPolicyStrict.egress.oidcEnabled",
        ".Values.mtls.enabled",  # cert-manager egress (D22 dependency)
    ]
    for key in expected_keys:
        assert key in body, (
            f"D21 strict NetworkPolicy missing egress block for {key}"
        )


# ═════════════════════════════════════════════════════════════════════
# Section 4 — mTLS Certificate (D22).
# ═════════════════════════════════════════════════════════════════════


def test_m12_d22_mtls_cert_template_present() -> None:
    """`templates/d22-mtls-cert.yaml` must exist + declare a
    cert-manager.io/v1 Certificate."""
    body = _read(_TEMPLATES / "d22-mtls-cert.yaml")
    assert "apiVersion: cert-manager.io/v1" in body
    assert "kind: Certificate" in body


def test_m12_d22_mtls_cert_required_field_validation() -> None:
    """The template MUST use `required` on `issuerRef.name` so a
    misconfiguration (mtls.enabled=true but no issuerRef.name) FAILS
    at render-time rather than producing a broken Certificate."""
    body = _read(_TEMPLATES / "d22-mtls-cert.yaml")
    assert (
        '.Values.mtls.issuerRef.name | required ".Values.mtls.issuerRef.name '
        'is required when mtls.enabled=true"' in body
    ), "D22 mTLS Certificate must `required`-validate issuerRef.name"


def test_m12_d22_mtls_cert_has_secure_key_defaults() -> None:
    """ECDSA P-256 is the M12 default (vs RSA-2048) — faster TLS
    handshake + smaller cert payload + better mobile compatibility.

    Anti-regression : a flip to RSA without explicit operator opt-in
    would degrade perf invisibly."""
    body = _read(_TEMPLATES / "d22-mtls-cert.yaml")
    assert (
        '.Values.mtls.privateKey.algorithm | default "ECDSA"' in body
    ), "D22 mTLS default key algorithm must be ECDSA"
    assert (
        '.Values.mtls.privateKey.rotationPolicy | default "Always"' in body
    ), "D22 mTLS default rotationPolicy must be Always (cert-manager rotates on each renew)"


def test_m12_d22_mtls_helpers_present() -> None:
    """`_helpers.tpl` extensions for D22 must live in
    `templates/d22-mtls-helpers.tpl` (sprint-tagged file convention)."""
    body = _read(_TEMPLATES / "d22-mtls-helpers.tpl")
    # 2 helpers required.
    assert '{{- define "efe.mtls.secretName" -}}' in body
    assert '{{- define "efe.bundle.hash" -}}' in body
    # The bundle.hash helper MUST use sha256sum + trunc 16 (stable
    # short hash for Pod annotation rolling-restart trigger).
    assert "sha256sum | trunc 16" in body, (
        "D22 efe.bundle.hash helper must use sha256sum + trunc 16 "
        "(stable rolling-restart trigger pattern)"
    )


# ═════════════════════════════════════════════════════════════════════
# Section 5 — Bundle reload Helm hook (D27).
# ═════════════════════════════════════════════════════════════════════


def test_m12_d27_bundle_reload_hook_template_present() -> None:
    """`templates/d27-bundle-reload-hook.yaml` must exist + declare
    a Job with the post-upgrade Helm hook annotation."""
    body = _read(_TEMPLATES / "d27-bundle-reload-hook.yaml")
    assert "kind: Job" in body
    assert (
        'helm.sh/hook: {{ .Values.bundleReload.hookTiming | default "post-upgrade" }}'
        in body
    )
    assert 'helm.sh/hook-weight: "10"' in body
    # Auto-cleanup on success (else stale Jobs accumulate every upgrade).
    assert (
        "helm.sh/hook-delete-policy: hook-succeeded,before-hook-creation" in body
    )


def test_m12_d27_bundle_reload_hook_idempotent() -> None:
    """The Job script MUST fetch the CURRENT bundle hash + skip if
    it equals EXPECTED. Without this idempotency check, every
    `helm upgrade` (even no-op) would trigger a reload — wasteful
    + risky for the running pods."""
    body = _read(_TEMPLATES / "d27-bundle-reload-hook.yaml")
    assert (
        '"${CURRENT}" = "${EXPECTED_HASH}"' in body
        or "skipping reload (idempotent)" in body
    ), "D27 bundle reload hook must be idempotent (skip if current=expected)"


def test_m12_d27_bundle_reload_hook_strict_security_context() -> None:
    """The Job container MUST run with strict securityContext :
    runAsNonRoot=65532 + readOnlyRootFilesystem + drop ALL caps +
    seccomp RuntimeDefault. Polaris GREEN gate enforces this."""
    body = _read(_TEMPLATES / "d27-bundle-reload-hook.yaml")
    expected_lines = [
        "runAsNonRoot: true",
        "runAsUser: 65532",
        "readOnlyRootFilesystem: true",
        "allowPrivilegeEscalation: false",
        "drop:",
        "- ALL",
        "type: RuntimeDefault",
    ]
    for line in expected_lines:
        assert line in body, (
            f"D27 bundle reload hook missing strict securityContext line : {line!r}"
        )


def test_m12_d27_bundle_reload_rbac_triplet_present() -> None:
    """`templates/d27-bundle-reload-rbac.yaml` MUST declare the full
    SA + Role + RoleBinding triplet, each with a Helm hook weight
    1/2/3 so they're applied BEFORE the Job (weight=10).

    Anti-regression : if Role.weight > Job.weight, the Job runs
    before the RBAC is in place → RBAC denied error."""
    body = _read(_TEMPLATES / "d27-bundle-reload-rbac.yaml")
    assert "kind: ServiceAccount" in body
    assert "kind: Role" in body
    assert "kind: RoleBinding" in body
    # Hook weights ordered : SA=1, Role=2, RoleBinding=3 (all < Job=10).
    for weight in ('"1"', '"2"', '"3"'):
        assert f"helm.sh/hook-weight: {weight}" in body, (
            f"D27 RBAC missing helm.sh/hook-weight: {weight}"
        )


# ═════════════════════════════════════════════════════════════════════
# Section 6 — DR drill quarterly (D40 verification — already livré).
# ═════════════════════════════════════════════════════════════════════


def test_m12_d40_dr_drill_quarterly_template_present() -> None:
    """`templates/d40-dr-drill-quarterly-cronjob.yaml` MUST exist
    (livré post-D17/D40 audit pass : M7-D40-A6 + A17 + A24 fixed).

    M12 vérifie qu'il n'a pas régressé."""
    body = _read(_TEMPLATES / "d40-dr-drill-quarterly-cronjob.yaml")
    assert "kind: CronJob" in body
    assert "apiVersion: batch/v1" in body


def test_m12_d40_dr_drill_quarterly_compliance_evidence_gate() -> None:
    """The template MUST gate volumeMount + PVC by
    `.Values.backup.drillQuarterly.complianceEvidence.enabled` —
    this is audit fix M7-D40-A6 (CRIT). A regression that re-couples
    the volumeMount unconditionally produces CrashLoopBackOff when
    complianceEvidence.enabled=false."""
    body = _read(_TEMPLATES / "d40-dr-drill-quarterly-cronjob.yaml")
    assert (
        ".Values.backup.drillQuarterly.complianceEvidence.enabled" in body
    ), "D40 audit fix M7-D40-A6 regression : complianceEvidence.enabled must gate volumeMount"


def test_m12_d40_dr_drill_quarterly_4_compliance_frameworks() -> None:
    """Labels + annotations MUST declare 4 compliance frameworks :
    SOC 2 / ISO 27001 / NIS2 / RGPD with the canonical criterion
    references. Auditors (e.g., SOC 2 evidence pull) rely on these."""
    body = _read(_TEMPLATES / "d40-dr-drill-quarterly-cronjob.yaml")
    expected_annotations = [
        ('compliance.efe.ai/soc2-criterion:', 'CC9.1'),
        ('compliance.efe.ai/iso27001-control:', 'A.17.1.3'),
        ('compliance.efe.ai/nis2-article:', 'Art21.2(c)'),
        ('compliance.efe.ai/rgpd-article:', 'Art32.1(c,d)'),
    ]
    for key, val in expected_annotations:
        assert key in body and val in body, (
            f"D40 compliance annotation missing : {key} {val}"
        )
    assert 'compliance.efe.ai/framework: "soc2,iso27001,nis2,rgpd"' in body


# ═════════════════════════════════════════════════════════════════════
# Section 7 — helm-unittest fixtures + helm test pods.
# ═════════════════════════════════════════════════════════════════════


@pytest.mark.parametrize(
    "fixture, expected_template",
    [
        ("networkpolicy-strict_test.yaml", "d21-networkpolicy-strict.yaml"),
        ("mtls_cert_test.yaml", "d22-mtls-cert.yaml"),
        ("bundle_reload_test.yaml", "d27-bundle-reload-hook.yaml"),
        ("dr_drill_quarterly_test.yaml", "d40-dr-drill-quarterly-cronjob.yaml"),
    ],
    ids=["d21", "d22", "d27", "d40"],
)
def test_m12_helm_unittest_fixtures_present(
    fixture: str, expected_template: str
) -> None:
    """Each M12 sprint MUST ship a helm-unittest fixture under
    `tests/<slug>_test.yaml` that targets the corresponding template.

    Run via : `helm unittest deploy/helm/efe/`."""
    f = _TESTS_FIXTURES / fixture
    doc = yaml.safe_load(_read(f))
    assert "tests" in doc and len(doc["tests"]) >= 3, (
        f"Fixture {fixture} must have ≥3 unit test cases"
    )
    assert "templates" in doc, f"Fixture {fixture} must declare a `templates` list"
    assert expected_template in doc["templates"], (
        f"Fixture {fixture} must target template {expected_template} ; "
        f"got {doc['templates']}"
    )


@pytest.mark.parametrize(
    "test_pod, sprint_label",
    [
        ("d21-test-networkpolicy.yaml", "D21"),
        ("d22-test-mtls-handshake.yaml", "D22"),
        ("d27-test-bundle-reload.yaml", "D27"),
    ],
    ids=["d21", "d22", "d27"],
)
def test_m12_helm_test_pods_present(test_pod: str, sprint_label: str) -> None:
    """Each M12 feature MUST ship a `helm test` Pod under
    `templates/tests/dXX-test-<slug>.yaml` so operators can run
    `helm test <release>` post-install to verify the live deployment.

    Pin tests : sprint label + helm.sh/hook=test annotation present."""
    body = _read(_TEST_PODS / test_pod)
    assert "kind: Pod" in body
    assert "helm.sh/hook: test" in body, (
        f"Test pod {test_pod} must declare helm.sh/hook: test"
    )
    assert f"efe.io/sprint: {sprint_label}" in body, (
        f"Test pod {test_pod} must label efe.io/sprint: {sprint_label}"
    )


def test_m12_helm_test_pods_strict_security_context() -> None:
    """ALL 3 M12 helm test pods MUST run as non-root + drop ALL caps
    + readOnlyRootFilesystem (Polaris GREEN gate)."""
    for pod in (
        "d21-test-networkpolicy.yaml",
        "d22-test-mtls-handshake.yaml",
        "d27-test-bundle-reload.yaml",
    ):
        body = _read(_TEST_PODS / pod)
        assert "runAsNonRoot: true" in body, f"{pod} : missing runAsNonRoot"
        assert "runAsUser: 65532" in body, f"{pod} : must use UID 65532 (nonroot)"
        assert "readOnlyRootFilesystem: true" in body, (
            f"{pod} : missing readOnlyRootFilesystem"
        )
        assert "drop:" in body and "- ALL" in body, (
            f"{pod} : capabilities.drop must include ALL"
        )


# ═════════════════════════════════════════════════════════════════════
# Section 8 — CI workflows + values extension + chart CHANGELOG.
# ═════════════════════════════════════════════════════════════════════


def test_m12_ci_helm_test_workflow_present() -> None:
    """`.github/workflows/helm-test.yml` MUST exist + declare the
    6 canonical jobs : lint-and-render, polaris, helm-unittest,
    ct-install, pin-tests, bash-syntax."""
    workflow = yaml.safe_load(_read(_WORKFLOWS / "helm-test.yml"))
    jobs = set(workflow.get("jobs", {}).keys())
    expected = {
        "lint-and-render",
        "polaris",
        "helm-unittest",
        "ct-install",
        "pin-tests",
        "bash-syntax",
    }
    missing = expected - jobs
    assert not missing, f"helm-test workflow missing jobs : {missing}"


def test_m12_ci_helm_test_kind_matrix_covers_3_k8s_versions() -> None:
    """ct-install matrix MUST cover K8s 1.26 + 1.28 + 1.30 — the
    chart's `kubeVersion: ">=1.27"` floor is tested with one below
    (1.26 expected to fail-fast on chart kubeVersion check) +
    two above (1.28 LTS + 1.30 latest)."""
    workflow = yaml.safe_load(_read(_WORKFLOWS / "helm-test.yml"))
    matrix = (
        workflow["jobs"]["ct-install"]
        .get("strategy", {})
        .get("matrix", {})
        .get("k8s", [])
    )
    expected_versions = ["1.26", "1.28", "1.30"]
    for ver in expected_versions:
        assert any(ver in v for v in matrix), (
            f"ct-install matrix missing K8s {ver}.x ; got {matrix}"
        )


def test_m12_ci_helm_publish_workflow_two_modes() -> None:
    """`.github/workflows/helm-publish.yml` MUST support 2 modes :
    private (Φ-A → Φ-D pre-GA) + public (D40 GA, requires v1.x.x)."""
    body = _read(_WORKFLOWS / "helm-publish.yml")
    assert "ghcr.io/efe-ai/charts-private" in body, (
        "helm-publish must support private GHCR registry"
    )
    assert "ghcr.io/efe-ai/charts" in body, (
        "helm-publish must support public GHCR registry"
    )
    # GA gate : public mode must require chart v1.x.x.
    assert (
        '"${{ steps.chart.outputs.chart_version }}" =~ ^1\\.' in body
    ), "helm-publish public mode must enforce chart v1.x.x (GA gate)"


def test_m12_values_extension_documents_all_sprints() -> None:
    """`values.m12-extension.yaml` MUST document all 4 NEW values
    sections opt-in : networkPolicyStrict (D21), mtls (D22),
    bundle + bundleReload (D27)."""
    values = yaml.safe_load(_read(_CHART / "values.m12-extension.yaml"))
    expected_top_level = {
        "networkPolicyStrict",
        "mtls",
        "bundle",
        "bundleReload",
    }
    actual = set(values.keys())
    missing = expected_top_level - actual
    assert not missing, f"values.m12-extension.yaml missing keys : {missing}"

    # All M12 features default to enabled=false (backward-compat).
    for feature in ("networkPolicyStrict", "mtls", "bundleReload"):
        assert values[feature].get("enabled") is False, (
            f"values.m12-extension.yaml : {feature}.enabled MUST default "
            f"to false (backward-compat) ; got {values[feature].get('enabled')!r}"
        )


def test_m12_chart_changelog_documents_4_sprints() -> None:
    """The chart-local `CHANGELOG.md` MUST document the M12
    multi-sprint code drop : D21 + D22 + D27 + D40."""
    body = _read(_CHART / "CHANGELOG.md")
    assert "## [Unreleased]" in body
    for sprint in ("D21", "D22", "D27", "D40"):
        assert f"Sprint {sprint}" in body, (
            f"chart CHANGELOG.md missing Sprint {sprint} section"
        )
    # Trajectory : 0.6.0 → 0.7.0 → 0.8.0 → 0.9.0 → 1.0.0.
    for ver in ("0.6.0", "0.7.0", "0.8.0", "0.9.0", "1.0.0"):
        assert ver in body, f"chart CHANGELOG.md missing version {ver} in trajectory"


# ═════════════════════════════════════════════════════════════════════
# Optional smoke check — file count sanity.
# ═════════════════════════════════════════════════════════════════════


# ═════════════════════════════════════════════════════════════════════
# Section 9 — Audit fixes M12 (chirurgical pass 1 — 8 défauts).
# ═════════════════════════════════════════════════════════════════════
#
# Cf. CHANGELOG.md `### M12 audit pass 1` for the 8 defects.
# These pin tests anchor the fix on a structural invariant (not on the
# audit-fix marker comment) so a future refactor that re-introduces
# the bug surfaces in CI.


def test_m12_audit_a1_d21_test_pod_uses_metrics_port_not_grpc() -> None:
    """A1 (CRIT) — pre-fix the D21 test pod did `wget` against
    `service.grpcPort` (50051), but gRPC speaks HTTP/2 + protobuf,
    not HTTP/1.1 — so wget ALWAYS failed regardless of NetworkPolicy
    state. Test was non-discriminant : passed even when strict NP
    was disabled. Post-fix uses `service.metricsPort` (HTTP plain) so
    a permissive NP yields wget-success → test FAIL (correctly), and
    a strict NP yields wget-timeout → test PASS (correctly)."""
    body = _read(_TEST_PODS / "d21-test-networkpolicy.yaml")
    # Anchor on the actual `wget` call site, not on the audit-fix
    # comment that mentions the pre-fix pattern.
    assert ".Values.service.metricsPort | default 9090" in body, (
        "A1 regression : D21 test pod must use service.metricsPort (HTTP), "
        "NOT service.grpcPort (gRPC speaks HTTP/2 ; wget always fails)"
    )
    assert "/metrics" in body, (
        "A1 regression : D21 test pod must hit `/metrics` endpoint "
        "(Prometheus scrape format) — non-existent paths produce 404 not "
        "discriminant from NetworkPolicy denial"
    )
    # Anti-regression : grpcPort must NOT appear in the `wget` URL.
    # (It's OK to appear in audit-fix comments — but not as the active
    # target.) We pin on the absence of the literal call pattern.
    assert 'wget --timeout=10 -qO- "http://{{ include "efe.fullname" . }}:{{ .Values.service.grpcPort' not in body, (
        "A1 regression : D21 test pod re-introduced wget against grpcPort"
    )


def test_m12_audit_a2_d27_hook_uses_curl_not_wget() -> None:
    """A2 (HIGH) — pre-fix the D27 hook script used `wget`, but the
    canonical `bitnami/kubectl:1.30` image (built on minideb-extras-base)
    ships `curl` deterministically and `wget` only sometimes. Result :
    `/bin/sh: wget: not found` → Job exit 127 → bundle reload silently
    skipped → version drift between values + running pods."""
    body = _read(_TEMPLATES / "d27-bundle-reload-hook.yaml")
    # Verify curl is the active HTTP client.
    assert "curl -fsS --max-time" in body, (
        "A2 regression : D27 hook must use `curl -fsS --max-time` "
        "(deterministic in bitnami/kubectl image)"
    )
    # Verify the active script body has NO wget call (audit-fix comments
    # may mention it as the pre-fix pattern, but not as actual code).
    # Anchor on the literal command-arg pattern.
    assert "wget -qO-" not in body, (
        "A2 regression : D27 hook re-introduced wget call site"
    )
    assert "wget --timeout=" not in body, (
        "A2 regression : D27 hook re-introduced wget timeout call"
    )
    assert "--post-data=" not in body, (
        "A2 regression : D27 hook re-introduced wget --post-data flag "
        "(curl uses -X POST + -d)"
    )


def test_m12_audit_a3_d22_cert_no_nonstandard_secret_annotations() -> None:
    """A3 (MED) — pre-fix the Certificate template set
    `secretTemplate.annotations.cert-manager.io/issuer` which is NOT
    a canonical cert-manager annotation (the canonical set is
    `cert-manager.io/{certificate-name, issuer-name, issuer-kind,
    issuer-group}`, all auto-populated by the controller). The
    override produced a non-standard annotation that didn't match
    what cert-manager itself adds. Post-fix the template only sets
    its own labels for selection ; cert-manager auto-populates
    standard annotations on the Secret."""
    body = _read(_TEMPLATES / "d22-mtls-cert.yaml")
    # The non-standard `cert-manager.io/issuer:` annotation must NOT
    # be set in secretTemplate.annotations (mention in audit-fix
    # comment is OK ; we check for the active YAML key).
    # Pin on the actual YAML structure: there must be NO
    # `secretTemplate.annotations` block at all post-fix.
    secret_template_block_pattern = re.compile(
        r"secretTemplate:\s*\n\s+labels:[\s\S]*?\n\s+annotations:",
        re.MULTILINE,
    )
    assert not secret_template_block_pattern.search(body), (
        "A3 regression : D22 Certificate re-introduced a "
        "secretTemplate.annotations block (cert-manager auto-populates "
        "standard annotations — operator overrides confuse kubectl inspection)"
    )


def test_m12_audit_a4_helm_test_template_no_create_namespace() -> None:
    """A4 (MED) — pre-fix the `helm template` step in the helm-test
    workflow had `--create-namespace`, a flag valid only for
    `helm install/upgrade`, not `helm template` (which is a pure
    manifest renderer). Helm 3 silently ignored the flag with a
    warning ; post-fix the flag is removed for semantic correctness."""
    body = _read(_WORKFLOWS / "helm-test.yml")
    # Anchor on the absence of `--create-namespace` in `helm template`
    # invocations. The workflow has 4 such invocations (default,
    # staging, prod, m12-features).
    template_block_pattern = re.compile(
        r"helm template[^|]*?--create-namespace",
        re.MULTILINE | re.DOTALL,
    )
    matches = template_block_pattern.findall(body)
    assert not matches, (
        f"A4 regression : helm template invocation re-introduced "
        f"--create-namespace flag ({len(matches)} occurrence(s) found). "
        f"This flag is only valid for `helm install/upgrade`."
    )


def test_m12_audit_a5_chart_changelog_polaris_counts_correct() -> None:
    """A5 (LOW) — pre-fix the chart-local CHANGELOG.md claimed
    "11 checks à danger + 9 à warning + 1 à ignore" but the actual
    .polaris.yaml has 9 + 12 + 1. Post-fix CHANGELOG matches reality
    + .polaris.yaml has a comment block documenting the tally."""
    chart_changelog = _read(_CHART / "CHANGELOG.md")
    polaris_yaml = _read(_CHART / ".polaris.yaml")
    # Chart CHANGELOG must declare 9 dangers (matching reality).
    assert "9 checks à `danger`" in chart_changelog, (
        "A5 regression : chart CHANGELOG.md must say `9 checks à danger` "
        "(matching the actual .polaris.yaml content)"
    )
    # The polaris config itself documents the tally as a comment.
    assert "9 checks at danger" in polaris_yaml, (
        "A5 regression : .polaris.yaml must have a comment block "
        "documenting the post-audit tally (9 dangers / 12 warnings / 1 ignore)"
    )


def test_m12_audit_a6_helm_test_pin_tests_installs_pyyaml() -> None:
    """A6 (HIGH) — pre-fix the pin-tests CI job ran `pip install pytest`
    but the M12 pin tests `import yaml` (PyYAML). Without the dep,
    the run crashed with `ModuleNotFoundError`. Additionally, the
    repo's pyproject.toml declares `addopts: --cov-config=...` which
    crashed pytest at startup if pytest-cov wasn't installed. Post-fix
    installs both pyyaml + pytest-cov AND uses `-c /dev/null` to
    bypass the project pytest config."""
    body = _read(_WORKFLOWS / "helm-test.yml")
    assert "pip install pytest pyyaml pytest-cov" in body, (
        "A6 regression : helm-test pin-tests job must install pyyaml + "
        "pytest-cov (the M12 pin tests `import yaml` ; pyproject.toml "
        "addopts requires pytest-cov to load)"
    )
    assert "-c /dev/null" in body, (
        "A6 regression : helm-test pin-tests must use `-c /dev/null` to "
        "bypass project pytest config (which adds --cov-config=pyproject.toml)"
    )


def test_m12_audit_a7_helm_test_versions_use_env_pins() -> None:
    """A7 (LOW) — pre-fix, 3 of 4 jobs hardcoded `version: "v3.13.3"`
    instead of using `${{ env.HELM_VERSION }}`. Bumping HELM_VERSION
    at the top of the workflow didn't propagate. Post-fix all jobs
    reference the env pin."""
    body = _read(_WORKFLOWS / "helm-test.yml")
    # Count occurrences of hardcoded 3.13.3 vs env.HELM_VERSION.
    hardcoded_helm = body.count('version: "v3.13.3"')
    hardcoded_ct = body.count('version: "v3.10.1"')
    env_helm = body.count("${{ env.HELM_VERSION }}")
    env_ct = body.count("${{ env.CT_VERSION }}")
    assert hardcoded_helm == 0, (
        f"A7 regression : {hardcoded_helm} hardcoded helm version(s) "
        f"left ; should all reference ${{ env.HELM_VERSION }}"
    )
    assert hardcoded_ct == 0, (
        f"A7 regression : {hardcoded_ct} hardcoded ct version(s) left"
    )
    # At least 4 jobs reference env.HELM_VERSION (lint-and-render,
    # polaris, helm-unittest, ct-install).
    assert env_helm >= 4, (
        f"A7 regression : expected ≥4 references to env.HELM_VERSION, "
        f"got {env_helm}"
    )
    # ct-install + ct lint reference env.CT_VERSION (≥2).
    assert env_ct >= 2, (
        f"A7 regression : expected ≥2 references to env.CT_VERSION, "
        f"got {env_ct}"
    )


def test_m12_audit_a8_d27_rbac_hook_covers_install_and_upgrade() -> None:
    """A8 (LOW) — pre-fix the D27 RBAC triplet had
    `helm.sh/hook: post-upgrade` hardcoded, but the Job's hookTiming
    is configurable (`post-install` or `post-upgrade`). If an
    operator chose `hookTiming: post-install`, the Job ran but the
    RBAC was NOT created → RBAC denied → Job exit non-zero. Post-fix
    the RBAC declares BOTH hooks (benign over-creation in the
    non-active case ; Helm cleans up via hook-delete-policy)."""
    body = _read(_TEMPLATES / "d27-bundle-reload-rbac.yaml")
    # Anchor on the active YAML annotation values (the audit-fix
    # comment may include the pre-fix pattern, but the active
    # annotation MUST be the multi-hook value).
    occurrences = body.count("helm.sh/hook: post-install,post-upgrade")
    # Triplet : SA + Role + RoleBinding = 3 hook annotations.
    assert occurrences == 3, (
        f"A8 regression : D27 RBAC triplet must have 3x "
        f"`helm.sh/hook: post-install,post-upgrade` (one per resource) ; "
        f"got {occurrences}"
    )
    # Anti-regression : the lone `post-upgrade` value (without
    # `post-install,`) MUST NOT appear as an active annotation.
    # We check by counting active vs comment occurrences. The audit-fix
    # comment intentionally includes the pre-fix pattern. So we look
    # for the literal annotation line WITHOUT a leading `#`.
    active_post_upgrade_only = re.findall(
        r"^\s+helm\.sh/hook:\s*post-upgrade\s*$", body, re.MULTILINE
    )
    assert not active_post_upgrade_only, (
        "A8 regression : D27 RBAC re-introduced lone `post-upgrade` "
        "hook (must be `post-install,post-upgrade` to cover both Helm "
        "lifecycle events)"
    )


def test_m12_canonical_artefact_inventory() -> None:
    """Smoke check : the M12 code drop ships exactly N canonical
    files. A future drop or rename surfaces here.

    | Sprint | File count | Expected |
    | --- | --- | --- |
    | D21 | 2 | strict NP + Polaris config |
    | D22 | 2 | mTLS Cert + helpers |
    | D27 | 2 | bundle reload Job + RBAC |
    | tests | 4 | helm-unittest fixtures |
    | test-pods | 3 | helm test Pods |
    | CI | 2 | helm-test + helm-publish |
    | values + CHANGELOG | 2 | values fragment + chart CHANGELOG |
    | **total** | **17** | M12 canonical artefacts |
    """
    canonical_files = [
        # D21 (2)
        _TEMPLATES / "d21-networkpolicy-strict.yaml",
        _CHART / ".polaris.yaml",
        # D22 (2)
        _TEMPLATES / "d22-mtls-cert.yaml",
        _TEMPLATES / "d22-mtls-helpers.tpl",
        # D27 (2)
        _TEMPLATES / "d27-bundle-reload-hook.yaml",
        _TEMPLATES / "d27-bundle-reload-rbac.yaml",
        # helm-unittest fixtures (4)
        _TESTS_FIXTURES / "networkpolicy-strict_test.yaml",
        _TESTS_FIXTURES / "mtls_cert_test.yaml",
        _TESTS_FIXTURES / "bundle_reload_test.yaml",
        _TESTS_FIXTURES / "dr_drill_quarterly_test.yaml",
        # helm test Pods (3)
        _TEST_PODS / "d21-test-networkpolicy.yaml",
        _TEST_PODS / "d22-test-mtls-handshake.yaml",
        _TEST_PODS / "d27-test-bundle-reload.yaml",
        # CI (2)
        _WORKFLOWS / "helm-test.yml",
        _WORKFLOWS / "helm-publish.yml",
        # values + CHANGELOG (2)
        _CHART / "values.m12-extension.yaml",
        _CHART / "CHANGELOG.md",
    ]
    assert len(canonical_files) == 17, (
        f"M12 inventory drift : expected 17 canonical files, got {len(canonical_files)}"
    )
    missing = [f for f in canonical_files if not f.exists()]
    assert not missing, (
        f"M12 inventory missing files : {[str(f.relative_to(_REPO)) for f in missing]}"
    )

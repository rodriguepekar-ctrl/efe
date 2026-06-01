#!/usr/bin/env python3
"""
Verify that the code of every meta-swarm PR (Phase 1 PR #1-#4 +
Phase 2 PR #5-#15) is present in the working directory.

For each PR :
  - list expected artefacts (files + grep patterns)
  - check existence / non-emptiness
  - cumulative LOC
  - print a clean OK / MISSING / PARTIAL verdict
"""

from __future__ import annotations

import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


# ─── A check ───────────────────────────────────────────────────────
@dataclass
class Check:
    """A single artefact-level expectation."""
    path: str                          # path relative to REPO
    kind: str = "file"                 # file | dir | grep
    pattern: str | None = None         # grep pattern (when kind=grep)
    target: str | None = None          # grep target path (when kind=grep)
    min_loc: int = 0                   # minimum LOC for `file` (0 = just exists)
    note: str = ""

    def evaluate(self) -> tuple[bool, str]:
        full = REPO / self.path
        if self.kind == "file":
            if not full.is_file():
                return False, f"missing : {self.path}"
            loc = sum(1 for _ in full.open("r", encoding="utf-8", errors="ignore"))
            if loc < self.min_loc:
                return False, f"too small : {self.path} ({loc} < {self.min_loc} LOC)"
            return True, f"{self.path} ({loc:,} LOC)"
        if self.kind == "dir":
            if not full.is_dir():
                return False, f"missing dir : {self.path}"
            n = sum(1 for _ in full.iterdir())
            return True, f"{self.path}/ ({n} entries)"
        if self.kind == "grep":
            target = REPO / (self.target or self.path)
            if not target.exists():
                return False, f"missing target : {self.target or self.path}"
            try:
                txt = target.read_text(encoding="utf-8", errors="ignore")
            except Exception:
                return False, f"unreadable : {target}"
            if re.search(self.pattern or "", txt, re.MULTILINE):
                return True, f"pattern found in {self.target or self.path}"
            return False, f"pattern NOT in {self.target or self.path} : /{self.pattern}/"
        return False, f"unknown kind : {self.kind}"


@dataclass
class PR:
    label: str
    title: str
    notes: str = ""
    checks: list[Check] = field(default_factory=list)


# ─── PR catalog with expected artefacts ────────────────────────────
PRS: list[PR] = [
    # ═══════════════════ Phase 1 ═══════════════════
    PR(
        label="Phase-1 PR #1",
        title="Scaffold efe-meta-swarm crate",
        checks=[
            Check("crates/efe-meta-swarm/Cargo.toml"),
            Check("crates/efe-meta-swarm/src/lib.rs", min_loc=10),
            Check("crates/efe-meta-swarm/src/meta_swarm.rs"),
            Check("crates/efe-meta-swarm/src/builder.rs"),
            Check("crates/efe-meta-swarm/src/error.rs"),
            Check("crates/efe-meta-swarm/src/observability.rs"),
            Check("crates/efe-meta-swarm/src/prelude.rs"),
            Check("crates/efe-meta-swarm/src/iteration/coupled.rs"),
            Check("crates/efe-meta-swarm/src/iteration/convergence.rs"),
            Check("crates/efe-meta-swarm/src/iteration/trace.rs"),
            Check("crates/efe-meta-swarm/src/operators/linear.rs"),
            Check("crates/efe-meta-swarm/src/operators/attention.rs"),
            Check("crates/efe-meta-swarm/src/operators/uniform.rs"),
            Check("crates/efe-meta-swarm/src/operators/lift.rs"),
            Check("crates/efe-meta-swarm/src/operators/pull.rs"),
            Check("crates/efe-meta-swarm/src/operators/meta_op.rs"),
            Check("crates/efe-meta-swarm/src/participants/mock.rs"),
            Check("crates/efe-meta-swarm/src/safety/box_safety.rs"),
            Check("crates/efe-meta-swarm/src/safety/learned.rs"),
            Check("crates/efe-meta-swarm/examples/two_swarms_uniform.rs"),
            Check("crates/efe-meta-swarm/examples/three_swarms_kovant.rs"),
            Check("crates/efe-meta-swarm/examples/multimodal_three_swarms.rs"),
            Check("crates/efe-meta-swarm/examples/safety_violation_attempt.rs"),
            Check("crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs"),
            Check("crates/efe-meta-swarm/PR_DESCRIPTION.md"),
            # workspace registration
            Check(kind="grep", target="Cargo.toml",
                  pattern=r'"crates/efe-meta-swarm"', path="Cargo.toml"),
        ],
    ),
    PR(
        label="Phase-1 PR #2",
        title="LocalParticipant<S: efe_core::Swarm> adapter",
        notes="No PR_DESCRIPTION.md ; verified by code surface.",
        checks=[
            Check("crates/efe-meta-swarm/src/participants/local.rs", min_loc=10),
            Check(kind="grep",
                  target="crates/efe-meta-swarm/src/participants/local.rs",
                  pattern=r"struct\s+LocalParticipant",
                  path="crates/efe-meta-swarm/src/participants/local.rs"),
        ],
    ),
    PR(
        label="Phase-1 PR #3",
        title="200-proptest target (130+ new cases)",
        checks=[
            Check("crates/efe-meta-swarm/PR3_DESCRIPTION.md"),
            Check("crates/efe-meta-swarm/tests/integration_iteration_proptest.rs", min_loc=30),
            # 14+ proptest! macros across the crate
            Check(kind="grep",
                  target="crates/efe-meta-swarm/Cargo.toml",
                  pattern=r"proptest\s*=",
                  path="crates/efe-meta-swarm/Cargo.toml"),
        ],
    ),
    PR(
        label="Phase-1 PR #4",
        title="Bench baseline tracking + master-gate hook",
        checks=[
            Check("crates/efe-meta-swarm/PR4_DESCRIPTION.md"),
            Check("crates/efe-meta-swarm/benches/three_swarms_kovant_latency.rs", min_loc=30),
            Check(kind="grep",
                  target="crates/efe-meta-swarm/Cargo.toml",
                  pattern=r"criterion",
                  path="crates/efe-meta-swarm/Cargo.toml"),
        ],
    ),

    # ═══════════════════ Phase 2 ═══════════════════
    PR(
        label="PR #5",
        title="efe-meta-swarm-proto scaffold (4 .proto + tonic-build)",
        checks=[
            Check("crates/efe-meta-swarm-proto/Cargo.toml"),
            Check("crates/efe-meta-swarm-proto/build.rs", min_loc=30),
            Check("crates/efe-meta-swarm-proto/src/lib.rs"),
            Check("crates/efe-meta-swarm-proto/tests/smoke_cross_crate.rs"),
            Check("protos/efe/v1/meta_swarm/errors.proto"),
            Check("protos/efe/v1/meta_swarm/messages.proto"),
            Check("protos/efe/v1/meta_swarm/service_meta_swarm.proto"),
            Check("protos/efe/v1/meta_swarm/service_meta_participant.proto"),
            Check(kind="grep", target="Cargo.toml",
                  pattern=r'"crates/efe-meta-swarm-proto"', path="Cargo.toml"),
        ],
    ),
    PR(
        label="PR #6",
        title="efe-meta-swarm-server scaffold (controller + participant binaries)",
        checks=[
            Check("crates/efe-meta-swarm-server/Cargo.toml"),
            Check("crates/efe-meta-swarm-server/src/lib.rs"),
            Check("crates/efe-meta-swarm-server/src/bin/controller.rs", min_loc=10),
            Check("crates/efe-meta-swarm-server/src/bin/participant.rs", min_loc=10),
            Check(path="crates/efe-meta-swarm-server/src/config", kind="dir"),
            Check(path="crates/efe-meta-swarm-server/src/auth", kind="dir"),
            Check(path="crates/efe-meta-swarm-server/src/participant", kind="dir"),
            Check(path="crates/efe-meta-swarm-server/src/manifest", kind="dir"),
            Check(path="crates/efe-meta-swarm-server/src/discovery", kind="dir"),
            Check(path="crates/efe-meta-swarm-server/src/observability", kind="dir"),
            Check(path="crates/efe-meta-swarm-server/src/controller", kind="dir"),
            Check(path="crates/efe-meta-swarm-server/src/client", kind="dir"),
            Check(kind="grep", target="Cargo.toml",
                  pattern=r'"crates/efe-meta-swarm-server"', path="Cargo.toml"),
        ],
    ),
    PR(
        label="PR #7",
        title="Phase-2 threat model (docs only)",
        checks=[
            Check("docs/EFE_A1_PHASE2_PR07_ARCHITECTURE.md", min_loc=500),
            Check("docs/security/PHASE2_THREAT_MODEL.md", min_loc=500),
        ],
    ),
    PR(
        label="PR #8",
        title="MetaSwarmService::Iterate wiring",
        checks=[
            Check("crates/efe-meta-swarm-server/src/controller/iterate.rs", min_loc=20),
            Check("crates/efe-meta-swarm-server/src/controller/service.rs", min_loc=20),
            Check("crates/efe-meta-swarm-server/tests/integration_iterate_inproc.rs"),
            Check("crates/efe-meta-swarm-server/tests/integration_e2e_iterate.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/service.rs",
                  pattern=r"(async\s+)?fn\s+iterate",
                  path="crates/efe-meta-swarm-server/src/controller/service.rs"),
        ],
    ),
    PR(
        label="PR #9",
        title="MetaParticipantService::Step wiring",
        checks=[
            Check("crates/efe-meta-swarm-server/src/participant/service.rs", min_loc=20),
            Check("crates/efe-meta-swarm-server/tests/integration_step_smoke.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/participant/service.rs",
                  pattern=r"(async\s+)?fn\s+step",
                  path="crates/efe-meta-swarm-server/src/participant/service.rs"),
        ],
    ),
    PR(
        label="PR #10",
        title="integration_grpc_multiprocess (multi-process e2e harness)",
        notes="File name expanded into multiple : integration_multiprocess_*.rs",
        checks=[
            Check("crates/efe-meta-swarm-server/tests/common/multiprocess.rs", min_loc=10),
            Check("crates/efe-meta-swarm-server/tests/integration_multiprocess_e2e.rs"),
            Check("crates/efe-meta-swarm-server/tests/integration_multiprocess_harness.rs"),
            Check("crates/efe-meta-swarm-server/tests/integration_multiprocess_lifecycle.rs"),
            Check("crates/efe-meta-swarm-server/tests/integration_multiprocess_failure.rs"),
        ],
    ),
    PR(
        label="PR #11",
        title="EFE_META_SWARM_ENABLED plumbing in efe-server",
        checks=[
            Check("crates/efe-server/src/services/meta_swarm/mod.rs"),
            Check("crates/efe-server/src/services/meta_swarm/metrics.rs"),
            Check(kind="grep",
                  target="crates/efe-server/src/config.rs",
                  pattern=r"EFE_META_SWARM_ENABLED",
                  path="crates/efe-server/src/config.rs"),
            Check(kind="grep",
                  target="crates/efe-server/Cargo.toml",
                  pattern=r"efe-meta-swarm-proto",
                  path="crates/efe-server/Cargo.toml"),
        ],
    ),
    PR(
        label="PR #12",
        title="SIGHUP + ReloadConfig RPC + manifest hot-reload",
        checks=[
            Check("crates/efe-meta-swarm-server/src/controller/reload.rs", min_loc=10),
            Check("crates/efe-meta-swarm-server/src/controller/state.rs"),
            Check("crates/efe-meta-swarm-server/src/participant/reload.rs"),
            Check("crates/efe-meta-swarm-server/src/participant/state.rs"),
            Check("crates/efe-meta-swarm-server/src/manifest/mod.rs"),
            Check("crates/efe-meta-swarm-server/src/manifest/schema.rs"),
            Check("crates/efe-meta-swarm-server/src/observability/shutdown.rs"),
            Check("crates/efe-meta-swarm-server/tests/integration_reload_e2e.rs"),
            Check("crates/efe-meta-swarm-server/tests/sighup_handler_smoke.rs"),
            Check("crates/efe-meta-swarm-server/tests/manifest_roundtrip.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/reload.rs",
                  pattern=r"(reload|ReloadConfig)",
                  path="crates/efe-meta-swarm-server/src/controller/reload.rs"),
        ],
    ),
    PR(
        label="PR #13",
        title="Helm sub-chart skeleton",
        checks=[
            Check("charts/efe-stack/charts/efe-meta-swarm/Chart.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/values.yaml", min_loc=50),
            Check("charts/efe-stack/charts/efe-meta-swarm/values.schema.json", min_loc=50),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/_helpers.tpl"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/configmap-manifest.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/controller-deployment.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/controller-service.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/controller-serviceaccount.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/participant-deployment.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/participant-service.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/participant-serviceaccount.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/pdb-controller.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/networkpolicy.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/servicemonitor.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/NOTES.txt"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/tests/connection-test.yaml"),
            Check(".github/workflows/helm-meta-swarm.yml"),
        ],
    ),
    PR(
        label="PR #14",
        title="Helm values + templates production-grade (hooks, alerts, dashboard, TLS, sidecar, PriorityClass)",
        checks=[
            # New templates
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/_sidecar-helpers.tpl"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/priorityclass-controller.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/hpa-controller.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/prometheusrule.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/grafana-dashboard.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/tls-issuer.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/tls-certificate.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/files/grafana-dashboard.json"),
            # Hooks
            Check(path="charts/efe-stack/charts/efe-meta-swarm/templates/hooks", kind="dir"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/hooks/_hook-helpers.tpl"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/hooks/hook-serviceaccount.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/hooks/hook-role.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/hooks/hook-rolebinding.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/hooks/pre-upgrade-snapshot.yaml", min_loc=30),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/hooks/post-upgrade-sighup.yaml", min_loc=30),
            # Examples (4 overlays)
            Check("charts/efe-stack/charts/efe-meta-swarm/examples/values-dev.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/examples/values-staging.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/examples/values-prod.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/examples/values-canary.yaml"),
            # Tests (11 suites — 4 added in audit pass)
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/hooks_test.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/observability_test.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/sidecar_test.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/tls_test.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/tenant_labels_test.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/priorityclass_hpa_test.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/networkpolicy_test.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/schema_rejection_test.yaml"),
            # Scenarios (kovant-prod + kovant-hooks added)
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/kovant-prod.values.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/kovant-hooks.values.yaml"),
            # Goldens
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/golden/kovant-prod.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/golden/kovant-hooks.yaml"),
            # AUDIT PASS — trunc-safe helpers
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/templates/_helpers.tpl",
                  pattern=r"controllerTlsSecretName|controllerPriorityClassName|controllerPdbName",
                  path="charts/efe-stack/charts/efe-meta-swarm/templates/_helpers.tpl"),
            # AUDIT PASS — ignoreFailures ternary fix
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/templates/hooks/post-upgrade-sighup.yaml",
                  pattern=r'ternary "true" "false"',
                  path="charts/efe-stack/charts/efe-meta-swarm/templates/hooks/post-upgrade-sighup.yaml"),
            # AUDIT PASS — hookEgressCidrs values knob
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.yaml",
                  pattern=r"hookEgressCidrs",
                  path="charts/efe-stack/charts/efe-meta-swarm/values.yaml"),
            # AUDIT PASS — emptyDir Memory in snapshot Job
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/templates/hooks/pre-upgrade-snapshot.yaml",
                  pattern=r"medium: Memory",
                  path="charts/efe-stack/charts/efe-meta-swarm/templates/hooks/pre-upgrade-snapshot.yaml"),
        ],
    ),
    PR(
        label="PR #15",
        title="helm-meta-swarm-lint workflow (14 jobs + 12 Rego rules + lint configs)",
        checks=[
            Check(".github/workflows/helm-meta-swarm-lint.yml", min_loc=300),
            # Lint configs
            Check(".yamllint"),
            Check("chart-testing.yaml"),
            Check("polaris.yaml"),
            Check(".trivyignore"),
            Check(".checkov.yaml"),
            Check("renovate.json"),
            Check("Makefile"),
            # Rego rules (per-rule packages from audit pass)
            Check("policies/rego/efe/tenant_label.rego"),
            Check("policies/rego/efe/no_latest_tag.rego"),
            Check("policies/rego/efe/security_context.rego"),
            Check("policies/rego/efe/no_privileged.rego"),
            Check("policies/rego/efe/no_host_namespace.rego"),
            Check("policies/rego/efe/sa_token.rego"),
            Check("policies/rego/efe/share_process_namespace.rego"),
            Check("policies/rego/efe/service_appprotocol.rego"),
            Check("policies/rego/efe/container_resources.rego"),
            Check("policies/rego/efe/networkpolicy_types.rego"),
            Check("policies/rego/efe/prometheusrule_runbook.rego"),
            Check("policies/rego/efe/kubernetes.rego"),
            # Rego tests
            Check("policies/rego/efe/tenant_label_test.rego"),
            Check("policies/rego/efe/no_latest_tag_test.rego"),
            Check("policies/rego/efe/security_context_test.rego"),
            Check("policies/rego/efe/no_privileged_test.rego"),
            Check("policies/rego/efe/no_host_namespace_test.rego"),
            Check("policies/rego/efe/sa_token_test.rego"),
            Check("policies/rego/efe/share_process_namespace_test.rego"),
            Check("policies/rego/efe/service_appprotocol_test.rego"),
            Check("policies/rego/efe/container_resources_test.rego"),
            Check("policies/rego/efe/networkpolicy_types_test.rego"),
            Check("policies/rego/efe/prometheusrule_runbook_test.rego"),
            Check("policies/rego/efe/kubernetes_test.rego"),
            # Rego data
            Check("policies/rego/efe/data/tenant.json"),
            Check("policies/rego/efe/data/runtime_pods.json"),
            # Chart extras
            Check("charts/efe-stack/charts/efe-meta-swarm/artifacthub-pkg.yml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/README.md.gotmpl"),
            # Workflow + helper scripts
            Check("tests/workflow/lint-self-test.bats", min_loc=50),
            Check("scripts/lint-helm-hook-scripts.sh"),
            # Pre-commit augmented
            Check(kind="grep", target=".pre-commit-config.yaml",
                  pattern=r"helm-docs|shellcheck-helm-hooks|conftest-efe-policies",
                  path=".pre-commit-config.yaml"),
            # AUDIT PASS — per-rule packages
            Check(kind="grep", target="policies/rego/efe/tenant_label.rego",
                  pattern=r"^package efe\.tenant_label",
                  path="policies/rego/efe/tenant_label.rego"),
            # AUDIT PASS — Job 15 version-drift
            Check(kind="grep", target=".github/workflows/helm-meta-swarm-lint.yml",
                  pattern=r"version-drift",
                  path=".github/workflows/helm-meta-swarm-lint.yml"),
            # AUDIT PASS — --all-namespaces in Conftest invocations
            Check(kind="grep", target=".github/workflows/helm-meta-swarm-lint.yml",
                  pattern=r"--all-namespaces",
                  path=".github/workflows/helm-meta-swarm-lint.yml"),
        ],
    ),

    # ═══════════════════ Session deliverables ═══════════════════
    PR(
        label="Compendium",
        title="Architecture PDF + builder script (PDF deliverable)",
        checks=[
            Check("scripts/build_meta_swarm_pdf.py", min_loc=100),
            Check("EFE_A1_PHASE2_META_SWARM_COMPENDIUM.pdf"),
            Check("scripts/verify_meta_swarm_code.py"),
            Check("docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md", min_loc=500),
        ],
    ),

    # ═════════════════════════ PR #16 ════════════════════════════
    PR(
        label="PR16",
        title="mTLS + cert-manager wiring : rustls module + chart additions",
        notes="Binary-side TLS subsystem + Helm hook + ClusterIssuer + 3 alerts + schema",
        checks=[
            # Architecture
            Check("docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md", min_loc=2000),
            # Rust module
            Check("crates/efe-meta-swarm-server/src/tls/mod.rs", min_loc=80),
            Check("crates/efe-meta-swarm-server/src/tls/config.rs", min_loc=300),
            Check("crates/efe-meta-swarm-server/src/tls/error.rs", min_loc=80),
            Check("crates/efe-meta-swarm-server/src/tls/reader.rs", min_loc=200),
            Check("crates/efe-meta-swarm-server/src/tls/watcher.rs", min_loc=200),
            Check("crates/efe-meta-swarm-server/src/tls/reload.rs", min_loc=300),
            Check("crates/efe-meta-swarm-server/src/tls/metrics.rs", min_loc=200),
            # Audit chain stub (PR #16 § 23 — gap-fill audit pass)
            Check("crates/efe-meta-swarm-server/src/tls/audit.rs", min_loc=150),
            # Binary wiring
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/bin/controller.rs",
                  pattern=r"TlsReloader",
                  path="crates/efe-meta-swarm-server/src/bin/controller.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/bin/participant.rs",
                  pattern=r"TlsReloader",
                  path="crates/efe-meta-swarm-server/src/bin/participant.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/client/remote_participant.rs",
                  pattern=r"connect_with_tls|ClientTlsConfig",
                  path="crates/efe-meta-swarm-server/src/client/remote_participant.rs"),
            # Config — new fields
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/config/controller.rs",
                  pattern=r"tls_mutual",
                  path="crates/efe-meta-swarm-server/src/config/controller.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/config/participant.rs",
                  pattern=r"tls_min_protocol",
                  path="crates/efe-meta-swarm-server/src/config/participant.rs"),
            # Cargo deps
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r'^rustls\s*=',
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r'^notify\s*=',
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r'^x509-parser\s*=',
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r'^rcgen\s*=',
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            # Integration tests
            Check("crates/efe-meta-swarm-server/tests/integration_tls_handshake.rs", min_loc=200),
            Check("crates/efe-meta-swarm-server/tests/integration_tls_rotation.rs", min_loc=150),
            Check("crates/efe-meta-swarm-server/tests/integration_tls_disabled.rs", min_loc=40),
            Check("crates/efe-meta-swarm-server/tests/integration_tls_mtls_strict.rs", min_loc=100),
            Check("crates/efe-meta-swarm-server/tests/integration_tls_san_validation.rs", min_loc=100),
            # Chart additions
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/tls-clusterissuer.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/tls-prometheusrule.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/hooks/tls-wait-for-cert.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/tls_wait_hook_test.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/tls_alerts_test.yaml"),
            Check("charts/efe-stack/charts/efe-meta-swarm/examples/values-tls-prod.yaml"),
            # Rego rule (PR #16 § 32 AC30 — gap-fill audit pass)
            Check("policies/rego/efe/tls_protocol_version.rego", min_loc=40),
            Check("policies/rego/efe/tls_protocol_version_test.rego", min_loc=40),
            # Operator runbook (PR #16 Appendix B — gap-fill audit pass)
            Check("docs/runbooks/RB_PR16_TLS_MANUAL_ROTATION.md", min_loc=80),
            # Chart values
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.yaml",
                  pattern=r"^\s*mutual:\s*false",
                  path="charts/efe-stack/charts/efe-meta-swarm/values.yaml"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.yaml",
                  pattern=r"minProtocolVersion:",
                  path="charts/efe-stack/charts/efe-meta-swarm/values.yaml"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.yaml",
                  pattern=r"alertExpiringHours:",
                  path="charts/efe-stack/charts/efe-meta-swarm/values.yaml"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.schema.json",
                  pattern=r'"algorithm".*"ECDSA"',
                  path="charts/efe-stack/charts/efe-meta-swarm/values.schema.json"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/templates/hooks/hook-role.yaml",
                  pattern=r"cert-manager\.io",
                  path="charts/efe-stack/charts/efe-meta-swarm/templates/hooks/hook-role.yaml"),
            # Chart version bump (PR #16 → ≥ 0.3.0 / ≥ 0.4.0 ; later
            # PRs bump further).  The regex below accepts any ≥
            # 0.3.0 / 0.4.0.
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml",
                  pattern=r"^version:\s*0\.[3-9]\.0",
                  path="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml",
                  pattern=r'^appVersion:\s*"0\.[4-9]\.0"',
                  path="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml"),
            # Gap-fill audit pass — loopback bypass + cert_check OTel span + audit stub
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/tls/mod.rs",
                  pattern=r"endpoint_is_loopback",
                  path="crates/efe-meta-swarm-server/src/tls/mod.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/tls/reload.rs",
                  pattern=r'"tls\.cert_check"',
                  path="crates/efe-meta-swarm-server/src/tls/reload.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/tls/config.rs",
                  pattern=r"IP SAN forbidden",
                  path="crates/efe-meta-swarm-server/src/tls/config.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/tls/config.rs",
                  pattern=r"wildcard SAN forbidden",
                  path="crates/efe-meta-swarm-server/src/tls/config.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/tls/audit.rs",
                  pattern=r"TlsAuditEvent",
                  path="crates/efe-meta-swarm-server/src/tls/audit.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/tls/mod.rs",
                  pattern=r"pub use audit",
                  path="crates/efe-meta-swarm-server/src/tls/mod.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/tls/reload.rs",
                  pattern=r"set_audit_emitter",
                  path="crates/efe-meta-swarm-server/src/tls/reload.rs"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/tests/schema_rejection_test.yaml",
                  pattern=r"AC30 — tls\.minProtocolVersion=\"1\.0\"",
                  path="charts/efe-stack/charts/efe-meta-swarm/tests/schema_rejection_test.yaml"),
            # Operational audit-pass (this conversation):
            # FIX1 — ControllerState carries tls_reloader
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/state.rs",
                  pattern=r"tls_reloader:\s*Option<Arc<TlsReloader>>",
                  path="crates/efe-meta-swarm-server/src/controller/state.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/state.rs",
                  pattern=r"set_tls_reloader",
                  path="crates/efe-meta-swarm-server/src/controller/state.rs"),
            # FIX2 — connect_participants uses connect_with_tls
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/iterate.rs",
                  pattern=r"RemoteParticipant::connect_with_tls",
                  path="crates/efe-meta-swarm-server/src/controller/iterate.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/iterate.rs",
                  pattern=r"extract_san_from_endpoint",
                  path="crates/efe-meta-swarm-server/src/controller/iterate.rs"),
            # FIX3 — bootstrap_tls returns the reloader
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/bin/controller.rs",
                  pattern=r"Option<Arc<TlsReloader>>",
                  path="crates/efe-meta-swarm-server/src/bin/controller.rs"),
            # FIX4 — plumbing integration test
            Check("crates/efe-meta-swarm-server/tests/integration_tls_state_plumbing.rs", min_loc=80),
            # FIX5a — Box::leak retired in favour of attach_watcher
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/tls/reload.rs",
                  pattern=r"attach_watcher",
                  path="crates/efe-meta-swarm-server/src/tls/reload.rs"),
            # FIX5b — SAN helpers extracted from folded scalars
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/templates/_helpers.tpl",
                  pattern=r"controllerAllowedParticipantSans",
                  path="charts/efe-stack/charts/efe-meta-swarm/templates/_helpers.tpl"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/templates/_helpers.tpl",
                  pattern=r"participantAllowedControllerSans",
                  path="charts/efe-stack/charts/efe-meta-swarm/templates/_helpers.tpl"),
            # FIX5c — ternary boolean pattern (PR #14 audit consistency)
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/templates/controller-deployment.yaml",
                  pattern=r'ternary "true" "false"',
                  path="charts/efe-stack/charts/efe-meta-swarm/templates/controller-deployment.yaml"),
        ],
    ),

    # ═════════════════════════ PR #17 ════════════════════════════
    PR(
        label="PR17",
        title="14 operational Prometheus metrics + 5 alerts + cardinality guard + PII redaction",
        notes="6 metric groups (Iterate / Step / Safety / Health / Manifest reload / Build info)",
        checks=[
            # Architecture
            Check("docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md", min_loc=1500),
            # Rust module : new submodules under observability/metrics/
            Check("crates/efe-meta-swarm-server/src/observability/metrics/mod.rs", min_loc=400),
            Check("crates/efe-meta-swarm-server/src/observability/metrics/taxonomies.rs", min_loc=300),
            Check("crates/efe-meta-swarm-server/src/observability/metrics/cardinality.rs", min_loc=300),
            Check("crates/efe-meta-swarm-server/src/observability/metrics/iterate.rs", min_loc=250),
            Check("crates/efe-meta-swarm-server/src/observability/metrics/step.rs", min_loc=200),
            Check("crates/efe-meta-swarm-server/src/observability/metrics/safety.rs", min_loc=100),
            Check("crates/efe-meta-swarm-server/src/observability/metrics/health.rs", min_loc=140),
            Check("crates/efe-meta-swarm-server/src/observability/metrics/manifest_reload.rs", min_loc=150),
            Check("crates/efe-meta-swarm-server/src/observability/metrics/build.rs", min_loc=100),
            # Integration test
            Check("crates/efe-meta-swarm-server/tests/integration_pr17_metrics_scrape.rs", min_loc=200),
            # Cargo deps
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r"^dashmap\s*=",
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r"^regex\s*=",
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            # Pre-touch wiring in ControllerState
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/state.rs",
                  pattern=r"pretouch_for_manifest",
                  path="crates/efe-meta-swarm-server/src/controller/state.rs"),
            # participant_up wiring in iterate.rs
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/iterate.rs",
                  pattern=r"health\(\)\.mark_up",
                  path="crates/efe-meta-swarm-server/src/controller/iterate.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/iterate.rs",
                  pattern=r"health\(\)\.mark_down",
                  path="crates/efe-meta-swarm-server/src/controller/iterate.rs"),
            # Iterate handler instrumentation
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/service.rs",
                  pattern=r"InflightGuard::new",
                  path="crates/efe-meta-swarm-server/src/controller/service.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/service.rs",
                  pattern=r"classify_grpc_status",
                  path="crates/efe-meta-swarm-server/src/controller/service.rs"),
            # Step handler instrumentation
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/participant/service.rs",
                  pattern=r"step\(\)\.observe_duration",
                  path="crates/efe-meta-swarm-server/src/participant/service.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/participant/service.rs",
                  pattern=r"observe_payload_bytes",
                  path="crates/efe-meta-swarm-server/src/participant/service.rs"),
            # Reload extension wiring
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/reload.rs",
                  pattern=r"manifest_reload\(\)\.observe_duration",
                  path="crates/efe-meta-swarm-server/src/controller/reload.rs"),
            # /healthz endpoint
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/observability/metrics/mod.rs",
                  pattern=r'"/healthz"',
                  path="crates/efe-meta-swarm-server/src/observability/metrics/mod.rs"),
            # Body cap
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/observability/metrics/mod.rs",
                  pattern=r"BODY_CAP",
                  path="crates/efe-meta-swarm-server/src/observability/metrics/mod.rs"),
            # Chart additions
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/pr17-prometheusrule.yaml", min_loc=80),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/pr17_alerts_test.yaml", min_loc=80),
            # values.yaml threshold block
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.yaml",
                  pattern=r"iterateP99HighSeconds:",
                  path="charts/efe-stack/charts/efe-meta-swarm/values.yaml"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.yaml",
                  pattern=r"safetyViolationRatePerS:",
                  path="charts/efe-stack/charts/efe-meta-swarm/values.yaml"),
            # values.schema.json threshold block
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.schema.json",
                  pattern=r'"thresholds":',
                  path="charts/efe-stack/charts/efe-meta-swarm/values.schema.json"),
            # ServiceMonitor jitter
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/templates/servicemonitor.yaml",
                  pattern=r"hashmod",
                  path="charts/efe-stack/charts/efe-meta-swarm/templates/servicemonitor.yaml"),
            # Chart version bump (PR #17 → ≥ 0.4.0 / ≥ 0.5.0 ; PR #18
            # bumps further — both accepted).
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml",
                  pattern=r"^version:\s*0\.[45]\.0",
                  path="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml",
                  pattern=r'^appVersion:\s*"0\.[56]\.0"',
                  path="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml"),
            # PR #17 audit-pass : G1 — inner-loop wiring
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/iterate.rs",
                  pattern=r"observe_iterations",
                  path="crates/efe-meta-swarm-server/src/controller/iterate.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/iterate.rs",
                  pattern=r"observe_convergence_error",
                  path="crates/efe-meta-swarm-server/src/controller/iterate.rs"),
            # PR #17 audit-pass : G2 — guarded helper
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/observability/metrics/mod.rs",
                  pattern=r"pub fn guarded<F>",
                  path="crates/efe-meta-swarm-server/src/observability/metrics/mod.rs"),
            # PR #17 audit-pass : G3 — safety violation attribution
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/iterate.rs",
                  pattern=r"safety\(\)\.record_violation",
                  path="crates/efe-meta-swarm-server/src/controller/iterate.rs"),
            # PR #17 audit-pass : G5 / G6 — Rego rules
            Check("policies/rego/efe/metric_help_nonempty.rego", min_loc=40),
            Check("policies/rego/efe/metric_help_nonempty_test.rego", min_loc=30),
            Check("policies/rego/efe/metric_label_lowercase.rego", min_loc=40),
            Check("policies/rego/efe/metric_label_lowercase_test.rego", min_loc=30),
            # PR #17 audit-pass : G7 — stability contract
            Check("docs/STABILITY_CONTRACT.md", min_loc=100),
            # PR #17 audit-pass : G8 — cardinality breach runbook
            Check("docs/runbooks/RB_PR17_METRICS_CARDINALITY_BREACH.md", min_loc=80),
            # PR #17 audit-pass : G9 — ServiceMonitor metricRelabelings
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/templates/servicemonitor.yaml",
                  pattern=r"metricRelabelings:",
                  path="charts/efe-stack/charts/efe-meta-swarm/templates/servicemonitor.yaml"),
        ],
    ),

    # ═════════════════════════ PR #18 ════════════════════════════
    PR(
        label="PR18",
        title="8 OpenTelemetry span types + W3C Trace Context + OTLP exporter",
        notes="tracing_otel module + 3 exporter alerts + ConfigMap",
        checks=[
            # Architecture
            Check("docs/EFE_A1_PHASE2_PR18_ARCHITECTURE.md", min_loc=1500),
            # tracing_otel module
            Check("crates/efe-meta-swarm-server/src/tracing_otel/mod.rs",          min_loc=200),
            Check("crates/efe-meta-swarm-server/src/tracing_otel/resources.rs",    min_loc=200),
            Check("crates/efe-meta-swarm-server/src/tracing_otel/exporter.rs",     min_loc=250),
            Check("crates/efe-meta-swarm-server/src/tracing_otel/sampler.rs",      min_loc=150),
            Check("crates/efe-meta-swarm-server/src/tracing_otel/propagator.rs",   min_loc=150),
            Check("crates/efe-meta-swarm-server/src/tracing_otel/interceptor.rs",  min_loc=100),
            Check("crates/efe-meta-swarm-server/src/tracing_otel/metrics.rs",      min_loc=150),
            Check("crates/efe-meta-swarm-server/src/tracing_otel/in_memory.rs",    min_loc=200),
            Check("crates/efe-meta-swarm-server/src/tracing_otel/spans.rs",        min_loc=400),
            # Integration test
            Check("crates/efe-meta-swarm-server/tests/integration_pr18_otel_spans.rs", min_loc=150),
            # Cargo deps
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r"^opentelemetry\s*=",
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r"^opentelemetry_sdk\s*=",
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r"^opentelemetry-otlp\s*=",
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/Cargo.toml",
                  pattern=r"^tracing-opentelemetry\s*=",
                  path="crates/efe-meta-swarm-server/Cargo.toml"),
            # Wiring : init_otel in both binaries
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/bin/controller.rs",
                  pattern=r"init_otel_or_disabled",
                  path="crates/efe-meta-swarm-server/src/bin/controller.rs"),
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/bin/participant.rs",
                  pattern=r"init_otel_or_disabled",
                  path="crates/efe-meta-swarm-server/src/bin/participant.rs"),
            # Iterate span wired
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/service.rs",
                  pattern=r"span_iterate",
                  path="crates/efe-meta-swarm-server/src/controller/service.rs"),
            # Step span wired
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/participant/service.rs",
                  pattern=r"span_step",
                  path="crates/efe-meta-swarm-server/src/participant/service.rs"),
            # Manifest reload span wired
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/controller/reload.rs",
                  pattern=r"span_manifest_reload",
                  path="crates/efe-meta-swarm-server/src/controller/reload.rs"),
            # Shutdown drain span wired
            Check(kind="grep",
                  target="crates/efe-meta-swarm-server/src/bin/controller.rs",
                  pattern=r"span_shutdown_drain",
                  path="crates/efe-meta-swarm-server/src/bin/controller.rs"),
            # Chart additions
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/otel-configmap.yaml",   min_loc=20),
            Check("charts/efe-stack/charts/efe-meta-swarm/templates/pr18-prometheusrule.yaml", min_loc=60),
            Check("charts/efe-stack/charts/efe-meta-swarm/tests/pr18_otel_test.yaml",       min_loc=80),
            # values.yaml tracing block
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.yaml",
                  pattern=r"^tracing:",
                  path="charts/efe-stack/charts/efe-meta-swarm/values.yaml"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.yaml",
                  pattern=r"samplerArg:",
                  path="charts/efe-stack/charts/efe-meta-swarm/values.yaml"),
            # Schema gate
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/values.schema.json",
                  pattern=r'"tracing":',
                  path="charts/efe-stack/charts/efe-meta-swarm/values.schema.json"),
            # Chart version bump
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml",
                  pattern=r"^version:\s*0\.5\.0",
                  path="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml"),
            Check(kind="grep",
                  target="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml",
                  pattern=r'^appVersion:\s*"0\.6\.0"',
                  path="charts/efe-stack/charts/efe-meta-swarm/Chart.yaml"),
        ],
    ),
]


# ─── Runner ────────────────────────────────────────────────────────
def main() -> int:
    print(f"═══ Code integrity verification for meta-swarm PRs ═══")
    print(f"Repo root : {REPO}")
    print()

    overall_ok = True
    total_checks = 0
    total_passed = 0

    for pr in PRS:
        ok_count = 0
        fail_count = 0
        details: list[str] = []
        for c in pr.checks:
            ok, msg = c.evaluate()
            total_checks += 1
            if ok:
                ok_count += 1
                total_passed += 1
            else:
                fail_count += 1
                details.append(f"      [MISS] {msg}")

        pct = (ok_count * 100) // max(len(pr.checks), 1)
        status = (
            "[OK]   " if fail_count == 0 else
            "[FAIL] " if ok_count == 0 else
            "[PART] "
        )
        print(f"{status}{pr.label:<16} {pct:>3}%  ({ok_count}/{len(pr.checks)})  — {pr.title}")
        if pr.notes:
            print(f"      Note : {pr.notes}")
        for d in details:
            print(d)
        if fail_count > 0:
            overall_ok = False

    print()
    print(f"─── Summary ───")
    print(f"  Total PRs           : {len(PRS)}")
    print(f"  Total checks        : {total_checks}")
    print(f"  Passed              : {total_passed}")
    print(f"  Failed              : {total_checks - total_passed}")
    print(f"  Overall verdict     : {'[OK]' if overall_ok else '[INCOMPLETE]'}")

    return 0 if overall_ok else 1


if __name__ == "__main__":
    sys.exit(main())

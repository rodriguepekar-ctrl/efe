"""Sprint **D40 (Φ-F)** — pin tests for the quarterly DR drill runbook.

Anti-regression : guards M7 D40 surface :

  * `dr/run_quarterly_drill.sh` exists, executable, declares 6 phases,
    cleanup trap, exit codes 1..12.
  * Helm CronJob `d40-dr-drill-quarterly-cronjob.yaml` exists with
    quarterly schedule, references the script, lists 4 compliance
    frameworks, mounts evidence PVC + DinD socket.
  * `values.yaml` exposes `backup.drillQuarterly.*` keys with the
    expected defaults (schedule, RTO target 2700s, Hello-World
    block, complianceEvidence with Object Lock COMPLIANCE +
    7-year retention).
  * Pin the JSON schemas (`helloworld-1.0.0.schema.json`,
    `dr-drill-outcome-2.0.0.schema.json`) for shape stability.
  * `docs/docs/howto/backup-and-restore.md` documents the
    quarterly drill section with the canonical script reference.
  * `docs/compliance/dr_drill_quarterly_evidence.md` lists the
    4 frameworks (SOC 2, ISO 27001, NIS2, RGPD).
  * `.ci-evidence/dr-drill-quarterly/README.md` documents the
    per-quarter directory layout.
  * CHANGELOG mentions D40.

Cf. `docs/EFE_BACKUP_EXTENSION_ARCHITECTURE.md` § 9.4.
"""
from __future__ import annotations

import json
import os
import re
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]

SCRIPT = REPO_ROOT / "dr" / "run_quarterly_drill.sh"
HELM_TEMPLATE = (
    REPO_ROOT
    / "deploy"
    / "helm"
    / "efe"
    / "templates"
    / "d40-dr-drill-quarterly-cronjob.yaml"
)
VALUES_YAML = REPO_ROOT / "deploy" / "helm" / "efe" / "values.yaml"
RUNBOOK_DOC = REPO_ROOT / "docs" / "docs" / "howto" / "backup-and-restore.md"
COMPLIANCE_DOC = REPO_ROOT / "docs" / "compliance" / "dr_drill_quarterly_evidence.md"
HELLOWORLD_SCHEMA = (
    REPO_ROOT / "docs" / "compliance" / "helloworld-1.0.0.schema.json"
)
OUTCOME_SCHEMA = (
    REPO_ROOT / "docs" / "compliance" / "dr-drill-outcome-2.0.0.schema.json"
)
EVIDENCE_README = (
    REPO_ROOT / ".ci-evidence" / "dr-drill-quarterly" / "README.md"
)
CHANGELOG = REPO_ROOT / "CHANGELOG.md"
ARCH_DOC = REPO_ROOT / "docs" / "EFE_BACKUP_EXTENSION_ARCHITECTURE.md"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# §9.4 — script existence + structure
# ─────────────────────────────────────────────────────────────────────


def test_script_exists_and_executable() -> None:
    assert SCRIPT.exists(), f"missing {SCRIPT}"
    assert SCRIPT.stat().st_mode & 0o111, "script must have execute bit set"


def test_script_uses_strict_bash_mode() -> None:
    content = _read(SCRIPT)
    assert "set -euo pipefail" in content
    # IFS reset prevents word-split surprises on tenant ids etc.
    assert "IFS=$'\\n\\t'" in content


def test_script_has_six_phases() -> None:
    content = _read(SCRIPT)
    for phase in [
        "phase_kind_setup",
        "phase_helm_install",
        "phase_restore",
        "phase_helloworld",
        "phase_verifier_suite",
        "emit_evidence",
    ]:
        assert f"{phase}()" in content, f"phase function missing: {phase}"


def test_script_has_cleanup_trap() -> None:
    """**B-6 directing principle** — cleanup obligatoire."""
    content = _read(SCRIPT)
    assert "trap cleanup EXIT" in content
    assert "kind delete cluster" in content


def test_script_args_accept_cluster_name_and_keep_and_evidence_dir() -> None:
    content = _read(SCRIPT)
    for flag in ["--cluster-name", "--backup-id", "--evidence-dir", "--keep", "--target-env"]:
        assert flag in content, f"flag {flag} not parsed"


def test_script_refuses_prod_target_env() -> None:
    """**B-8 directing principle** — cross-env safety."""
    content = _read(SCRIPT)
    # Must reject prod / production at parse time.
    assert "prod|production|PROD|PRODUCTION" in content
    assert "forbidden for the drill (B-8)" in content


def test_script_documents_exit_codes() -> None:
    content = _read(SCRIPT)
    # Distinct exit codes per failure category (mirrors restore.sh + D40).
    for ec in [
        "exit 1",     # arg parse
        "exit 2",     # pre-flight
        "exit 3",     # kind setup fail
        "exit 4",     # helm install fail
        "exit 5",     # restore fail
        "exit 6",     # helloworld fail
        "exit 7",     # verifier suite fail
        "exit 12",    # helloworld latency violation
    ]:
        assert ec in content, f"missing distinct exit: {ec}"


def test_script_helloworld_has_six_steps() -> None:
    content = _read(SCRIPT)
    for step in [
        "Step 1/6 — register_catalog",
        "Step 2/6 — register_class",
        "Step 3/6 — register_sentence",
        "Step 4/6 — resolve_catalog",
        "Step 5/6 — execute_workflow",
        "Step 6/6 — assert_response",
    ]:
        assert step in content, f"helloworld step missing: {step}"


def test_script_emits_outcome_and_attestation() -> None:
    content = _read(SCRIPT)
    assert "OUTCOME_JSON=" in content
    assert "ATTESTATION_TXT=" in content
    assert "TIMING_JSON=" in content


def test_script_signs_attestation_with_ed25519() -> None:
    """Attestation pipeline best-effort : signed when key present,
    unsigned otherwise (graceful degradation, mirrors D17 § AC3)."""
    content = _read(SCRIPT)
    assert "sign_attestation" in content
    assert "Ed25519" in content or "ed25519" in content.lower()


def test_script_lists_compliance_frameworks() -> None:
    content = _read(SCRIPT)
    for fw in ["SOC2-CC9.1", "ISO27001-A.17.1.3", "NIS2-Art21", "RGPD-Art32"]:
        assert fw in content, f"framework label missing in script: {fw}"


def test_script_helloworld_obeys_60s_latency_cap() -> None:
    """SLO D40-DR-2 — Hello-World latency ≤ 60 s."""
    content = _read(SCRIPT)
    # The script enforces HELLOWORLD_MAX_LATENCY_MS with default 60000.
    assert "HELLOWORLD_MAX_LATENCY_MS" in content
    assert "60000" in content
    # Exit code 12 is reserved for SLO D40-DR-2 violation.
    assert "SLO D40-DR-2" in content


# ─────────────────────────────────────────────────────────────────────
# §9.3 — Helm CronJob template
# ─────────────────────────────────────────────────────────────────────


def test_helm_cronjob_template_exists() -> None:
    assert HELM_TEMPLATE.exists(), f"missing {HELM_TEMPLATE}"


def test_helm_cronjob_uses_run_quarterly_drill_script() -> None:
    content = _read(HELM_TEMPLATE)
    assert "/usr/local/bin/run_quarterly_drill.sh" in content


def test_helm_cronjob_declares_compliance_framework_labels() -> None:
    content = _read(HELM_TEMPLATE)
    assert 'compliance.efe.ai/framework: "soc2,iso27001,nis2,rgpd"' in content
    for ann in [
        'compliance.efe.ai/soc2-criterion: "CC9.1"',
        'compliance.efe.ai/iso27001-control: "A.17.1.3"',
        'compliance.efe.ai/nis2-article: "Art21.2(c)"',
        'compliance.efe.ai/rgpd-article: "Art32.1(c,d)"',
    ]:
        assert ann in content, f"compliance annotation missing: {ann}"


def test_helm_cronjob_concurrency_forbid_and_no_retry() -> None:
    """B-5 fail-fast : no retry, drills don't paper over failures."""
    content = _read(HELM_TEMPLATE)
    assert "concurrencyPolicy: Forbid" in content
    assert "backoffLimit: 0" in content


def test_helm_cronjob_active_deadline_60min() -> None:
    content = _read(HELM_TEMPLATE)
    # default 3600s = 60 min hard timeout.
    assert "default 3600" in content


def test_helm_cronjob_history_limits_8q() -> None:
    """8 quarters of history ≈ 2 years on-cluster (B-3 says long-term
    archive lives in S3 Object Lock — but on-cluster needs to keep
    enough for at-a-glance audit)."""
    content = _read(HELM_TEMPLATE)
    assert "successfulJobsHistoryLimit: 8" in content
    assert "failedJobsHistoryLimit: 8" in content


def test_helm_cronjob_mounts_compliance_pvc_and_dind_socket() -> None:
    content = _read(HELM_TEMPLATE)
    assert "name: compliance-evidence" in content
    assert "name: dind" in content
    assert "/var/run/docker.sock" in content
    assert "/var/lib/efe/compliance-evidence" in content


def test_helm_cronjob_mounts_attestation_key_optional() -> None:
    """Attestation key is optional in dev/kind clusters (B-3 caveat) ;
    SealedSecret-mounted in production."""
    content = _read(HELM_TEMPLATE)
    assert "name: attestation-key" in content
    assert "secretName: " in content and "-attestation-key" in content
    assert "optional: true" in content


def test_helm_cronjob_resources_8cpu_16gi() -> None:
    """The drill is heavier than weekly — kind-in-pod + minio + cnpg."""
    content = _read(HELM_TEMPLATE)
    assert 'cpu: "8"' in content
    assert "memory: 16Gi" in content


def test_helm_cronjob_renders_pvc_companion_object() -> None:
    """The CronJob ships with a PVC for the compliance-evidence volume."""
    content = _read(HELM_TEMPLATE)
    assert "kind: PersistentVolumeClaim" in content
    assert "compliance-evidence" in content


# ─────────────────────────────────────────────────────────────────────
# values.yaml drillQuarterly section
# ─────────────────────────────────────────────────────────────────────


def test_values_yaml_drill_quarterly_section_present() -> None:
    content = _read(VALUES_YAML)
    assert "drillQuarterly:" in content
    # Must come AFTER the existing drill: section.
    drill_idx = content.find("drill:")
    quarterly_idx = content.find("drillQuarterly:")
    assert drill_idx > 0 and quarterly_idx > drill_idx


def test_values_yaml_drill_quarterly_schedule_is_quarterly_cron() -> None:
    """Default schedule "0 4 1 1,4,7,10 *" = 1st of each quarter, 04:00 UTC."""
    content = _read(VALUES_YAML)
    assert '"0 4 1 1,4,7,10 *"' in content


def test_values_yaml_drill_quarterly_rto_target_45min() -> None:
    content = _read(VALUES_YAML)
    # 2700 s = 45 min — relaxed vs weekly's 1800 s.
    assert "rtoTargetSeconds: 2700" in content


def test_values_yaml_drill_quarterly_active_deadline_60min() -> None:
    content = _read(VALUES_YAML)
    # 3600 s = 60 min hard timeout.
    assert "activeDeadlineSeconds: 3600" in content


def test_values_yaml_helloworld_subsection() -> None:
    content = _read(VALUES_YAML)
    for key in [
        "helloWorld:",
        "tenantId:",
        "catalogName:",
        "className:",
        "sentence:",
        "maxLatencyMs:",
        "fallbackMockBundle:",
    ]:
        assert key in content, f"helloWorld subkey missing: {key}"


def test_values_yaml_helloworld_default_disables_mock_fallback() -> None:
    """B-2 — refuse mock embedder by default."""
    content = _read(VALUES_YAML)
    # Find the line `fallbackMockBundle:` in drillQuarterly section.
    m = re.search(r"fallbackMockBundle:\s*(\S+)", content)
    assert m is not None, "fallbackMockBundle missing"
    assert m.group(1).lower() in ("false", '"false"'), \
        f"fallbackMockBundle MUST default to false (B-2). Got: {m.group(1)!r}"


def test_values_yaml_compliance_evidence_subsection() -> None:
    content = _read(VALUES_YAML)
    for key in [
        "complianceEvidence:",
        "bucket:",
        "objectLockMode:",
        "retentionYears:",
        "attestationKeyId:",
    ]:
        assert key in content, f"complianceEvidence subkey missing: {key}"


def test_values_yaml_object_lock_mode_compliance() -> None:
    """B-3 — Object Lock COMPLIANCE mode (vs Governance) is mandatory
    for SOC 2 / RGPD evidence (un-deletable even by root admin)."""
    content = _read(VALUES_YAML)
    assert 'objectLockMode: "COMPLIANCE"' in content


def test_values_yaml_retention_7y() -> None:
    """SOX/SOC2 require ≥ 7-year retention for control-test evidence."""
    content = _read(VALUES_YAML)
    assert "retentionYears: 7" in content


def test_values_yaml_drill_quarterly_yaml_parses() -> None:
    """Sanity : the YAML loads cleanly with PyYAML."""
    content = _read(VALUES_YAML)
    parsed = yaml.safe_load(content)
    backup = parsed.get("backup", {})
    drill_q = backup.get("drillQuarterly", {})
    assert drill_q.get("enabled") is True
    assert drill_q.get("rtoTargetSeconds") == 2700
    assert drill_q.get("schedule") == "0 4 1 1,4,7,10 *"
    hw = drill_q.get("helloWorld", {})
    assert hw.get("maxLatencyMs") == 60000
    assert hw.get("fallbackMockBundle") is False


# ─────────────────────────────────────────────────────────────────────
# JSON schemas (helloworld + outcome v2)
# ─────────────────────────────────────────────────────────────────────


def test_helloworld_schema_exists_and_loads() -> None:
    assert HELLOWORLD_SCHEMA.exists()
    schema = json.loads(HELLOWORLD_SCHEMA.read_text(encoding="utf-8"))
    assert schema["$schema"] == "http://json-schema.org/draft-07/schema#"
    assert schema["properties"]["schema_version"]["const"] == "1.0.0"


def test_helloworld_schema_required_fields_pinned() -> None:
    schema = json.loads(HELLOWORLD_SCHEMA.read_text(encoding="utf-8"))
    required = set(schema["required"])
    expected = {
        "schema_version",
        "tenant_id",
        "catalog_name",
        "final_version",
        "class_name",
        "sentence",
        "anchors_compiled",
        "execute",
        "hmac_chain_post_execute_ok",
        "all_steps_ok",
        "elapsed_secs",
    }
    assert expected.issubset(required), \
        f"missing required fields in schema: {expected - required}"


def test_outcome_v2_schema_exists_and_loads() -> None:
    assert OUTCOME_SCHEMA.exists()
    schema = json.loads(OUTCOME_SCHEMA.read_text(encoding="utf-8"))
    assert schema["properties"]["schema_version"]["const"] == "2.0.0"


def test_outcome_v2_schema_drill_kind_enum() -> None:
    schema = json.loads(OUTCOME_SCHEMA.read_text(encoding="utf-8"))
    drill_kind = schema["properties"]["drill_kind"]
    assert set(drill_kind["enum"]) == {"weekly", "quarterly"}


def test_outcome_v2_schema_compliance_frameworks_enum() -> None:
    schema = json.loads(OUTCOME_SCHEMA.read_text(encoding="utf-8"))
    fw_enum = schema["properties"]["compliance_frameworks"]["items"]["enum"]
    assert set(fw_enum) == {
        "SOC2-CC9.1",
        "ISO27001-A.17.1.3",
        "NIS2-Art21",
        "RGPD-Art32",
    }


def test_outcome_v2_schema_verifiers_min5() -> None:
    """5 canonical verifiers : audit_chain, bundle_signatures,
    oidc_issuers, catalog_smoke, execute_smoke."""
    schema = json.loads(OUTCOME_SCHEMA.read_text(encoding="utf-8"))
    verifiers = schema["properties"]["verifiers"]
    assert verifiers["minItems"] == 5
    kinds = verifiers["items"]["properties"]["kind"]["enum"]
    assert set(kinds) == {
        "audit_chain",
        "bundle_signatures",
        "oidc_issuers",
        "catalog_smoke",
        "execute_smoke",
    }


# ─────────────────────────────────────────────────────────────────────
# Documentation (runbook + compliance)
# ─────────────────────────────────────────────────────────────────────


def test_runbook_doc_mentions_quarterly_drill_section() -> None:
    content = _read(RUNBOOK_DOC)
    assert "Quarterly DR drill" in content
    assert "run_quarterly_drill.sh" in content


def test_runbook_doc_links_compliance_evidence() -> None:
    content = _read(RUNBOOK_DOC)
    assert "dr_drill_quarterly_evidence.md" in content


def test_compliance_doc_exists() -> None:
    assert COMPLIANCE_DOC.exists()


def test_compliance_doc_lists_four_frameworks() -> None:
    content = _read(COMPLIANCE_DOC)
    for fw in ["SOC 2", "ISO 27001", "NIS2", "RGPD"]:
        assert fw in content, f"framework not documented: {fw}"


def test_compliance_doc_lists_d40_slos() -> None:
    content = _read(COMPLIANCE_DOC)
    for slo in ["D40-DR-1", "D40-DR-2", "D40-DR-3", "D40-DR-4"]:
        assert slo in content, f"SLO {slo} not documented"


def test_compliance_doc_documents_failed_drill_handling() -> None:
    """Failed drills MUST emit evidence too — auditors need the failure
    record as much as the success record."""
    content = _read(COMPLIANCE_DOC)
    assert "Failed-drill handling" in content or "failed drill" in content.lower()
    assert "post-mortem" in content.lower()


def test_evidence_readme_exists_and_documents_layout() -> None:
    assert EVIDENCE_README.exists()
    content = _read(EVIDENCE_README)
    # Must list the 8 expected files in the per-quarter dir.
    for f in [
        "outcome.json",
        "kind-cluster.log",
        "helm-install.log",
        "restore.log",
        "helloworld.log",
        "verifier-suite.log",
        "timing.json",
        "attestation.txt",
    ]:
        assert f in content, f"per-quarter file not documented: {f}"


# ─────────────────────────────────────────────────────────────────────
# CHANGELOG
# ─────────────────────────────────────────────────────────────────────


def test_changelog_mentions_d40_extension() -> None:
    content = _read(CHANGELOG)
    assert "D40" in content
    # The extension entry should match the established pattern.
    assert (
        "M7 extension" in content or "efe-backup (DR drill quarterly)" in content
    ), "CHANGELOG missing M7 extension entry"


def test_changelog_d40_lists_compliance_frameworks() -> None:
    """Anti-doc-drift : the CHANGELOG entry MUST list the 4 frameworks
    so the audit log of the project itself reflects the contract."""
    content = _read(CHANGELOG)
    # Find the D40 entry block.
    m = re.search(r"###.*D40.*?(?=^###|\Z)", content, re.S | re.M)
    assert m, "no D40 section found in CHANGELOG"
    block = m.group(0)
    for fw in ["SOC 2", "ISO 27001", "NIS2", "RGPD"]:
        assert fw in block, f"CHANGELOG D40 entry missing framework: {fw}"


# ─────────────────────────────────────────────────────────────────────
# Architecture document cross-reference
# ─────────────────────────────────────────────────────────────────────


def test_architecture_doc_exists_and_references_d40() -> None:
    assert ARCH_DOC.exists()
    content = _read(ARCH_DOC)
    assert "D40" in content
    # Must declare the GA target with 2 cadences.
    assert "weekly" in content.lower() and "quarterly" in content.lower()


# ─────────────────────────────────────────────────────────────────────
# Audit fixes M7-D40-A1..A24 — pin tests anti-régression
#
# Each fix is codified by a dedicated test. A future refactor that
# accidentally regresses one of the audit-fixed defects will trip the
# corresponding test.
# ─────────────────────────────────────────────────────────────────────


def test_audit_fix_a1_cleanup_preserves_emit_evidence_rc() -> None:
    """**Audit fix M7-D40-A1 (CRIT)** — `cleanup()` MUST preserve the
    return code of `emit_evidence` so a successful drill that violates
    RTO (rc=11 from emit_evidence) is surfaced as the script exit
    code. Pre-audit the `|| log` in cleanup swallowed the rc → SLO
    D40-DR-1 violations were silent."""
    content = _read(SCRIPT)
    # The fix uses `local emit_rc=0 ; emit_evidence "$rc" || emit_rc=$?`
    # then promotes emit_rc to rc when non-zero.
    assert "emit_rc=0" in content
    assert "emit_evidence \"$rc\" || emit_rc=$?" in content
    assert 'rc=$emit_rc' in content
    # Anchor : the audit fix marker is in the docstring of cleanup.
    assert "Audit fix M7-D40-A1" in content


def test_audit_fix_a6_helm_volumes_conditional() -> None:
    """**Audit fix M7-D40-A6 (CRIT)** — Helm CronJob volumes +
    volumeMounts referencing `compliance-evidence` are conditionally
    rendered ONLY when `complianceEvidence.enabled == true`.
    Pre-audit a config with `enabled=false` rendered a volumeMount
    pointing at a non-existent PVC → CrashLoopBackOff."""
    content = _read(HELM_TEMPLATE)
    # The fix introduces $evidenceEnabled := .Values...complianceEvidence.enabled
    assert "$evidenceEnabled :=" in content
    # Both the volumeMount AND the volume MUST be guarded by `if $evidenceEnabled`.
    # We require at least 3 occurrences (mount, volume, args, PVC creation).
    occurrences = content.count("if $evidenceEnabled")
    assert occurrences >= 3, (
        f"expected at least 3 conditional gates on $evidenceEnabled, "
        f"got {occurrences}"
    )
    # Anchor the audit fix marker.
    assert "Audit fix M7-D40-A6" in content


def test_audit_fix_a14_helloworld_uses_grpcurl_not_admin_subcmd() -> None:
    """**Audit fix M7-D40-A14 (CRIT)** — Hello-World steps use
    `grpcurl` against the live gRPC endpoint (CatalogService +
    SwarmService) instead of `efe-server admin <cmd>` subcommands
    that DO NOT exist in the v0.1.18 binary."""
    content = _read(SCRIPT)
    # Step 1-5 must call grpcurl, not `admin register-catalog` etc.
    assert "grpcurl" in content
    # Verify the canonical gRPC services are referenced.
    assert "efe.v1.catalog.CatalogService/RegisterCatalog" in content
    assert "efe.v1.catalog.CatalogService/EvolveCatalog" in content
    assert "efe.v1.catalog.CatalogService/GetCatalog" in content
    assert "efe.v1.SwarmService/Execute" in content
    # The phantom `admin register-catalog` family MUST NOT appear.
    assert "admin register-catalog" not in content, \
        "Hello-World must not invoke phantom `efe-server admin register-catalog`"
    assert "admin evolve-catalog" not in content
    assert "admin canary-execute" not in content
    # Audit fix marker.
    assert "Audit fix M7-D40-A14" in content


def test_audit_fix_a14_helloworld_uses_response_keys_not_hardcoded_versions() -> None:
    """**Audit fix M7-D40-A14 sub-fix** — the v2 / v3 catalog keys for
    EvolveCatalog steps 3+4 are read from the previous step's response
    `.key` field (more robust than hardcoding `@v2` / `@v3` which
    assumes monotonic increment by 1)."""
    content = _read(SCRIPT)
    assert 'from_key_v2=$(jq -r' in content or "from_key_v2=$(jq -r '.key" in content
    assert "from_key_v3=$(jq -r" in content


def test_audit_fix_a17_backup_id_auto_resolve() -> None:
    """**Audit fix M7-D40-A17 (CRIT)** — `--backup-id` is auto-resolved
    via `efe-backup list --since=7d` when `EFE_BACKUP_AUTO_RESOLVE=true`.
    The Helm CronJob sets this env var so it can run unattended.
    Pre-audit the script exit'd 1 immediately if --backup-id was
    missing → CronJob crashloop without evidence."""
    content = _read(SCRIPT)
    assert "EFE_BACKUP_AUTO_RESOLVE" in content
    assert "auto_resolve_backup_id" in content
    # The arg-parse MUST allow an empty BACKUP_ID when AUTO_RESOLVE=true.
    assert "EFE_BACKUP_AUTO_RESOLVE:-false" in content
    # The Helm CronJob MUST set the env var to true.
    helm = _read(HELM_TEMPLATE)
    assert "EFE_BACKUP_AUTO_RESOLVE" in helm
    assert 'value: "true"' in helm
    # Audit fix marker.
    assert "Audit fix M7-D40-A17" in content


def test_audit_fix_a24_helm_install_uses_reuse_values() -> None:
    """**Audit fix M7-D40-A24 (CRIT)** — `phase_helm_install` uses
    `helm upgrade --reuse-values` (not `--install` from-scratch),
    so the base values planted by `setup-test-env.sh` (storage.bucket,
    storage.region, bundles.sourceBucket, bundles.destBucket — all
    REQUIRED when backup.enabled=true) survive the upgrade. Pre-audit
    the fresh `helm upgrade --install` reset those fields → chart
    crashed at validation."""
    content = _read(SCRIPT)
    # The fix uses `helm upgrade efe ... --reuse-values --set ...`.
    assert "--reuse-values" in content
    # And a comment explaining why.
    assert "Audit fix M7-D40-A24" in content
    # The phase MUST first verify the base release exists (not blindly install).
    assert "helm status efe" in content


def test_audit_fix_a7_portable_date_arithmetic() -> None:
    """**Audit fix M7-D40-A7 (HIGH)** — replace `date +%-m` (GNU
    extension that fails on busybox / Alpine) with portable bash
    arithmetic that strips the leading zero."""
    content = _read(SCRIPT)
    # The fix introduces a `_drill_quarter` helper.
    assert "_drill_quarter()" in content
    # The helper MUST handle leading-zero parsing via base-10 force.
    assert "10#$month_str" in content or "10#${month_str}" in content
    # The original `+%-m` (GNU-specific) is gone from arithmetic contexts.
    # We allow it inside the helper as a comment but not as an expression.
    assert "$(( ($(date -u +%-m)" not in content


def test_audit_fix_a18_dev_secret_values_configurable() -> None:
    """**Audit fix M7-D40-A18 (HIGH)** — `backup.devSecretValues=true`
    is opt-in via env var (default false). Pre-audit the script
    hardcoded `--set backup.devSecretValues=true` → SecOps risk in
    production drills (chart's dev defaults injected over real
    SealedSecrets)."""
    content = _read(SCRIPT)
    assert "DEV_SECRET_VALUES" in content
    # The default MUST be false.
    assert 'EFE_DRILL_DEV_SECRET_VALUES:-false' in content
    # values.yaml + Helm CronJob MUST default to false.
    values = _read(VALUES_YAML)
    assert "devSecretValues: false" in values
    helm = _read(HELM_TEMPLATE)
    assert "EFE_DRILL_DEV_SECRET_VALUES" in helm


def test_audit_fix_a19_namespace_configurable() -> None:
    """**Audit fix M7-D40-A19 (HIGH)** — `--namespace` arg + env var
    `EFE_DRILL_NAMESPACE` make the K8s namespace configurable. Pre-audit
    `--namespace efe` was hardcoded → drill couldn't run in a separate
    drill ns."""
    content = _read(SCRIPT)
    assert '--namespace)      NAMESPACE="$2"' in content
    assert "EFE_DRILL_NAMESPACE" in content
    # The kubectl exec calls MUST use $NAMESPACE, not a hardcoded "efe".
    assert 'kubectl --namespace "$NAMESPACE" exec' in content


def test_audit_fix_a16_evidence_dir_absolute_path() -> None:
    """**Audit fix M7-D40-A16 (MED)** — EVIDENCE_DIR is converted to
    absolute path before any phase runs. Pre-audit a relative
    `--evidence-dir ./...` was ambiguous when phases changed CWD."""
    content = _read(SCRIPT)
    # The fix uses `EVIDENCE_DIR="$(cd "$EVIDENCE_DIR" && pwd)"`.
    assert 'EVIDENCE_DIR="$(cd "$EVIDENCE_DIR" && pwd)"' in content


def test_audit_fix_a8_helloworld_fail_step_captured() -> None:
    """**Audit fix M7-D40-A8 (MED)** — `HELLOWORLD_FAIL_STEP` captures
    which step failed, surfaced in the log + outcome.json so an
    auditor reading a failed drill can immediately pinpoint the
    failure mode without grepping the helloworld.log."""
    content = _read(SCRIPT)
    assert "HELLOWORLD_FAIL_STEP=" in content
    # Each step MUST set its own marker.
    for step in [
        '1_register_catalog',
        '2_register_class',
        '3_register_sentence',
        '4_resolve_catalog',
        '5_execute_workflow',
        '6_assert_hmac_chain',
    ]:
        assert step in content, f"helloworld fail step marker missing: {step}"


def test_audit_fix_a9_helm_timeout_configurable() -> None:
    """**Audit fix M7-D40-A9 (MED)** — `helm upgrade --timeout` is
    configurable via env var `EFE_DRILL_HELM_TIMEOUT` + values.yaml
    `drillQuarterly.helmTimeout`."""
    content = _read(SCRIPT)
    assert "EFE_DRILL_HELM_TIMEOUT" in content
    assert 'HELM_TIMEOUT="${EFE_DRILL_HELM_TIMEOUT:-5m}"' in content
    values = _read(VALUES_YAML)
    assert "helmTimeout:" in values


def test_audit_helm_cronjob_passes_namespace_arg_to_script() -> None:
    """The Helm CronJob MUST pass the runtime namespace to the script
    via `--namespace=` arg, so the script's kubectl calls hit the
    right ns."""
    content = _read(HELM_TEMPLATE)
    assert "--namespace={{ .Release.Namespace }}" in content


def test_audit_script_documents_all_audit_fix_markers() -> None:
    """Anti-doc-drift : the script header MUST mention the post-audit
    pass + reference the CHANGELOG audit fixes section so a future
    reader knows to consult it before refactoring."""
    content = _read(SCRIPT)
    assert "Post-audit pass 1" in content
    assert "M7-D40-A" in content


def test_audit_changelog_lists_audit_fixes() -> None:
    """The CHANGELOG audit fix section MUST exist and list all 5
    CRIT defects (A1, A6, A14, A17, A24) by ID."""
    content = _read(CHANGELOG)
    # Match the audit fixes section.
    m = re.search(r"#### Audit fixes — M7.*?(?=^####|^###|\Z)", content, re.S | re.M)
    assert m, "no M7 audit fixes section found in CHANGELOG"
    block = m.group(0)
    for defect in ["M7-D40-A1", "M7-D40-A6", "M7-D40-A14", "M7-D40-A17", "M7-D40-A24"]:
        assert defect in block, (
            f"CHANGELOG audit fixes section missing defect: {defect}"
        )

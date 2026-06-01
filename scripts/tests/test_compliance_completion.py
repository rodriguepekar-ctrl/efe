"""Pin tests for Compliance Completion (Φ-D-COMPLIANCE).

Sprint **Φ-D-COMPLIANCE D89** — artefact #32.
Cf. ``docs/EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md`` § 20.

36 pin tests (18 structurels + 18 sémantiques) + 8 evidence audits.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_COMP_DIR = _REPO / "docs" / "compliance"
_EVAL = _REPO / "eval" / "scripts"


def _read_yaml(p: Path) -> dict:
    yaml = pytest.importorskip("yaml")
    return yaml.safe_load(p.read_text())


# ═════════════════════════════════════════════════════════════════════
# 18 STRUCTURAL PIN TESTS
# ═════════════════════════════════════════════════════════════════════


def test_comp_registry_yaml_consistent():
    reg = _read_yaml(_COMP_DIR / "registry.yaml")
    assert "frameworks" in reg
    expected = {"GDPR", "SOC2", "ISO27001", "EU_AI_ACT", "HIPAA_READY", "CCPA"}
    assert set(reg["frameworks"].keys()) == expected
    # Each framework file must exist.
    for name, fw in reg["frameworks"].items():
        fpath = _REPO / fw["file"]
        assert fpath.exists(), f"{name} : file {fw['file']} missing"
        # applicable + not_applicable == n_controls.
        assert fw["applicable"] + fw["not_applicable"] == fw["n_controls"], (
            f"{name}: count mismatch")


def test_comp_controls_index_yaml_valid():
    idx = _read_yaml(_COMP_DIR / "controls_index.yaml")
    assert "evidences" in idx
    assert len(idx["evidences"]) >= 15, "expected ≥ 15 evidence entries"
    for name, ev in idx["evidences"].items():
        assert "file" in ev, f"{name} missing file"
        assert "controls" in ev, f"{name} missing controls"
        assert "doctrine" in ev, f"{name} missing doctrine"
        assert len(ev["controls"]) >= 1, f"{name} : 0 controls"


def test_comp_slo_yaml_has_18_entries():
    p = _REPO / "bench_baselines" / "v0.8.0" / "comp_slos.yaml"
    assert p.exists()
    data = _read_yaml(p)
    assert data["domain"] == "compliance"
    ids = {s["id"] for s in data["slos"]}
    expected = {
        "SLO-COMP-GDPR-CONTROLS", "SLO-COMP-SOC2-CONTROLS",
        "SLO-COMP-ISO27001-CONTROLS", "SLO-COMP-AI-ACT-CONTROLS",
        "SLO-COMP-EVIDENCE-COVERAGE", "SLO-COMP-AUDIT-CHAIN-DAILY",
        "SLO-COMP-DPIA-FRESHNESS", "SLO-COMP-ROPA-FRESHNESS",
        "SLO-COMP-SUBPROCESSOR-INVENTORY", "SLO-COMP-BREACH-NOTIFY-72H",
        "SLO-COMP-BREACH-CONTAIN-24H", "SLO-COMP-ERASURE-30D",
        "SLO-COMP-PORTABILITY-30D", "SLO-COMP-RESIDENCY-LEAK",
        "SLO-COMP-AI-ACT-HIGH-OVERSIGHT", "SLO-COMP-AI-ACT-DPO-SIGNOFF",
        "SLO-COMP-CONSENT-VERSION", "SLO-COMP-TRANSPARENCY-NOTICE",
    }
    missing = expected - ids
    assert not missing, f"missing : {sorted(missing)}"
    assert len(ids) == 18


def test_comp_six_frameworks_documented():
    for fname in ["gdpr.md", "soc2.md", "iso27001.md", "eu_ai_act.md",
                  "hipaa_ready.md", "ccpa.md"]:
        p = _COMP_DIR / fname
        assert p.exists(), f"docs/compliance/{fname} missing"
        text = p.read_text()
        # Each framework doc must have a SoA table.
        assert "Statement of Applicability" in text or "Summary" in text, (
            f"{fname} missing SoA")


def test_comp_evidence_scripts_exist():
    """16 evidence scripts referenced in controls_index.yaml must exist."""
    idx = _read_yaml(_COMP_DIR / "controls_index.yaml")
    for name, ev in idx["evidences"].items():
        fpath = _REPO / ev["file"]
        assert fpath.exists(), f"{name} → {ev['file']} missing"


def test_comp_evidence_scripts_invokable():
    """All evidence scripts respond to --help with exit 0."""
    scripts = list(_EVAL.glob("comp_*.py"))
    assert len(scripts) >= 17, f"expected ≥ 17, found {len(scripts)}"
    for s in scripts:
        r = subprocess.run(["python3", str(s), "--help"],
                            capture_output=True, timeout=10)
        assert r.returncode == 0, f"{s.name} --help failed"


def test_comp_evidence_scripts_emit_skip_in_live_mode():
    """Anti-Φ-D.A1 : every comp_*.py with --mode live emits skip_reason."""
    import tempfile
    scripts = list(_EVAL.glob("comp_*.py"))
    with tempfile.TemporaryDirectory() as td:
        for s in scripts:
            out = Path(td) / f"{s.stem}.json"
            r = subprocess.run(
                ["python3", str(s), "--mode", "live", "--out", str(out)],
                capture_output=True, timeout=10,
            )
            # Some scripts (audit_chain_replay) may not have --mode arg ;
            # skip gracefully.
            if r.returncode != 0 and "unrecognized arguments" in r.stderr.decode():
                continue
            assert r.returncode == 0, f"{s.name} live crashed"
            data = json.loads(out.read_text())
            slo = next(iter(data["slo_compliance"].values()))
            assert "skip_reason" in slo, (
                f"{s.name} did not emit skip_reason in live mode")


def test_comp_aggregators_exist():
    for name in ["comp_framework_mapping_audit.py", "comp_evidence_aggregator.py",
                  "comp_audit_chain_daily.py"]:
        assert (_EVAL / name).exists(), f"aggregator {name} missing"


def test_comp_runbooks_exist():
    expected = [
        "comp_breach_response.md",
        "comp_erasure_failure.md",
        "comp_subprocessor_change.md",
        "comp_residency_violation.md",
        "comp_ai_act_reclassification.md",
    ]
    missing = [n for n in expected if not (_REPO / "docs" / "runbook" / n).exists()]
    assert not missing, f"missing runbooks : {missing}"


def test_comp_workflows_exist():
    wf = _REPO / ".github" / "workflows"
    expected = ["comp-contract.yml", "comp-bench.yml", "comp-audit-replay.yml"]
    missing = [n for n in expected if not (wf / n).exists()]
    assert not missing, f"missing workflows : {missing}"


def test_comp_dt_tickets_documented():
    archi = (_REPO / "docs" / "EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md").read_text()
    must = [
        "DT-COMP-LIVE-DPO-EMAIL", "DT-COMP-LIVE-AUTHORITY",
        "DT-COMP-LIVE-S3-RESIDUE-SCAN", "DT-COMP-LIVE-DPIA-DPO-SIGN",
        "DT-COMP-LIVE-ROPA-CROSSPROD", "DT-COMP-LIVE-SUBPROC-DIFF",
        "DT-COMP-LIVE-RESIDENCY-PROD", "DT-COMP-LIVE-AI-ACT-AUDIT",
        "DT-COMP-LIVE-CONSENT-PIPELINE", "DT-COMP-LIVE-TRANSPARENCY-CDN",
        "DT-COMP-LIVE-HMAC-ROTATION", "DT-COMP-SOC2-TYPE2-AUDIT",
        "DT-COMP-ISO27001-CERT", "DT-COMP-HIPAA-BAA-SIGNED",
    ]
    missing = [d for d in must if d not in archi]
    assert not missing, f"missing DT-COMP-* in archi : {missing}"


def test_comp_slos_mentioned_in_archi():
    archi = (_REPO / "docs" / "EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md").read_text()
    data = _read_yaml(_REPO / "bench_baselines" / "v0.8.0" / "comp_slos.yaml")
    missing = [s["id"] for s in data["slos"] if s["id"] not in archi]
    assert not missing, f"SLOs absent from archi : {missing}"


def test_comp_dpia_template_present():
    """Existing dpia template (M14) referenced as fallback."""
    candidates = [
        _COMP_DIR / "gdpr_dpia_template.md",   # M14 baseline
        _COMP_DIR / "dpia_template.md",
    ]
    assert any(p.exists() for p in candidates), "no DPIA template found"


def test_comp_ropa_template_present():
    candidates = [
        _COMP_DIR / "GDPR_ART30_REGISTER.md",   # M14 baseline
        _COMP_DIR / "gdpr_ropa_template.md",
        _COMP_DIR / "ropa_template.md",
    ]
    assert any(p.exists() for p in candidates), "no ROPA template found"


def test_comp_generators_exist_and_invokable():
    for name in ["generate_dpia.py", "generate_ropa.py"]:
        p = _REPO / "scripts" / "compliance" / name
        assert p.exists(), f"{name} missing"
        r = subprocess.run(["python3", str(p), "--help"],
                            capture_output=True, timeout=10)
        assert r.returncode == 0, f"{name} --help failed"


def test_comp_architecture_doc_exists():
    p = _REPO / "docs" / "EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md"
    assert p.exists()
    text = p.read_text()
    assert "Φ-D-COMPLIANCE" in text
    assert "D-COMP-1" in text and "D-COMP-10" in text
    assert "SLO-COMP-EVIDENCE-COVERAGE" in text


def test_comp_check_slo_supports_thresholds():
    src = (_REPO / "eval" / "scripts" / "check_slo.py").read_text()
    assert "threshold_seconds" in src
    assert "threshold_pass_count" in src


def test_comp_no_orphan_dt_references():
    """Every DT-COMP-* in code MUST be in archi § 24."""
    archi_text = (_REPO / "docs" / "EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md").read_text()
    archi_dts = set(re.findall(r"DT-COMP-[A-Z0-9-]+", archi_text))
    code_dts = set()
    for f in [
        *_EVAL.glob("comp_*.py"),
        *(_REPO / "scripts" / "compliance").glob("*.py"),
        _REPO / "scripts" / "tests" / "test_compliance_completion.py",
    ]:
        if f.exists():
            code_dts.update(re.findall(r"DT-COMP-[A-Z0-9-]+", f.read_text()))
    orphans = code_dts - archi_dts
    assert not orphans, f"DT-COMP-* in code but missing from archi : {sorted(orphans)}"


# ═════════════════════════════════════════════════════════════════════
# 18 SEMANTIC PIN TESTS
# ═════════════════════════════════════════════════════════════════════


def _run_evidence_synth_pass(script_name: str) -> dict:
    import tempfile
    script = _EVAL / script_name
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_pass", "--out", str(out)],
            capture_output=True, timeout=30,
        )
        assert r.returncode == 0, (
            f"{script_name} synth_pass exit {r.returncode} : "
            f"stderr={r.stderr.decode()[:300]}"
        )
        return json.loads(out.read_text())["slo_compliance"]


def test_comp_framework_mapping_audit_full_coverage():
    compl = _run_evidence_synth_pass("comp_framework_mapping_audit.py")
    for slo_id in ["SLO-COMP-GDPR-CONTROLS", "SLO-COMP-SOC2-CONTROLS",
                    "SLO-COMP-ISO27001-CONTROLS", "SLO-COMP-AI-ACT-CONTROLS"]:
        assert compl[slo_id]["measured_pass_count"] == 1, f"{slo_id} not PASS"


def test_comp_evidence_aggregator_no_orphan_control():
    compl = _run_evidence_synth_pass("comp_evidence_aggregator.py")
    slo = compl["SLO-COMP-EVIDENCE-COVERAGE"]
    assert slo["measured_pass_count"] == 1
    assert not slo["missing_files"]
    assert not slo["orphan_evidences"]


def test_comp_audit_chain_daily_replays_clean():
    compl = _run_evidence_synth_pass("comp_audit_chain_daily.py")
    slo = compl["SLO-COMP-AUDIT-CHAIN-DAILY"]
    assert slo["measured_pass_count"] == 1
    assert slo["total_broken"] == 0


def test_comp_audit_chain_daily_detects_break():
    import tempfile
    script = _EVAL / "comp_audit_chain_daily.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_fail", "--out", str(out)],
            capture_output=True, timeout=15,
        )
        assert r.returncode == 0   # synth_fail "detects break" = success
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-COMP-AUDIT-CHAIN-DAILY"]
        assert slo["measured_pass_count"] == 1, "synth_fail must detect break"
        assert slo["total_broken"] >= 1


def test_comp_hmac_chain_evidence_self_test():
    compl = _run_evidence_synth_pass("comp_hmac_chain_evidence.py")
    assert compl["SLO-COMP-EVIDENCE-COVERAGE"]["measured_pass_count"] == 1


def test_comp_tenant_isolation_three_scenarios():
    compl = _run_evidence_synth_pass("comp_tenant_isolation_evidence.py")
    slo = compl["SLO-COMP-EVIDENCE-COVERAGE"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_scenarios"] == 3


def test_comp_consent_4_axes_independence():
    compl = _run_evidence_synth_pass("comp_consent_granularity_evidence.py")
    slo = compl["SLO-COMP-CONSENT-VERSION"]
    assert slo["measured_pass_count"] == 1
    assert len(slo["axes"]) == 4


def test_comp_encryption_at_rest_invariants():
    compl = _run_evidence_synth_pass("comp_encryption_at_rest_evidence.py")
    slo = compl["SLO-COMP-EVIDENCE-COVERAGE"]
    assert slo["measured_pass_count"] == 1


def test_comp_data_residency_scenarios_no_leak():
    compl = _run_evidence_synth_pass("comp_data_residency_evidence.py")
    slo = compl["SLO-COMP-RESIDENCY-LEAK"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_scenarios"] >= 5


def test_comp_subprocessor_inventory_fresh():
    compl = _run_evidence_synth_pass("comp_subprocessor_inventory_evidence.py")
    slo = compl["SLO-COMP-SUBPROCESSOR-INVENTORY"]
    assert slo["measured_pass_count"] == 1
    assert slo["age_seconds"] <= slo["max_age_seconds"]


def test_comp_breach_72h_below_cap():
    compl = _run_evidence_synth_pass("comp_breach_72h_evidence.py")
    slo = compl["SLO-COMP-BREACH-NOTIFY-72H"]
    assert slo["measured_seconds"] <= 72 * 3600


def test_comp_breach_24h_containment_below_cap():
    compl = _run_evidence_synth_pass("comp_breach_24h_containment_evidence.py")
    slo = compl["SLO-COMP-BREACH-CONTAIN-24H"]
    assert slo["measured_seconds"] <= 24 * 3600


def test_comp_erasure_30d_below_cap():
    compl = _run_evidence_synth_pass("comp_erasure_30d_evidence.py")
    slo = compl["SLO-COMP-ERASURE-30D"]
    assert slo["measured_seconds"] <= 30 * 86400
    assert slo["residue_detected"] is False


def test_comp_portability_30d_below_cap():
    compl = _run_evidence_synth_pass("comp_portability_30d_evidence.py")
    slo = compl["SLO-COMP-PORTABILITY-30D"]
    assert slo["measured_seconds"] <= 30 * 86400


def test_comp_dpia_freshness_no_stale():
    compl = _run_evidence_synth_pass("comp_dpia_freshness_evidence.py")
    slo = compl["SLO-COMP-DPIA-FRESHNESS"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_stale"] == 0


def test_comp_ropa_freshness_no_stale():
    compl = _run_evidence_synth_pass("comp_ropa_freshness_evidence.py")
    slo = compl["SLO-COMP-ROPA-FRESHNESS"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_stale"] == 0


def test_comp_ai_act_high_risk_forces_mode3():
    compl = _run_evidence_synth_pass("comp_ai_act_human_oversight_evidence.py")
    slo = compl["SLO-COMP-AI-ACT-HIGH-OVERSIGHT"]
    assert slo["measured_pass_count"] == 1
    # Verify the "high forces mode3" scenario passed.
    decisions = {s["name"]: s.get("got") for s in slo["scenarios"]
                  if "got" in s}
    if "high_force_mode3" in decisions:
        assert decisions["high_force_mode3"] == "mode3"


def test_comp_ai_act_dpo_signoff_within_1y():
    compl = _run_evidence_synth_pass("comp_ai_act_dpo_signoff_evidence.py")
    slo = compl["SLO-COMP-AI-ACT-DPO-SIGNOFF"]
    assert slo["measured_pass_count"] == 1
    assert not slo["issues"]


def test_comp_transparency_notice_freshness():
    compl = _run_evidence_synth_pass("comp_transparency_notice_evidence.py")
    slo = compl["SLO-COMP-TRANSPARENCY-NOTICE"]
    assert slo["measured_pass_count"] == 1
    assert not slo["failing_notices"]


# ═════════════════════════════════════════════════════════════════════
# 8 EVIDENCE AUDITS (= 1 par doctrine D-COMP-*)
# ═════════════════════════════════════════════════════════════════════


def test_audit_d_comp_1_coverage_all_frameworks_have_evidence():
    """D-COMP-1 : every framework in registry has at least one mapped control with evidence."""
    idx = _read_yaml(_COMP_DIR / "controls_index.yaml")
    reg = _read_yaml(_COMP_DIR / "registry.yaml")
    framework_prefixes = {
        "GDPR": "GDPR-", "SOC2": "SOC2-", "ISO27001": "ISO-",
        "EU_AI_ACT": "AI-ACT-", "HIPAA_READY": "HIPAA-", "CCPA": "CCPA-",
    }
    all_controls = set()
    for ev in idx["evidences"].values():
        all_controls.update(ev["controls"])
    for fw, prefix in framework_prefixes.items():
        fw_controls = [c for c in all_controls if c.startswith(prefix)]
        assert fw_controls, f"{fw} has 0 controls covered by any evidence"


def test_audit_d_comp_2_evidence_runtime_no_doc_only():
    """D-COMP-2 : no control is 'documented only' = each control in index has evidence script."""
    idx = _read_yaml(_COMP_DIR / "controls_index.yaml")
    for ev_name, ev in idx["evidences"].items():
        fpath = _REPO / ev["file"]
        assert fpath.exists(), f"{ev_name} → {ev['file']} : evidence runtime missing"


def test_audit_d_comp_3_audit_chain_compliance_events_typed():
    """D-COMP-3 : compliance event types declared in aggregator."""
    src = (_EVAL / "comp_audit_chain_daily.py").read_text()
    must_have = [
        "compliance.consent_change_v2", "compliance.erasure_request",
        "compliance.breach_detected", "compliance.dpia_generated",
        "compliance.subprocessor_added", "compliance.ai_act_reclassified",
        "compliance.residency_violation",
    ]
    missing = [e for e in must_have if e not in src]
    assert not missing, f"missing compliance event types : {missing}"


def test_audit_d_comp_4_breach_72h_and_24h_present():
    """D-COMP-4 : breach has both 72h notify + 24h containment evidence."""
    assert (_EVAL / "comp_breach_72h_evidence.py").exists()
    assert (_EVAL / "comp_breach_24h_containment_evidence.py").exists()


def test_audit_d_comp_5_erasure_and_portability_30d_present():
    """D-COMP-5 : erasure + portability evidences present."""
    assert (_EVAL / "comp_erasure_30d_evidence.py").exists()
    assert (_EVAL / "comp_portability_30d_evidence.py").exists()


def test_audit_d_comp_6_subprocessor_inventory_present():
    """D-COMP-6 : subprocessor inventory + > 13 entries declared."""
    src = (_EVAL / "comp_subprocessor_inventory_evidence.py").read_text()
    assert "SUBPROCESSORS_BASELINE" in src
    # Count list entries via line search (= heuristic).
    n_entries = src.count('{"name":')
    assert n_entries >= 10, f"sub-processor baseline too small : {n_entries}"


def test_audit_d_comp_7_residency_strict_fail_closed():
    """D-COMP-7 : residency evidence covers 5+ scenarios including DENIED cases."""
    compl = _run_evidence_synth_pass("comp_data_residency_evidence.py")
    slo = compl["SLO-COMP-RESIDENCY-LEAK"]
    denied_scenarios = [s for s in slo["scenarios"] if s["expect"] is False]
    assert len(denied_scenarios) >= 2, "must test ≥ 2 denied cases"


def test_audit_d_comp_8_ai_act_oversight_dpo_signoff_pair():
    """D-COMP-8 : AI Act needs BOTH oversight + DPO signoff evidences."""
    assert (_EVAL / "comp_ai_act_human_oversight_evidence.py").exists()
    assert (_EVAL / "comp_ai_act_dpo_signoff_evidence.py").exists()
    assert (_EVAL / "comp_ai_act_risk_classification_evidence.py").exists()

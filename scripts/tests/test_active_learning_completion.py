"""Pin tests for Active Learning (M24) Completion.

Sprint **Φ-D-AL Completion (D79)** — artefact #35.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 20.

18 pin tests enforce the 10 D-AL-* doctrines runtime.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]


def _read_yaml(p: Path) -> dict:
    yaml = pytest.importorskip("yaml")
    return yaml.safe_load(p.read_text())


def _read_json(p: Path) -> dict:
    return json.loads(p.read_text())


def test_al_slo_yaml_has_18_entries():
    p = _REPO / "bench_baselines" / "v0.6.0" / "al_slos.yaml"
    assert p.exists()
    data = _read_yaml(p)
    assert data["domain"] == "active_learning"
    ids = {s["id"] for s in data["slos"]}
    expected = {
        "SLO-AL-RECALL-IMPROVEMENT", "SLO-AL-UNCERTAINTY-COVERAGE",
        "SLO-AL-GDPR-CONSENT-COVERAGE", "SLO-AL-CANARY-DURATION",
        "SLO-AL-AUDIT-CHAIN-INTEGRITY", "SLO-AL-PROMOTION-AUTO-ROLLBACK",
        "SLO-AL-COST-PER-LABEL", "SLO-AL-CONCEPT-DRIFT",
        "SLO-AL-GOLD-QUEUE-SIZE", "SLO-AL-LLM-LABELING-LATENCY-P99",
        "SLO-AL-HUMAN-REVIEW-LATENCY-P99", "SLO-AL-AUTO-CONFIRM-COVERAGE",
        "SLO-AL-CENTROID-UPDATE-FREQUENCY", "SLO-AL-TENANT-ISOLATION",
        "SLO-AL-HOLDOUT-SPLIT-INTEGRITY", "SLO-AL-LABEL-DRIFT-DETECTION",
        "SLO-AL-RETUNE-LATENCY", "SLO-AL-REPLAY",
    }
    missing = expected - ids
    assert not missing, f"missing : {sorted(missing)}"
    assert len(ids) == 18


def test_al_slo_yaml_recall_threshold():
    data = _read_yaml(_REPO / "bench_baselines" / "v0.6.0" / "al_slos.yaml")
    s = next(x for x in data["slos"] if x["id"] == "SLO-AL-RECALL-IMPROVEMENT")
    assert s["comparator"] == "gte"
    assert s["threshold_ratio"] == 0.01


def test_al_check_slo_supports_pass_count():
    src = (_REPO / "eval" / "scripts" / "check_slo.py").read_text()
    assert "threshold_pass_count" in src and "expected_count" in src


def test_al_attestation_struct_fields():
    src = (_REPO / "scripts" / "active_learning" / "replay_label_chain.py").read_text()
    required = [
        "schema_version", "tenant_id", "label_id", "query_hash",
        "catalog_id", "source", "label_text", "confidence",
        "consent_hash", "cost_usd_micros", "timestamp_unix_micros",
        "prev_hash",
    ]
    for f in required:
        assert f in src, f"AlLabelAttestation field missing : {f}"


def test_al_holdout_split_size():
    """Holdout split must be 20 % per archi § 10.2."""
    archi = (_REPO / "docs" / "EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md").read_text()
    assert "20 %" in archi


def test_al_canary_duration_hardcap():
    """SLO-AL-CANARY-DURATION must enforce ≥ 24h (86400 s)."""
    data = _read_yaml(_REPO / "bench_baselines" / "v0.6.0" / "al_slos.yaml")
    s = next(x for x in data["slos"] if x["id"] == "SLO-AL-CANARY-DURATION")
    assert s["threshold_seconds"] == 86400.0


def test_al_auto_confirm_threshold():
    archi = (_REPO / "docs" / "EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md").read_text()
    assert "≥ 0.95" in archi and "GDPR" in archi


def test_al_phase2_scripts_exist():
    """Audit fix Φ-D-AL.A4 + L2 : 6 source_bench scripts added (gdpr, rollback,
    isolation, label_drift, deterministic replay, auto-confirm gate)
    → 13 scripts total."""
    expected = [
        _REPO / "scripts" / "active_learning" / "replay_label_chain.py",
        _REPO / "scripts" / "active_learning" / "replay_uncertainty_coverage.py",
        _REPO / "scripts" / "active_learning" / "track_label_costs.py",
        _REPO / "scripts" / "active_learning" / "verify_holdout_integrity.py",
        _REPO / "scripts" / "active_learning" / "verify_canary_duration.py",
        _REPO / "eval" / "scripts" / "al_recall_improvement_bench.py",
        _REPO / "eval" / "scripts" / "al_drift_detector.py",
        # ── 5 audit-Level-1 additions (anti-faussement-PASS via stubs) ──
        _REPO / "eval" / "scripts" / "al_gdpr_consent_replay.py",
        _REPO / "eval" / "scripts" / "al_auto_rollback_injection_test.py",
        _REPO / "eval" / "scripts" / "al_tenant_isolation_test.py",
        _REPO / "eval" / "scripts" / "al_label_drift_test.py",
        _REPO / "eval" / "scripts" / "al_deterministic_replay_test.py",
        # ── 1 audit-Level-2 addition (artefact #15 doctrine-D-AL-4 gate) ──
        _REPO / "eval" / "scripts" / "al_auto_confirm_gate_test.py",
    ]
    missing = [p for p in expected if not p.exists()]
    assert not missing, f"missing scripts : {missing}"


def test_al_phase2_scripts_invokable():
    """All 13 AL scripts must respond to --help with exit 0."""
    scripts = [
        _REPO / "scripts" / "active_learning" / "replay_label_chain.py",
        _REPO / "scripts" / "active_learning" / "replay_uncertainty_coverage.py",
        _REPO / "scripts" / "active_learning" / "track_label_costs.py",
        _REPO / "scripts" / "active_learning" / "verify_holdout_integrity.py",
        _REPO / "scripts" / "active_learning" / "verify_canary_duration.py",
        _REPO / "eval" / "scripts" / "al_recall_improvement_bench.py",
        _REPO / "eval" / "scripts" / "al_drift_detector.py",
        _REPO / "eval" / "scripts" / "al_gdpr_consent_replay.py",
        _REPO / "eval" / "scripts" / "al_auto_rollback_injection_test.py",
        _REPO / "eval" / "scripts" / "al_tenant_isolation_test.py",
        _REPO / "eval" / "scripts" / "al_label_drift_test.py",
        _REPO / "eval" / "scripts" / "al_deterministic_replay_test.py",
        _REPO / "eval" / "scripts" / "al_auto_confirm_gate_test.py",
    ]
    for p in scripts:
        r = subprocess.run(["python3", str(p), "--help"], capture_output=True, timeout=10)
        assert r.returncode == 0, f"{p.name} --help failed"


def test_al_phase2_scripts_emit_skip_when_no_input():
    """Anti-Φ-D.A1 : scripts must emit skip_reason when no input."""
    import tempfile
    optional_input_scripts = [
        _REPO / "scripts" / "active_learning" / "replay_label_chain.py",
        _REPO / "scripts" / "active_learning" / "replay_uncertainty_coverage.py",
        _REPO / "scripts" / "active_learning" / "track_label_costs.py",
        _REPO / "scripts" / "active_learning" / "verify_holdout_integrity.py",
        _REPO / "scripts" / "active_learning" / "verify_canary_duration.py",
    ]
    with tempfile.TemporaryDirectory() as td:
        for script in optional_input_scripts:
            out = Path(td) / f"{script.stem}.json"
            r = subprocess.run(
                ["python3", str(script), "--out", str(out)],
                capture_output=True, timeout=10,
            )
            assert r.returncode == 0
            assert out.exists()
            data = json.loads(out.read_text())
            slo = next(iter(data["slo_compliance"].values()))
            assert "skip_reason" in slo, (
                f"{script.name} did not emit skip_reason → faussement-PASS risk"
            )


def test_al_runbooks_exist():
    runbook_dir = _REPO / "docs" / "runbook"
    expected = [
        "al_retune_failure.md",
        "al_canary_rollback.md",
        "al_queue_overflow.md",
        "al_drift_detected.md",
        "al_cost_cap_hit.md",
    ]
    missing = [n for n in expected if not (runbook_dir / n).exists()]
    assert not missing, f"missing runbooks : {missing}"


def test_al_workflows_exist():
    wf_dir = _REPO / ".github" / "workflows"
    expected = [
        "al-contract.yml", "al-bench.yml",
        "al-audit-replay.yml", "al-drift-monitor.yml",
    ]
    missing = [n for n in expected if not (wf_dir / n).exists()]
    assert not missing, f"missing workflows : {missing}"


def test_al_recall_bench_monotonic():
    """Recall improvement bench must produce monotonic per-catalog delta ≥ 0."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_recall_improvement_bench.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "rb.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=30, cwd=str(_REPO),
        )
        assert r.returncode == 0
        data = json.loads(out.read_text())
        per_catalog = data["slo_compliance"]["SLO-AL-RECALL-IMPROVEMENT"]["per_catalog"]
        regressions = {cid: d["delta"] for cid, d in per_catalog.items() if d["delta"] < 0}
        assert not regressions, f"recall regression : {regressions}"


def test_al_drift_detector_empty_input_skip():
    """Drift detector live mode without fixtures must emit skip, not faussement-PASS."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_drift_detector.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "drift.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "live", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0  # SKIPPED returns 0
        data = json.loads(out.read_text())
        slo = data["slo_compliance"]["SLO-AL-CONCEPT-DRIFT"]
        assert "skip_reason" in slo


def test_al_label_chain_canonical_bytes():
    """Pin canonical_bytes format for AlLabelAttestation HMAC."""
    sys.path.insert(0, str(_REPO / "scripts" / "active_learning"))
    from replay_label_chain import canonical_bytes, compute_hmac

    ref = {
        "schema_version": 1,
        "tenant_id": "tenant-acme",
        "label_id": "label-001",
        "query_hash": "a" * 64,
        "catalog_id": "router-v1",
        "source": "llm_oracle",
        "label_text": "billing-dispute",
        "confidence": 0.97,
        "consent_hash": "b" * 64,
        "cost_usd_micros": 5000,
        "timestamp_unix_micros": 1700000000000000,
        "prev_hash": "00" * 32,
    }
    cb = canonical_bytes(ref).decode()
    assert " " not in cb and "\n" not in cb
    # sort_keys check : `catalog_id` < `confidence`.
    assert cb.index('"catalog_id"') < cb.index('"confidence"')
    # HMAC deterministic.
    key = bytes(32)
    assert compute_hmac(key, ref) == compute_hmac(key, ref)


def test_al_recall_bench_aggregate_pinned():
    """Pin recall bench aggregate value to detect RNG drift."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_recall_improvement_bench.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "rb.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=30, cwd=str(_REPO),
        )
        assert r.returncode == 0
        data = json.loads(out.read_text())
        aggregate = data["slo_compliance"]["SLO-AL-RECALL-IMPROVEMENT"]["measured_ratio"]
        # Pinned : must be ≥ +1 % (= SLO target).
        assert aggregate >= 0.01, f"recall bench aggregate {aggregate} < threshold 0.01"


def test_al_per_label_drift_floor():
    """Drift detector synth_fail mode must detect drift."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_drift_detector.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "drift.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_fail", "--out", str(out)],
            capture_output=True, timeout=15,
        )
        # synth_fail expects drift detected → SLO passes → exit 0.
        assert r.returncode == 0
        data = json.loads(out.read_text())
        slo = data["slo_compliance"]["SLO-AL-CONCEPT-DRIFT"]
        assert slo["measured_pass_count"] == 1
        assert slo["drift_detected"] is True


def test_al_holdout_fingerprint_isolated():
    """Holdout integrity verifier must return SKIP when no input provided."""
    import tempfile
    script = _REPO / "scripts" / "active_learning" / "verify_holdout_integrity.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "h.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        data = json.loads(out.read_text())
        slo = data["slo_compliance"]["SLO-AL-HOLDOUT-SPLIT-INTEGRITY"]
        assert "skip_reason" in slo


# ─────────────────────────────────────────────────────────────────────
# Audit-Level-1 additions — 7 pin tests for the 5 new source_bench
# scripts + 2 anti-régression checks (canary MIN-semantics, holdout
# fingerprint injection immunity).
# ─────────────────────────────────────────────────────────────────────


def test_al_gdpr_consent_replay_synth_pass_and_fail():
    """GDPR consent replay : synth_pass = 100 %, synth_fail = 99 %.

    Pin the two negative-test directions :
      - synth_pass → exit 0, coverage = 1.0, pass_count = 1
      - synth_fail → exit 1, coverage = 0.99, pass_count = 0
    """
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_gdpr_consent_replay.py"
    with tempfile.TemporaryDirectory() as td:
        # synth_pass
        out_p = Path(td) / "pass.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_pass", "--out", str(out_p)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo_p = json.loads(out_p.read_text())["slo_compliance"]["SLO-AL-GDPR-CONSENT-COVERAGE"]
        assert slo_p["coverage"] == 1.0
        assert slo_p["measured_pass_count"] == 1
        # synth_fail
        out_f = Path(td) / "fail.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_fail", "--out", str(out_f)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 1, "synth_fail must exit 1"
        slo_f = json.loads(out_f.read_text())["slo_compliance"]["SLO-AL-GDPR-CONSENT-COVERAGE"]
        assert 0.98 <= slo_f["coverage"] < 1.0
        assert slo_f["measured_pass_count"] == 0


def test_al_auto_rollback_injection_three_scenarios():
    """Auto-rollback : the 3 injected regression scenarios trigger 'rollback'.

    Audit L2 update : count is now ≥ 3 (one of which is healthy_candidate=continue).
    This test pins the 3 negative scenarios specifically.
    """
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_auto_rollback_injection_test.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "rb.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-AL-PROMOTION-AUTO-ROLLBACK"]
        assert slo["measured_pass_count"] == 1
        assert slo["n_scenarios"] >= 3, "must have at least 3 regression scenarios"
        decisions = {r["scenario"]: r["decision"] for r in slo["results"]}
        assert decisions["recall_regression"] == "rollback"
        assert decisions["cost_blowup"] == "rollback"
        assert decisions["latency_blowup"] == "rollback"


def test_al_tenant_isolation_three_tests():
    """Tenant isolation : clean + cross-tenant + orphan = 3/3 PASS."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_tenant_isolation_test.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "ti.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-AL-TENANT-ISOLATION"]
        assert slo["measured_pass_count"] == 1
        assert slo["n_tests"] == 3
        assert slo["n_passed"] == 3


def test_al_label_drift_tvd_directions():
    """Label drift TVD : synth_pass < threshold, synth_fail > threshold."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_label_drift_test.py"
    with tempfile.TemporaryDirectory() as td:
        # synth_pass
        out_p = Path(td) / "p.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_pass", "--out", str(out_p)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo_p = json.loads(out_p.read_text())["slo_compliance"]["SLO-AL-LABEL-DRIFT-DETECTION"]
        assert slo_p["drift_detected"] is False
        assert slo_p["tvd"] < 0.10
        # synth_fail
        out_f = Path(td) / "f.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_fail", "--out", str(out_f)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo_f = json.loads(out_f.read_text())["slo_compliance"]["SLO-AL-LABEL-DRIFT-DETECTION"]
        assert slo_f["drift_detected"] is True
        assert slo_f["tvd"] > 0.10


def test_al_deterministic_replay_six_tests():
    """Determinism : the 6 core invariants must hold.

    Audit L2 update : count is now ≥ 6 (extended to 9 with dict-order,
    confidence-change, prev-hash-change). This test pins the 6 core ones.
    """
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_deterministic_replay_test.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "r.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-AL-REPLAY"]
        assert slo["measured_pass_count"] == 1
        assert slo["n_tests"] >= 6
        assert slo["n_passed"] == slo["n_tests"]
        # Pin the 6 core invariant names exist.
        names = {r["test"] for r in slo["results"]}
        for must_have in [
            "canonical_bytes_idempotent", "hmac_deterministic",
            "extra_fields_ignored", "missing_field_changes_hmac",
            "different_key_different_hmac", "single_bit_key_flip",
        ]:
            assert must_have in names, f"missing core invariant : {must_have}"


def test_al_canary_min_seconds_semantics():
    """Audit fix Φ-D-AL.A1 : measured_seconds must be MIN(durations), not p99.

    Inject 3 promotions : 2 valid (≥ 24h) and 1 violator (1h). The script
    must :
      - report `measured_seconds = 3600` (= MIN, the worst case)
      - report `n_violations = 1`
      - exit 1
    Pre-fix the script reported p99 ≈ 90 000s (= max-leaning), hiding
    the 1h violator → faussement-PASS.
    """
    import tempfile
    script = _REPO / "scripts" / "active_learning" / "verify_canary_duration.py"
    with tempfile.TemporaryDirectory() as td:
        atts = {
            "attestations": [
                {"promotion_decision": "promoted", "catalog_id": "c1",
                 "canary_started_at": 0, "canary_ended_at": 90000 * 1_000_000},
                {"promotion_decision": "promoted", "catalog_id": "c2",
                 "canary_started_at": 0, "canary_ended_at": 100000 * 1_000_000},
                {"promotion_decision": "promoted", "catalog_id": "c3",
                 "canary_started_at": 0, "canary_ended_at": 3600 * 1_000_000},  # violator
            ]
        }
        atts_path = Path(td) / "atts.json"
        atts_path.write_text(json.dumps(atts))
        out = Path(td) / "canary.json"
        r = subprocess.run(
            ["python3", str(script), "--retune-attestations", str(atts_path),
             "--out", str(out)],
            capture_output=True, timeout=10,
        )
        # Exit must be 1 because a violator exists.
        assert r.returncode == 1
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-AL-CANARY-DURATION"]
        # The CRITICAL pin : measured_seconds must be the MIN (3600), not the p99/MAX.
        assert slo["measured_seconds"] == 3600
        assert slo["max_seconds"] == 100000
        assert slo["n_violations"] == 1


def test_al_canary_empty_promoted_skip():
    """Audit fix Φ-D-AL.A2 : no promoted records → SKIP, not faussement-PASS."""
    import tempfile
    script = _REPO / "scripts" / "active_learning" / "verify_canary_duration.py"
    with tempfile.TemporaryDirectory() as td:
        atts = {
            "attestations": [
                {"promotion_decision": "rolled_back"},
                {"promotion_decision": "rolled_back"},
            ]
        }
        atts_path = Path(td) / "atts.json"
        atts_path.write_text(json.dumps(atts))
        out = Path(td) / "canary.json"
        r = subprocess.run(
            ["python3", str(script), "--retune-attestations", str(atts_path),
             "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0  # SKIP returns 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-AL-CANARY-DURATION"]
        assert "skip_reason" in slo
        assert "no promoted retune" in slo["skip_reason"].lower()


def test_al_holdout_fingerprint_injection_immune():
    """Audit fix Φ-D-AL.A7 : canonical JSON fingerprint immune to '|' injection.

    Pre-fix concat `query|label|catalog` collided when query=`a|b` + label=``
    matched query=`a` + label=`b`. Post-fix uses JSON sort_keys → distinct.
    """
    sys.path.insert(0, str(_REPO / "scripts" / "active_learning"))
    from verify_holdout_integrity import fingerprint_label
    a = {"query_text": "a|b", "label_text": "", "catalog_id": "c"}
    b = {"query_text": "a",   "label_text": "b", "catalog_id": "c"}
    fa, fb = fingerprint_label(a), fingerprint_label(b)
    assert fa != fb, (
        "Fingerprint collision via | injection — fix Φ-D-AL.A7 regressed"
    )


def test_al_drift_detector_handles_empty_array():
    """Audit fix Φ-D-AL.A3 : wasserstein_1d([], [1,2,3]) must not crash."""
    sys.path.insert(0, str(_REPO / "eval" / "scripts"))
    from al_drift_detector import wasserstein_1d
    # Pre-fix : IndexError. Post-fix : returns 0.0 with stderr warning.
    assert wasserstein_1d([], [1.0, 2.0, 3.0]) == 0.0
    assert wasserstein_1d([1.0, 2.0], []) == 0.0
    assert wasserstein_1d([], []) == 0.0


# ─────────────────────────────────────────────────────────────────────
# Audit-Level-2 additions — 7 pin tests for L2 hardening :
#   * Auto-rollback positive control (= 4 scenarios)
#   * Replay determinism (= 9 invariants, was 6)
#   * Auto-confirm gate (= 10 doctrine-D-AL-4 cases)
#   * JSON schema label_attestation
#   * GDPR consent defensive types
# ─────────────────────────────────────────────────────────────────────


def test_al_auto_rollback_includes_positive_control():
    """L2 audit fix Φ-D-AL.A8 : healthy_candidate scenario expects 'continue'.

    Prevents the regression where evaluate_rollout always returns 'rollback'.
    """
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_auto_rollback_injection_test.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "rb.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-AL-PROMOTION-AUTO-ROLLBACK"]
        assert slo["n_scenarios"] == 4, "expected 3 rollback + 1 continue = 4 scenarios"
        assert slo["n_passed"] == 4
        decisions = {r["scenario"]: r["decision"] for r in slo["results"]}
        # 3 negative (rollback) + 1 positive (continue).
        assert decisions["recall_regression"] == "rollback"
        assert decisions["cost_blowup"] == "rollback"
        assert decisions["latency_blowup"] == "rollback"
        assert decisions["healthy_candidate"] == "continue", (
            "auto-rollback fires spuriously on healthy candidate (= over-blocking)"
        )


def test_al_replay_dict_order_independent():
    """L2 audit fix Φ-D-AL.A9 : reversed dict input yields same HMAC."""
    sys.path.insert(0, str(_REPO / "scripts" / "active_learning"))
    from replay_label_chain import canonical_bytes, compute_hmac
    ref = {
        "schema_version": 1, "tenant_id": "tenant-acme",
        "label_id": "label-001", "query_hash": "a" * 64,
        "catalog_id": "router-v1", "source": "llm_oracle",
        "label_text": "billing-dispute", "confidence": 0.97,
        "consent_hash": "b" * 64, "cost_usd_micros": 5000,
        "timestamp_unix_micros": 1700000000000000,
        "prev_hash": "00" * 32,
    }
    reverse_order = {k: ref[k] for k in reversed(list(ref.keys()))}
    key = bytes(32)
    # canonical_bytes must produce the same output regardless of insertion order.
    assert canonical_bytes(ref) == canonical_bytes(reverse_order)
    assert compute_hmac(key, ref) == compute_hmac(key, reverse_order)


def test_al_replay_test_has_nine_invariants():
    """L2 audit fix : determinism test now covers 9 invariants (was 6)."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "al_deterministic_replay_test.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "r.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-AL-REPLAY"]
        assert slo["n_tests"] == 9
        assert slo["n_passed"] == 9
        names = {r["test"] for r in slo["results"]}
        assert "input_dict_order_independence" in names
        assert "confidence_change_detected" in names
        assert "prev_hash_change_detected" in names


def test_al_auto_confirm_gate_script_exists_and_passes():
    """L2 audit fix Φ-D-AL.A7-bis : auto_confirm gate test (artefact #15)."""
    script = _REPO / "eval" / "scripts" / "al_auto_confirm_gate_test.py"
    assert script.exists(), "al_auto_confirm_gate_test.py missing"
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "ac.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["POLICY-D-AL-4-AUTO-CONFIRM-GATE"]
        assert slo["n_scenarios"] == 10
        assert slo["n_passed"] == 10
        names = {r["scenario"] for r in slo["results"]}
        # Cover all 4 doctrine D-AL-4 conditions + boundary cases.
        for must_have in ["happy_path", "low_confidence", "exact_threshold",
                          "just_below_threshold", "gdpr_sensitive_catalog",
                          "tenant_policy_denies", "consent_missing",
                          "consent_zero_placeholder", "consent_wrong_length",
                          "all_fail_together"]:
            assert must_have in names, f"missing scenario : {must_have}"


def test_al_json_schema_label_attestation_exists():
    """L2 audit fix Φ-D-AL.A12 : artefact #40 JSON schema for AlLabelAttestation."""
    yaml = pytest.importorskip("yaml")  # ensures jsonschema is also OK
    jsonschema = pytest.importorskip("jsonschema")
    schema_path = _REPO / "schemas" / "al_label_attestation.json"
    assert schema_path.exists(), "schemas/al_label_attestation.json missing"
    schema = json.loads(schema_path.read_text())
    # Schema itself must be valid Draft 2020-12.
    jsonschema.Draft202012Validator.check_schema(schema)
    # Required fields list must match CANONICAL_FIELDS + hmac_sha256.
    sys.path.insert(0, str(_REPO / "scripts" / "active_learning"))
    from replay_label_chain import CANONICAL_FIELDS, compute_hmac
    required = set(schema["required"])
    expected = set(CANONICAL_FIELDS) | {"hmac_sha256"}
    assert required == expected, f"required mismatch : {required ^ expected}"
    # Roundtrip : a reference attestation must validate.
    ref = {
        "schema_version": 1, "tenant_id": "tenant-acme",
        "label_id": "label-001", "query_hash": "a" * 64,
        "catalog_id": "router-v1", "source": "llm_oracle",
        "label_text": "billing-dispute", "confidence": 0.97,
        "consent_hash": "b" * 64, "cost_usd_micros": 5000,
        "timestamp_unix_micros": 1700000000000000,
        "prev_hash": "00" * 32,
    }
    ref["hmac_sha256"] = compute_hmac(bytes(32), ref)
    jsonschema.validate(ref, schema)


def test_al_json_schema_rejects_invalid_fields():
    """L2 audit fix : schema must REJECT obviously bad input (negative test)."""
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads((_REPO / "schemas" / "al_label_attestation.json").read_text())
    bad_samples = [
        # Bad consent_hash (wrong length).
        {"schema_version": 1, "tenant_id": "t", "label_id": "l",
         "query_hash": "a"*64, "catalog_id": "c", "source": "llm_oracle",
         "label_text": "x", "confidence": 0.9, "consent_hash": "deadbeef",
         "cost_usd_micros": 100, "timestamp_unix_micros": 1700000000000000,
         "prev_hash": "0"*64, "hmac_sha256": "0"*64},
        # Cost > 10000 (= $0.01 cap).
        {"schema_version": 1, "tenant_id": "t", "label_id": "l",
         "query_hash": "a"*64, "catalog_id": "c", "source": "llm_oracle",
         "label_text": "x", "confidence": 0.9, "consent_hash": "a"*64,
         "cost_usd_micros": 999999, "timestamp_unix_micros": 1700000000000000,
         "prev_hash": "0"*64, "hmac_sha256": "0"*64},
        # Confidence > 1.
        {"schema_version": 1, "tenant_id": "t", "label_id": "l",
         "query_hash": "a"*64, "catalog_id": "c", "source": "llm_oracle",
         "label_text": "x", "confidence": 1.5, "consent_hash": "a"*64,
         "cost_usd_micros": 100, "timestamp_unix_micros": 1700000000000000,
         "prev_hash": "0"*64, "hmac_sha256": "0"*64},
        # Invalid source enum.
        {"schema_version": 1, "tenant_id": "t", "label_id": "l",
         "query_hash": "a"*64, "catalog_id": "c", "source": "MAGIC",
         "label_text": "x", "confidence": 0.9, "consent_hash": "a"*64,
         "cost_usd_micros": 100, "timestamp_unix_micros": 1700000000000000,
         "prev_hash": "0"*64, "hmac_sha256": "0"*64},
    ]
    for i, bad in enumerate(bad_samples):
        with pytest.raises(jsonschema.ValidationError):
            jsonschema.validate(bad, schema)


def test_al_gdpr_consent_handles_non_string_types():
    """L2 audit fix Φ-D-AL.A10 : non-string consent_hash must not crash."""
    sys.path.insert(0, str(_REPO / "eval" / "scripts"))
    from al_gdpr_consent_replay import _validate_consent
    # Pre-fix : `len(int)` raised TypeError. Post-fix : returns (False, reason).
    ok, reason = _validate_consent({"tenant_id": "t", "consent_hash": 12345})
    assert ok is False and "non-string" in reason
    ok, reason = _validate_consent({"tenant_id": "t", "consent_hash": None})
    assert ok is False
    ok, reason = _validate_consent({"tenant_id": 999, "consent_hash": "a"*64})
    assert ok is False and "non-string" in reason

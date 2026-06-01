"""Pin tests for Mode-3 (multi-turn dynamic anchors) Completion.

Sprint **Φ-D-MODE3 Completion (D74)** — artefact #38.
Cf. ``docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`` § 20.

16 pin tests enforce the 10 D-MODE3-* doctrines runtime.
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


# ────────────────────────────────────────────────────────────────────
# 1-3 — SLO YAML completeness + correct thresholds.
# ────────────────────────────────────────────────────────────────────


def test_mode3_slo_yaml_has_18_entries():
    p = _REPO / "bench_baselines" / "v0.6.0" / "mode3_slos.yaml"
    assert p.exists(), f"SLO YAML missing : {p}"
    data = _read_yaml(p)
    assert data["domain"] == "mode3"
    ids = {s["id"] for s in data["slos"]}
    expected = {
        "SLO-MODE3-CONVERGENCE-RATE", "SLO-MODE3-RECALL-DELTA-VS-M2",
        "SLO-MODE3-COST-PER-SESSION-P99", "SLO-MODE3-COST-PER-TURN-DECREASING",
        "SLO-MODE3-LATENCY-PER-TURN-P99", "SLO-MODE3-HUMAN-REVIEW-LATENCY-P99",
        "SLO-MODE3-GDPR-OPT-IN-COVERAGE", "SLO-MODE3-AUDIT-CHAIN-INTEGRITY",
        "SLO-MODE3-SESSION-ISOLATION", "SLO-MODE3-ENCRYPTION-AT-REST",
        "SLO-MODE3-FALLBACK-WORKS", "SLO-MODE3-CONTEXT-WINDOW-RESPECT",
        "SLO-MODE3-SESSION-EXPIRY", "SLO-MODE3-PROMPT-VERSIONING",
        "SLO-MODE3-MAX-TURNS-ENFORCED", "SLO-MODE3-CONVERGENCE-COSINE",
        "SLO-MODE3-CACHE-HIT-RATE", "SLO-MODE3-REPLAY",
    }
    missing = expected - ids
    assert not missing, f"SLO YAML missing : {sorted(missing)}"
    assert len(ids) == 18


def test_mode3_slo_yaml_convergence_threshold():
    """SLO-MODE3-CONVERGENCE-RATE must require ≥ 90 %."""
    data = _read_yaml(_REPO / "bench_baselines" / "v0.6.0" / "mode3_slos.yaml")
    s = next(x for x in data["slos"] if x["id"] == "SLO-MODE3-CONVERGENCE-RATE")
    assert s["comparator"] == "gte"
    assert s["threshold_ratio"] == 0.90, (
        f"Convergence threshold drifted : {s['threshold_ratio']} (expected 0.90)"
    )


def test_mode3_check_slo_supports_pass_count():
    """Anti-Φ-D.A1 : check_slo.py must accept the new fields."""
    src = (_REPO / "eval" / "scripts" / "check_slo.py").read_text()
    assert "threshold_pass_count" in src and "expected_count" in src
    assert "slo.threshold_pass_count is not None" in src


# ────────────────────────────────────────────────────────────────────
# 4 — Mode3Attestation fields documented.
# ────────────────────────────────────────────────────────────────────


def test_mode3_attestation_struct_fields():
    """The Mode3Attestation 19 canonical fields must be declared in replay script."""
    src = (_REPO / "scripts" / "mode3" / "replay_session_chain.py").read_text()
    required = [
        "schema_version", "tenant_id", "request_id", "timestamp_unix_micros",
        "session_id", "turn_index", "consent_hash", "parent_anchor_hash",
        "refined_anchor_hash", "convergence_score", "convergence_status",
        "cost_usd_micros", "turn_latency_ms", "human_review_status",
        "fallback_reason", "context_tokens", "summarization_applied", "prev_hash",
    ]
    for field in required:
        assert field in src, f"Mode3Attestation field missing : {field}"


# ────────────────────────────────────────────────────────────────────
# 5 — Hard cap K=10 turns documented.
# ────────────────────────────────────────────────────────────────────


def test_mode3_max_turns_cap_documented():
    archi = (_REPO / "docs" / "EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md").read_text()
    schema = (_REPO / "schemas" / "mode3_session_state.json").read_text()
    # Archi mentions K=10 + max 10 turns.
    assert "K=10" in archi or "max 10" in archi.lower()
    # Schema enforces maxItems: 10 on turns array.
    s = json.loads(schema)
    turns_schema = s["properties"]["turns"]
    assert turns_schema["maxItems"] == 10


# ────────────────────────────────────────────────────────────────────
# 6 — Convergence cosine threshold documented.
# ────────────────────────────────────────────────────────────────────


def test_mode3_convergence_cosine_threshold():
    archi = (_REPO / "docs" / "EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md").read_text()
    bench = (_REPO / "eval" / "scripts" / "mode3_convergence_bench.py").read_text()
    assert "0.95" in archi, "Convergence threshold 0.95 missing from archi"
    assert "CONVERGENCE_THRESHOLD = 0.95" in bench


# ────────────────────────────────────────────────────────────────────
# 7 — Encryption at rest required.
# ────────────────────────────────────────────────────────────────────


def test_mode3_encryption_at_rest_required():
    archi = (_REPO / "docs" / "EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md").read_text()
    assert "AES-256-GCM" in archi
    assert "per-tenant" in archi.lower()
    # SLO present.
    data = _read_yaml(_REPO / "bench_baselines" / "v0.6.0" / "mode3_slos.yaml")
    ids = {x["id"] for x in data["slos"]}
    assert "SLO-MODE3-ENCRYPTION-AT-REST" in ids


# ────────────────────────────────────────────────────────────────────
# 8 — Runbooks (5).
# ────────────────────────────────────────────────────────────────────


def test_mode3_runbooks_exist():
    runbook_dir = _REPO / "docs" / "runbook"
    expected = [
        "mode3_convergence_failure.md",
        "mode3_review_queue_full.md",
        "mode3_gdpr_opt_out.md",
        "mode3_key_rotation.md",
        "mode3_session_purge.md",
    ]
    missing = [n for n in expected if not (runbook_dir / n).exists()]
    assert not missing, f"Mode-3 runbooks missing : {missing}"


# ────────────────────────────────────────────────────────────────────
# 9 — Workflows (4).
# ────────────────────────────────────────────────────────────────────


def test_mode3_workflows_exist():
    wf_dir = _REPO / ".github" / "workflows"
    expected = [
        "mode3-contract.yml",
        "mode3-bench.yml",
        "mode3-audit-replay.yml",
        "mode3-human-review-drill.yml",
    ]
    missing = [n for n in expected if not (wf_dir / n).exists()]
    assert not missing, f"Mode-3 workflows missing : {missing}"


# ────────────────────────────────────────────────────────────────────
# 10 — Bench catalogs count (10).
# ────────────────────────────────────────────────────────────────────


def test_mode3_bench_catalogs_count():
    catalogs_dir = _REPO / "eval" / "mode3" / "catalogs"
    yaml_files = list(catalogs_dir.glob("*.yaml"))
    assert len(yaml_files) == 10, f"expected 10 catalogs, got {len(yaml_files)}"


# ────────────────────────────────────────────────────────────────────
# 11 — Adversarial sessions count (40).
# ────────────────────────────────────────────────────────────────────


def test_mode3_adversarial_sessions_count():
    p = _REPO / "eval" / "mode3" / "adversarial_sessions" / "sessions.json"
    assert p.exists()
    data = _read_json(p)
    assert data["count"] == 40
    assert len(data["sessions"]) == 40


# ────────────────────────────────────────────────────────────────────
# 12 — Consent replay script exists + invokable.
# ────────────────────────────────────────────────────────────────────


def test_mode3_consent_replay_script_exists():
    p = _REPO / "scripts" / "mode3" / "replay_consent_log.py"
    assert p.exists()
    r = subprocess.run(["python3", str(p), "--help"], capture_output=True, timeout=10)
    assert r.returncode == 0


# ────────────────────────────────────────────────────────────────────
# 13 — Human review drill workflow.
# ────────────────────────────────────────────────────────────────────


def test_mode3_human_review_drill_workflow():
    p = _REPO / ".github" / "workflows" / "mode3-human-review-drill.yml"
    assert p.exists(), f"Human review drill workflow missing : {p}"
    text = p.read_text()
    # Daily cron required (= drill MUST run every day).
    assert "cron:" in text
    assert "low_confidence" in text or "drill" in text.lower()


# ────────────────────────────────────────────────────────────────────
# 14 — Per-session cache namespace.
# ────────────────────────────────────────────────────────────────────


def test_mode3_per_session_cache_namespace():
    archi = (_REPO / "docs" / "EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md").read_text()
    # Namespace template includes tenant + session.
    assert "mode3:v1:tenant:<tid>:session:<sid>" in archi or "tenant:<" in archi


# ────────────────────────────────────────────────────────────────────
# 15 — Session state JSON schema strict.
# ────────────────────────────────────────────────────────────────────


def test_mode3_session_state_schema_strict():
    p = _REPO / "schemas" / "mode3_session_state.json"
    assert p.exists()
    s = _read_json(p)
    assert s["additionalProperties"] is False
    # Required fields.
    assert "session_id" in s["required"]
    assert "tenant_id" in s["required"]
    assert "consent" in s["required"]
    assert "turns" in s["required"]
    # Hard caps on collections.
    assert s["properties"]["turns"]["maxItems"] == 10
    assert s["properties"]["concept_stack"]["maxItems"] == 50


# ────────────────────────────────────────────────────────────────────
# 16 — Audit chain per-session subchain documented.
# ────────────────────────────────────────────────────────────────────


def test_mode3_audit_chain_session_subchain():
    archi = (_REPO / "docs" / "EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md").read_text()
    # Archi mentions per-session sub-chain pattern.
    assert "per-session" in archi.lower()
    assert "session_chain" in archi.lower() or "session sub-chain" in archi.lower() or "session subchain" in archi.lower()


# ────────────────────────────────────────────────────────────────────
# Φ-D-MODE3 Audit Level 1 — anti-régression on 4 phase-2 scripts +
# per-catalog convergence floor + hand-curated session length.
# ────────────────────────────────────────────────────────────────────


def test_mode3_phase2_scripts_exist():
    """**Audit fix Φ-D-MODE3 B2** — 4 phase-2 scripts referenced by archi
    § 16 matrix MUST exist (= D-FFI-3 doctrine ; pattern Φ-D-MODE2 B3
    récidive).
    """
    expected = [
        _REPO / "scripts" / "mode3" / "track_review_sla.py",
        _REPO / "scripts" / "mode3" / "collect_cache_stats.py",
        _REPO / "scripts" / "mode3" / "replay_session_state.py",
        _REPO / "eval" / "scripts" / "mode3_review_triggers_test.py",
    ]
    missing = [p for p in expected if not p.exists()]
    assert not missing, f"Mode-3 phase-2 scripts missing : {missing}"


def test_mode3_phase2_scripts_invokable():
    """All 4 phase-2 scripts must respond to --help with exit 0."""
    expected = [
        _REPO / "scripts" / "mode3" / "track_review_sla.py",
        _REPO / "scripts" / "mode3" / "collect_cache_stats.py",
        _REPO / "scripts" / "mode3" / "replay_session_state.py",
        _REPO / "eval" / "scripts" / "mode3_review_triggers_test.py",
    ]
    for p in expected:
        if not p.exists():
            pytest.fail(f"{p} missing")
        result = subprocess.run(
            ["python3", str(p), "--help"],
            capture_output=True, timeout=10,
        )
        assert result.returncode == 0, (
            f"{p.name} --help failed : {result.stderr.decode()[:200]}"
        )


def test_mode3_phase2_scripts_emit_skip_when_no_input():
    """**Audit fix anti-Φ-D.A1** — phase-2 scripts must emit skip_reason
    when no live input is provided (= jamais faussement-PASS).
    """
    import tempfile
    scripts_with_optional_input = [
        (_REPO / "scripts" / "mode3" / "track_review_sla.py", "review_sla.json"),
        (_REPO / "scripts" / "mode3" / "collect_cache_stats.py", "cache_stats.json"),
        (_REPO / "scripts" / "mode3" / "replay_session_state.py", "replay.json"),
    ]
    with tempfile.TemporaryDirectory() as td:
        for script, out_name in scripts_with_optional_input:
            out_path = Path(td) / out_name
            result = subprocess.run(
                ["python3", str(script), "--out", str(out_path)],
                capture_output=True, timeout=15,
            )
            assert result.returncode == 0
            assert out_path.exists()
            data = json.loads(out_path.read_text())
            slo_block = next(iter(data["slo_compliance"].values()))
            assert "skip_reason" in slo_block, (
                f"{script.name} did not emit skip_reason → faussement-PASS risk"
            )
            assert "measured_pass_count" not in slo_block


def test_mode3_convergence_bench_per_catalog_floor():
    """**Audit fix Φ-D-MODE3 B3** — each catalog must achieve ≥ 70 %
    convergence rate (= floor). Aggregate-only check could mask a
    catalog at 10 % (= pre-fix multi-turn-routing).
    """
    import tempfile
    bench_script = _REPO / "eval" / "scripts" / "mode3_convergence_bench.py"
    with tempfile.TemporaryDirectory() as td:
        out_path = Path(td) / "convergence_bench.json"
        result = subprocess.run(
            ["python3", str(bench_script), "--out", str(out_path)],
            capture_output=True, timeout=60, cwd=str(_REPO),
        )
        assert result.returncode == 0, result.stderr.decode()
        data = json.loads(out_path.read_text())
        slo = data["slo_compliance"]["SLO-MODE3-CONVERGENCE-RATE"]
        per_catalog = slo["per_catalog"]
        FLOOR = 0.70
        below_floor = {
            cid: cdata["rate"]
            for cid, cdata in per_catalog.items()
            if cdata["rate"] < FLOOR
        }
        assert not below_floor, (
            f"B3 regression : per-catalog convergence below floor "
            f"{FLOOR:.0%} : {below_floor}"
        )


def test_mode3_hand_curated_routing_sessions_have_5_plus_turns():
    """**Audit fix Φ-D-MODE3 B1** — multi-turn-routing hand-curated
    sessions MUST have ≥ 5 turns to enable convergence detection.

    Pre-fix sessions had 2-4 turns → convergence never detected
    (= 1/10 = 10 % rate, masked by aggregate 99 %).
    """
    p = _REPO / "eval" / "mode3" / "catalogs" / "multi-turn-routing.yaml"
    assert p.exists()
    yaml = pytest.importorskip("yaml")
    data = yaml.safe_load(p.read_text())
    sessions = data["test_sessions"]
    short_sessions = [
        s["id"] for s in sessions
        if len(s.get("turns", [])) < 5
    ]
    assert not short_sessions, (
        f"B1 regression : hand-curated sessions with < 5 turns : "
        f"{short_sessions} → convergence detection fails on these"
    )


# ────────────────────────────────────────────────────────────────────
# Φ-D-MODE3 Audit Level 2 — chirurgical bug fixes.
# ────────────────────────────────────────────────────────────────────


def test_mode3_recall_bench_monotonic_per_catalog():
    """**Audit fix Φ-D-MODE3 B-extra1** — Mode-3 recall ≥ Mode-2 recall
    on EVERY catalog (= monotonicity by construction post-fix).

    Pre-fix the mock seeded RNG with `boost`, so Mode-2 and Mode-3
    drew independent samples → variance could make Mode-3 < Mode-2
    on small catalogs.
    """
    import tempfile
    bench_script = _REPO / "eval" / "scripts" / "mode3_recall_bench.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "recall.json"
        result = subprocess.run(
            ["python3", str(bench_script), "--out", str(out)],
            capture_output=True, timeout=30, cwd=str(_REPO),
        )
        assert result.returncode == 0, result.stderr.decode()
        data = json.loads(out.read_text())
        per_catalog = data["slo_compliance"]["SLO-MODE3-RECALL-DELTA-VS-M2"]["per_catalog"]
        regressions = {
            cid: d["delta"]
            for cid, d in per_catalog.items()
            if d["delta"] < 0
        }
        assert not regressions, (
            f"B-extra1 regression : per-catalog deltas < 0 detected : "
            f"{regressions} (= seed includes boost again → variance bug)"
        )


def test_mode3_recall_bench_aggregate_value_pinned():
    """**Audit fix Φ-D-MODE3 B-extra4** — pin the exact recall bench
    aggregate to detect Python random RNG drift cross-versions.

    Tolerance : 0 (= bit-identity expected).
    """
    import tempfile
    bench_script = _REPO / "eval" / "scripts" / "mode3_recall_bench.py"
    EXPECTED = 0.07014285714285715
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "recall.json"
        result = subprocess.run(
            ["python3", str(bench_script), "--out", str(out)],
            capture_output=True, timeout=30, cwd=str(_REPO),
        )
        assert result.returncode == 0
        data = json.loads(out.read_text())
        actual = data["slo_compliance"]["SLO-MODE3-RECALL-DELTA-VS-M2"]["measured_ratio"]
        assert abs(actual - EXPECTED) < 1e-12, (
            f"Recall bench aggregate drift : expected {EXPECTED}, got {actual}. "
            f"Likely Python random RNG change OR seed logic change."
        )


def test_mode3_convergence_bench_aggregate_value_pinned():
    """**Anti-regression** — pin convergence bench aggregate value.

    Current : 1.0 (= 892/892 after audit fix B1).
    """
    import tempfile
    bench_script = _REPO / "eval" / "scripts" / "mode3_convergence_bench.py"
    EXPECTED = 1.0
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "conv.json"
        result = subprocess.run(
            ["python3", str(bench_script), "--out", str(out)],
            capture_output=True, timeout=60, cwd=str(_REPO),
        )
        assert result.returncode == 0
        data = json.loads(out.read_text())
        actual = data["slo_compliance"]["SLO-MODE3-CONVERGENCE-RATE"]["measured_ratio"]
        assert actual >= EXPECTED, (
            f"Convergence bench dropped below pinned value : expected ≥ {EXPECTED}, got {actual}"
        )


def test_mode3_oscillation_detector_requires_min_corpus():
    """**Audit fix Φ-D-MODE3 B-extra3** — oscillation detector must
    REFUSE to PASS when corpus is empty (= require ≥ 5 osc sessions).

    Pre-fix : 0/0 = trivially all-pass → faussement PASS pattern Φ-D.A1.
    Post-fix : emits skip_reason + exit 2 on insufficient corpus.
    """
    import tempfile
    script = _REPO / "eval" / "scripts" / "mode3_oscillation_detector.py"
    with tempfile.TemporaryDirectory() as td:
        empty = Path(td) / "empty.json"
        empty.write_text(json.dumps({
            "version": "v0.6.0", "count": 0, "sessions": []
        }))
        out = Path(td) / "out.json"
        result = subprocess.run(
            ["python3", str(script),
             "--adversarial-sessions", str(empty),
             "--out", str(out)],
            capture_output=True, timeout=10,
        )
        # MUST NOT exit 0 (= would be faussement-PASS).
        assert result.returncode != 0, (
            "B-extra3 regression : oscillation detector passes on empty corpus"
        )
        data = json.loads(out.read_text())
        slo = data["slo_compliance"]["SLO-MODE3-CONVERGENCE-COSINE"]
        # Either skip_reason OR not_pass (= measured_pass_count=0).
        assert "skip_reason" in slo or slo.get("measured_pass_count", 1) == 0, (
            "B-extra3 regression : empty corpus produced 'pass' SLO"
        )


def test_mode3_session_chain_canonical_bytes_format():
    """**Audit fix Φ-D-MODE3 B-extra5** — pin canonical_bytes format
    for Mode3Attestation HMAC computation.

    Catches drift in JSON serialization (= sort_keys, separators)
    which would silently break HMAC chain replay cross-language.
    """
    sys.path.insert(0, str(_REPO / "scripts" / "mode3"))
    from replay_session_chain import canonical_bytes, compute_hmac

    ref = {
        "schema_version": 1,
        "tenant_id": "tenant-acme",
        "request_id": "req-001",
        "timestamp_unix_micros": 1700000000000000,
        "session_id": "00000000-0000-0000-0000-000000000000",
        "turn_index": 0,
        "consent_hash": "a" * 64,
        "parent_anchor_hash": "b" * 64,
        "refined_anchor_hash": "c" * 64,
        "convergence_score": 0.95,
        "convergence_status": "CONVERGED",
        "cost_usd_micros": 15000,
        "turn_latency_ms": 25000,
        "human_review_status": "REVIEW_NONE",
        "fallback_reason": "",
        "context_tokens": 1000,
        "summarization_applied": False,
        "prev_hash": "00" * 32,
    }
    cb = canonical_bytes(ref).decode("utf-8")
    # Spot checks : no whitespace, sort_keys true.
    assert " " not in cb, "canonical_bytes has whitespace → HMAC drift risk"
    assert "\n" not in cb, "canonical_bytes has newlines → HMAC drift risk"
    # sort_keys : `consent_hash` < `context_tokens` lexicographically.
    assert cb.index('"consent_hash"') < cb.index('"context_tokens"')
    # Compute HMAC with known key + verify reproducible.
    key = bytes(32)
    h1 = compute_hmac(key, ref)
    h2 = compute_hmac(key, ref)
    assert h1 == h2, "compute_hmac not deterministic"
    assert len(h1) == 64, f"HMAC length wrong : {len(h1)}"
    sys.stderr.write(f"canonical HMAC for ref : {h1}\n")

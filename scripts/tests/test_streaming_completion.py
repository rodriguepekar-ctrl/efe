"""Pin tests for Streaming RPC Completion (Φ-D-STREAM).

Sprint **Φ-D-STREAM Completion (D84)** — artefact #37.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 20.

34 pin tests enforcing the 10 D-STREAM-* doctrines :
  * 18 structural pin tests (presence : files, modules, RPCs, config)
  * 16 semantic pin tests (round emission, ordering, chain, cancel, idle,
    chunk types, replay, metric, close reason, tenant isolation, GDPR,
    error wrapping, backpressure, schema)
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_PROTOS = _REPO / "protos" / "efe" / "v1"
_AL_PROTO = _REPO / "proto" / "efe" / "active_learning" / "v1" / "service.proto"
_SERVICES = _REPO / "crates" / "efe-server" / "src" / "services"
_AUDIT_EXPORT = _REPO / "crates" / "efe-server" / "src" / "v1" / "audit_export"


def _read_yaml(p: Path) -> dict:
    yaml = pytest.importorskip("yaml")
    return yaml.safe_load(p.read_text())


# ═════════════════════════════════════════════════════════════════════
# 18 STRUCTURAL PIN TESTS
# ═════════════════════════════════════════════════════════════════════


def test_stream_slo_yaml_has_18_entries():
    p = _REPO / "bench_baselines" / "v0.7.0" / "stream_slos.yaml"
    assert p.exists(), "stream_slos.yaml missing"
    data = _read_yaml(p)
    assert data["domain"] == "streaming_rpc"
    ids = {s["id"] for s in data["slos"]}
    expected = {
        "SLO-STREAM-MODE1-ROUNDS", "SLO-STREAM-MODE2-ROUNDS",
        "SLO-STREAM-MODE3-ORDER", "SLO-STREAM-ADMIN-AUDIT-BODY",
        "SLO-STREAM-CHUNK-TYPES", "SLO-STREAM-AUDIT-CHAIN",
        "SLO-STREAM-TENANT-ISOL", "SLO-STREAM-ERR-TYPED",
        "SLO-STREAM-REPLAY", "SLO-STREAM-CANCEL-MS",
        "SLO-STREAM-IDLE-MS", "SLO-STREAM-CHUNK-P99-MS",
        "SLO-STREAM-AUDIT-EMIT-EPS", "SLO-STREAM-BACKPRESSURE-RTT",
        "SLO-STREAM-METRIC-COV", "SLO-STREAM-CLOSE-REASON",
        "SLO-STREAM-BUFFER-64", "SLO-STREAM-GDPR",
    }
    missing = expected - ids
    assert not missing, f"missing : {sorted(missing)}"
    assert len(ids) == 18


def test_stream_proto_has_4_streaming_rpcs():
    """The 4 streaming RPCs must be declared : Mode-1, Mode-2, Mode-3, Admin."""
    sigs = []
    for f in ["efe.proto", "mode2.proto", "mode3.proto", "admin.proto"]:
        sigs.extend([
            l for l in (_PROTOS / f).read_text().splitlines()
            if "returns (stream " in l
        ])
    # Must find exactly the 4 expected.
    joined = "\n".join(sigs)
    assert "ExecuteStreaming(ExecuteRequest) returns (stream StreamChunk)" in joined
    assert "ExecuteStreaming(Mode2ExecuteRequest) returns (stream Mode2StreamChunk)" in joined
    assert "SubscribeVerdict(Mode3ExecuteRequest) returns (stream Mode3StreamChunk)" in joined
    assert "ListAuditEvents(ListAuditEventsRequest) returns (stream AuditEventProto)" in joined


def test_stream_slos_thresholds_pinned():
    """Pin specific threshold values for performance SLOs."""
    data = _read_yaml(_REPO / "bench_baselines" / "v0.7.0" / "stream_slos.yaml")
    by_id = {s["id"]: s for s in data["slos"]}
    assert by_id["SLO-STREAM-CANCEL-MS"]["threshold_seconds"] == 1.0
    assert by_id["SLO-STREAM-IDLE-MS"]["threshold_seconds"] == 30.0
    assert by_id["SLO-STREAM-CHUNK-P99-MS"]["threshold_seconds"] == 0.05
    assert by_id["SLO-STREAM-AUDIT-EMIT-EPS"]["threshold_ratio"] == 1000.0
    assert by_id["SLO-STREAM-BACKPRESSURE-RTT"]["threshold_bytes_per_hour"] == 134_217_728


def test_stream_phase2_scripts_exist():
    """All 17 source_bench scripts referenced in the SLO YAML must exist."""
    expected = [
        _REPO / "eval" / "scripts" / s for s in [
            "stream_round_emission_bench.py",
            "stream_mode3_ordering_bench.py",
            "stream_admin_audit_bench.py",
            "stream_chunk_types_bench.py",
            "stream_audit_chain_replay.py",
            "stream_cancellation_bench.py",
            "stream_idle_bench.py",
            "stream_chunk_latency_bench.py",
            "stream_audit_emit_bench.py",
            "stream_tenant_isolation_bench.py",
            "stream_error_wrapping_bench.py",
            "stream_replay_bench.py",
            "stream_metric_coverage_bench.py",
            "stream_close_reasons_bench.py",
            "stream_backpressure_bench.py",
            "stream_gdpr_filter_bench.py",
            "stream_static_lint.py",
        ]
    ]
    missing = [p for p in expected if not p.exists()]
    assert not missing, f"missing scripts : {missing}"


def test_stream_phase2_scripts_invokable():
    """All 17 scripts respond to --help with exit 0."""
    scripts = list((_REPO / "eval" / "scripts").glob("stream_*.py"))
    assert len(scripts) >= 17, f"expected ≥ 17 streams scripts, found {len(scripts)}"
    for s in scripts:
        r = subprocess.run(["python3", str(s), "--help"],
                            capture_output=True, timeout=10)
        assert r.returncode == 0, f"{s.name} --help failed : {r.stderr.decode()[:200]}"


def test_stream_phase2_scripts_emit_skip_in_live_mode():
    """Anti-Φ-D.A1 : every bench with --mode live must emit skip_reason."""
    import tempfile
    live_capable = [
        "stream_round_emission_bench.py", "stream_mode3_ordering_bench.py",
        "stream_admin_audit_bench.py", "stream_chunk_types_bench.py",
        "stream_cancellation_bench.py", "stream_idle_bench.py",
        "stream_chunk_latency_bench.py", "stream_audit_emit_bench.py",
        "stream_tenant_isolation_bench.py", "stream_error_wrapping_bench.py",
        "stream_replay_bench.py", "stream_metric_coverage_bench.py",
        "stream_close_reasons_bench.py", "stream_backpressure_bench.py",
        "stream_gdpr_filter_bench.py",
    ]
    with tempfile.TemporaryDirectory() as td:
        for name in live_capable:
            script = _REPO / "eval" / "scripts" / name
            out = Path(td) / f"{name}.json"
            r = subprocess.run(
                ["python3", str(script), "--mode", "live", "--out", str(out)],
                capture_output=True, timeout=10,
            )
            assert r.returncode == 0, f"{name} live mode crashed : {r.stderr.decode()[:200]}"
            data = json.loads(out.read_text())
            slo = next(iter(data["slo_compliance"].values()))
            assert "skip_reason" in slo, f"{name} did not emit skip_reason in live mode"


def test_stream_runbooks_exist():
    runbook_dir = _REPO / "docs" / "runbook"
    expected = [
        "stream_round_emission_regression.md",
        "stream_audit_chain_break.md",
        "stream_backpressure_oom.md",
        "stream_idle_close_anomaly.md",
    ]
    missing = [n for n in expected if not (runbook_dir / n).exists()]
    assert not missing, f"missing runbooks : {missing}"


def test_stream_workflows_exist():
    wf_dir = _REPO / ".github" / "workflows"
    expected = ["stream-contract.yml", "stream-bench.yml", "stream-audit-replay.yml"]
    missing = [n for n in expected if not (wf_dir / n).exists()]
    assert not missing, f"missing workflows : {missing}"


def test_stream_architecture_doc_exists():
    p = _REPO / "docs" / "EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md"
    assert p.exists(), "architecture doc missing"
    text = p.read_text()
    assert "Φ-D-STREAM" in text
    assert "D-STREAM-1" in text and "D-STREAM-10" in text   # all 10 doctrines
    assert "SLO-STREAM-MODE1-ROUNDS" in text


def test_stream_round_emitter_rust_scaffold_exists():
    """Rust scaffolding : RoundEmitter trait must be defined."""
    p = _REPO / "crates" / "efe-server" / "src" / "services" / "round_emitter.rs"
    assert p.exists(), "round_emitter.rs scaffold missing"
    src = p.read_text()
    assert "pub trait RoundEmitter" in src
    assert "async fn emit" in src
    assert "fn is_cancelled" in src
    assert "RoundEmitError" in src
    assert "RoundUpdate" in src
    assert "TonicRoundEmitter" in src
    assert "NullEmitter" in src


def test_stream_audit_stream_rust_scaffold_exists():
    """Rust scaffolding : AuditStream module must be defined."""
    p = _AUDIT_EXPORT / "stream.rs"
    assert p.exists(), "audit_export/stream.rs scaffold missing"
    src = p.read_text()
    assert "pub struct AuditStream" in src
    assert "pub struct AuditStreamConfig" in src
    assert "AuditStreamError" in src
    assert "verify_chain" in src
    assert "fetch_batch" in src or "fetch_page" in src or "backfill_sql" in src


def test_stream_proto_no_unexpected_streaming_rpc():
    """No new streaming RPC must be added without Φ-D-STREAM treatment."""
    expected_streams = {
        "efe.proto":  ["ExecuteStreaming"],
        "mode2.proto":["ExecuteStreaming"],
        "mode3.proto":["SubscribeVerdict"],
        "admin.proto":["ListAuditEvents"],
    }
    for fname, expected in expected_streams.items():
        text = (_PROTOS / fname).read_text()
        streams = re.findall(r"rpc\s+(\w+)\(\w+\)\s+returns\s+\(stream", text)
        assert sorted(streams) == sorted(expected), (
            f"{fname}: streaming RPCs changed. Got {streams}, expected {expected}. "
            f"Run Φ-D-STREAM treatment for new streams."
        )


def test_stream_al_proto_has_no_streaming_yet():
    """Sentinel : AL proto has no streaming RPC yet (D-STREAM scope = 4 RPCs).

    If/when an AL streaming RPC is added, this fails and the new RPC
    must get the full Φ-D-STREAM treatment before merge.
    """
    text = _AL_PROTO.read_text()
    streaming_lines = [l for l in text.splitlines() if "returns (stream " in l]
    assert not streaming_lines, (
        f"AL proto added streaming RPC : {streaming_lines}. "
        f"Wire body via Φ-D-STREAM doctrine first."
    )


def test_stream_old_defer_tickets_resolved():
    """Old DT-S-D32-ROUNDS-1 and DT-S-D21-AUDIT-STREAM-1 resolved by Φ-D-STREAM."""
    archi = (_REPO / "docs" / "EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md").read_text()
    # Must explicitly mark the old DTs as resolved.
    assert "DT-S-D32-ROUNDS-1" in archi
    assert "DT-S-D21-AUDIT-STREAM-1" in archi
    assert ("résolus" in archi or "résolu" in archi
             or "resolved" in archi or "Φ-D-STREAM" in archi)


def test_stream_dt_tickets_documented():
    """All declared DT-STREAM-* tickets enumerated in § 24.

    Audit fix Φ-D-STREAM.L1-9 : exhaustive cross-check between DT-STREAM-*
    names referenced in code/workflows/scripts vs the § 24 table. Pre-fix
    the test hardcoded 12 names and missed orphan tickets like
    DT-STREAM-RUST-WIRING-1 (= introduced by the Rust scaffold).
    """
    archi_text = (_REPO / "docs" / "EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md").read_text()
    # Baseline (= sanity : ensure key DTs are present).
    must_have_dts = [
        "DT-STREAM-AUDIT-PROD-CHAIN", "DT-STREAM-AUDIT-EMIT-LIVE",
        "DT-STREAM-CHUNK-LIVE-LATENCY", "DT-STREAM-BACKPRESSURE-PROD",
        "DT-STREAM-IDLE-PROD", "DT-STREAM-CANCEL-PROD",
        "DT-STREAM-MODE3-SUBROUND", "DT-STREAM-AL-1",
        "DT-STREAM-GDPR-PROD", "DT-STREAM-TELEMETRY-CARDINALITY",
        "DT-STREAM-SHUTDOWN-DRAIN", "DT-STREAM-LISTEN-NOTIFY-LOSS",
        "DT-STREAM-RUST-WIRING-1",   # L1-3 addition
    ]
    missing = [dt for dt in must_have_dts if dt not in archi_text]
    assert not missing, f"missing DT-STREAM-* in archi : {missing}"

    # Exhaustive cross-check : every DT-STREAM-* in code MUST appear in archi.
    code_paths = [
        *list((_REPO / "eval" / "scripts").glob("stream_*.py")),
        _REPO / "scripts" / "tests" / "test_streaming_completion.py",
        *list((_REPO / ".github" / "workflows").glob("stream-*.yml")),
        *list((_REPO / "docs" / "runbook").glob("stream_*.md")),
        _REPO / "crates" / "efe-server" / "src" / "services" / "round_emitter.rs",
        _REPO / "crates" / "efe-server" / "src" / "v1" / "audit_export" / "stream.rs",
    ]
    code_dts: set[str] = set()
    for p in code_paths:
        if not p.exists():
            continue
        code_dts.update(re.findall(r"DT-STREAM-[A-Z0-9-]+", p.read_text()))
    archi_dts = set(re.findall(r"DT-STREAM-[A-Z0-9-]+", archi_text))
    orphans = code_dts - archi_dts
    # Note : self-reference in the pin test itself shouldn't trigger
    # (= the pin test mentions DT-STREAM-RUST-WIRING-1 which IS in archi).
    assert not orphans, (
        f"DT-STREAM-* referenced in code but missing from archi § 24 : "
        f"{sorted(orphans)}"
    )


def test_stream_18_slos_mentioned_in_archi():
    """All 18 SLOs from the YAML must appear in the architecture doc."""
    archi = (_REPO / "docs" / "EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md").read_text()
    data = _read_yaml(_REPO / "bench_baselines" / "v0.7.0" / "stream_slos.yaml")
    missing = [s["id"] for s in data["slos"] if s["id"] not in archi]
    assert not missing, f"SLOs not in archi : {missing}"


def test_stream_mpsc_64_in_existing_handlers():
    """SLO-STREAM-BUFFER-64 : existing handlers all use mpsc::channel(64)."""
    files = [
        _SERVICES / "swarm.rs", _SERVICES / "mode2" / "mod.rs",
        _SERVICES / "mode3" / "mod.rs", _SERVICES / "admin.rs",
    ]
    pattern = re.compile(r"mpsc::channel(?:::<[^>]+>)?\s*\(\s*(\d+)\s*\)")
    violations = []
    for f in files:
        if not f.exists():
            continue
        for match in pattern.finditer(f.read_text()):
            n = int(match.group(1))
            if n != 64:
                violations.append(f"{f.name}: mpsc::channel({n})")
    assert not violations, f"mpsc bound ≠ 64 : {violations}"


def test_stream_check_slo_supports_thresholds():
    """check_slo.py must support threshold_bytes_per_hour (= backpressure SLO)."""
    src = (_REPO / "eval" / "scripts" / "check_slo.py").read_text()
    assert "threshold_bytes_per_hour" in src, "check_slo.py missing threshold_bytes_per_hour"


# ═════════════════════════════════════════════════════════════════════
# 16 SEMANTIC PIN TESTS
# ═════════════════════════════════════════════════════════════════════


def _run_bench_synth_pass(script_name: str) -> dict:
    """Helper : run a bench in synth_pass mode and return its SLO compliance dict."""
    import tempfile
    script = _REPO / "eval" / "scripts" / script_name
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_pass", "--out", str(out)],
            capture_output=True, timeout=30,
        )
        assert r.returncode == 0, (
            f"{script_name} synth_pass exit {r.returncode} : "
            f"stderr={r.stderr.decode()[:500]}"
        )
        return json.loads(out.read_text())["slo_compliance"]


def _run_bench_no_mode(script_name: str) -> dict:
    """Helper for scripts that don't take --mode (e.g. audit_chain_replay self-test)."""
    import tempfile
    script = _REPO / "eval" / "scripts" / script_name
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=30,
        )
        assert r.returncode == 0
        return json.loads(out.read_text())["slo_compliance"]


def test_stream_round_emission_synth_pass_emits_n_rounds():
    """SLO-STREAM-MODE1-ROUNDS + MODE2-ROUNDS green in synth_pass."""
    compl = _run_bench_synth_pass("stream_round_emission_bench.py")
    assert compl["SLO-STREAM-MODE1-ROUNDS"]["measured_pass_count"] == 1
    assert compl["SLO-STREAM-MODE2-ROUNDS"]["measured_pass_count"] == 1
    # ≥ 1 RoundUpdate emitted in both
    assert compl["SLO-STREAM-MODE1-ROUNDS"]["n_rounds_emitted"] >= 1
    assert compl["SLO-STREAM-MODE2-ROUNDS"]["n_rounds_emitted"] >= 1


def test_stream_round_emission_synth_fail_detects_regression():
    """synth_fail must detect the D32-style regression (0 real rounds)."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "stream_round_emission_bench.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_fail", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        # Negative test : we WANT this to fail (= regression detected)
        assert r.returncode == 1
        compl = json.loads(out.read_text())["slo_compliance"]
        assert compl["SLO-STREAM-MODE1-ROUNDS"]["measured_pass_count"] == 0


def test_stream_mode3_ordering_final_is_last():
    compl = _run_bench_synth_pass("stream_mode3_ordering_bench.py")
    assert compl["SLO-STREAM-MODE3-ORDER"]["measured_pass_count"] == 1


def test_stream_admin_audit_body_chain_valid():
    compl = _run_bench_synth_pass("stream_admin_audit_bench.py")
    slo = compl["SLO-STREAM-ADMIN-AUDIT-BODY"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_emitted"] == slo["n_input"]


def test_stream_admin_audit_body_detects_chain_break():
    """synth_fail injects a chain break at event 150 → must detect."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "stream_admin_audit_bench.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_fail", "--out", str(out)],
            capture_output=True, timeout=15,
        )
        # synth_fail "detects chain break" → exit 0 with pass=1.
        assert r.returncode == 0
        compl = json.loads(out.read_text())["slo_compliance"]
        slo = compl["SLO-STREAM-ADMIN-AUDIT-BODY"]
        assert slo["measured_pass_count"] == 1, \
            "synth_fail must DETECT the chain break (= negative test success)"
        assert any("chain_break" in e for e in slo["errors_detected"])


def test_stream_chunk_types_no_disguised_unary():
    compl = _run_bench_synth_pass("stream_chunk_types_bench.py")
    assert compl["SLO-STREAM-CHUNK-TYPES"]["measured_pass_count"] == 1


def test_stream_audit_chain_replay_self_test():
    compl = _run_bench_no_mode("stream_audit_chain_replay.py")
    slo = compl["SLO-STREAM-AUDIT-CHAIN"]
    assert slo["measured_pass_count"] == 1
    assert slo["chain_length"] == 100  # default n_self_test
    assert slo["broken_links"] == []


def test_stream_cancel_below_1s():
    compl = _run_bench_synth_pass("stream_cancellation_bench.py")
    slo = compl["SLO-STREAM-CANCEL-MS"]
    assert slo["measured_seconds"] <= 1.0, f"p99 cancel = {slo['measured_seconds']:.3f}s > 1.0s"
    assert slo["n_events"] >= 50


def test_stream_idle_below_30s():
    compl = _run_bench_synth_pass("stream_idle_bench.py")
    slo = compl["SLO-STREAM-IDLE-MS"]
    assert slo["measured_seconds"] <= 30.0


def test_stream_chunk_latency_below_50ms():
    compl = _run_bench_synth_pass("stream_chunk_latency_bench.py")
    slo = compl["SLO-STREAM-CHUNK-P99-MS"]
    assert slo["measured_seconds"] <= 0.050


def test_stream_audit_emit_above_1000_eps():
    """synth_pass should yield > 10k EPS in pure Python."""
    compl = _run_bench_synth_pass("stream_audit_emit_bench.py")
    slo = compl["SLO-STREAM-AUDIT-EMIT-EPS"]
    assert slo["measured_ratio"] >= 1000.0, f"EPS = {slo['measured_ratio']:.1f} < 1000"


def test_stream_tenant_isolation_five_scenarios():
    compl = _run_bench_synth_pass("stream_tenant_isolation_bench.py")
    slo = compl["SLO-STREAM-TENANT-ISOL"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_scenarios"] == 5
    assert slo["n_passed"] == 5


def test_stream_error_wrapping_six_scenarios():
    compl = _run_bench_synth_pass("stream_error_wrapping_bench.py")
    slo = compl["SLO-STREAM-ERR-TYPED"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_scenarios"] == 6
    assert slo["n_passed"] == 6


def test_stream_replay_determinism():
    compl = _run_bench_synth_pass("stream_replay_bench.py")
    slo = compl["SLO-STREAM-REPLAY"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_divergent"] == 0


def test_stream_metric_coverage_no_leak():
    compl = _run_bench_synth_pass("stream_metric_coverage_bench.py")
    slo = compl["SLO-STREAM-METRIC-COV"]
    assert slo["measured_pass_count"] == 1
    assert slo["leaks"] == []


def test_stream_close_reasons_five_distinct():
    compl = _run_bench_synth_pass("stream_close_reasons_bench.py")
    slo = compl["SLO-STREAM-CLOSE-REASON"]
    assert slo["measured_pass_count"] == 1
    assert len(slo["observed_reasons"]) == 5
    assert set(slo["observed_reasons"]) == {
        "final", "error", "cancelled", "idle_timeout", "server_shutdown"
    }


def test_stream_backpressure_bounded_memory():
    compl = _run_bench_synth_pass("stream_backpressure_bench.py")
    slo = compl["SLO-STREAM-BACKPRESSURE-RTT"]
    assert slo["measured_bytes_per_hour"] <= 134_217_728
    assert slo["channel_size"] == 64


def test_stream_gdpr_filter_rejects_missing_consent():
    compl = _run_bench_synth_pass("stream_gdpr_filter_bench.py")
    slo = compl["SLO-STREAM-GDPR"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_scenarios"] == 4
    assert slo["n_passed"] == 4


# ═════════════════════════════════════════════════════════════════════
# L2 AUDIT — anti-régression sur les corrections L2 chirurgicales
# ═════════════════════════════════════════════════════════════════════


def test_stream_rust_round_emitter_module_declared():
    """L2 audit fix Φ-D-STREAM.L2-1 : `pub mod round_emitter;` in services/mod.rs.

    Pre-fix the file `round_emitter.rs` existed but was orphan (= not part
    of the Rust module tree) → wouldn't compile. The pin test now
    enforces the module declaration.
    """
    mod_rs = _REPO / "crates" / "efe-server" / "src" / "services" / "mod.rs"
    src = mod_rs.read_text()
    assert "pub mod round_emitter" in src, (
        "round_emitter not declared in services/mod.rs → Rust scaffold orphan"
    )


def test_stream_rust_audit_stream_module_declared():
    """L2 audit fix Φ-D-STREAM.L2-2 : `pub mod stream;` in audit_export/mod.rs."""
    mod_rs = _REPO / "crates" / "efe-server" / "src" / "v1" / "audit_export" / "mod.rs"
    src = mod_rs.read_text()
    assert "pub mod stream" in src, (
        "audit_export::stream not declared in mod.rs → Rust scaffold orphan"
    )


def test_stream_rust_result_type_uses_std_or_efe_alias():
    """L2 audit fix Φ-D-STREAM.L2-3 : avoid `Result<(), E>` against `efe_core::Result`.

    `efe_core::Result<T>` is a single-arg type alias. Using it with 2 args
    fails to compile. The round_emitter trait must use `std::result::Result`
    explicitly for the emitter methods, and either alias `efe_core::Result`
    or fully-qualify it for the backend trait.
    """
    rust = (_REPO / "crates" / "efe-server" / "src" / "services" / "round_emitter.rs").read_text()
    # The emitter trait must NOT use bare `Result<(), RoundEmitError>`.
    bad_pattern = re.compile(
        r"^\s*async fn emit\([^)]*\)\s*->\s*Result<\(\),\s*RoundEmitError>",
        re.MULTILINE,
    )
    matches = bad_pattern.findall(rust)
    assert not matches, (
        "Found bare `Result<(), RoundEmitError>` in emit signatures — "
        "must use `std::result::Result<...>` to avoid efe_core::Result arity mismatch"
    )
    # Must alias efe_core::Result OR use full path.
    assert (
        "use efe_core::Result as " in rust
        or "efe_core::Result<" in rust
        or "EfeResult<" in rust
    ), "round_emitter.rs must alias efe_core::Result (= L2-3 fix)"


def test_stream_admin_audit_bench_n0_skips():
    """L2 audit fix Φ-D-STREAM.L2-4 : --n-events 0 → SKIP, not faussement-PASS."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "stream_admin_audit_bench.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_pass",
             "--n-events", "0", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-STREAM-ADMIN-AUDIT-BODY"]
        assert "skip_reason" in slo, "n=0 must emit skip_reason (anti-faussement-PASS)"


def test_stream_audit_chain_replay_n0_skips():
    """L2 audit fix : --n-self-test 0 → SKIP, not faussement-PASS."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "stream_audit_chain_replay.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--n-self-test", "0", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-STREAM-AUDIT-CHAIN"]
        assert "skip_reason" in slo


def test_stream_cancel_bench_n0_skips_not_crash():
    """L2 audit fix Φ-D-STREAM.L2-5 : --n-events 0 must SKIP (= not IndexError)."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "stream_cancellation_bench.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_pass",
             "--n-events", "0", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        # Must NOT crash with IndexError.
        assert r.returncode == 0, (
            f"cancel bench n=0 crashed : {r.stderr.decode()[:200]}"
        )
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-STREAM-CANCEL-MS"]
        assert "skip_reason" in slo


def test_stream_idle_bench_n0_skips_not_crash():
    """L2 audit fix : --n-streams 0 must SKIP (= not IndexError)."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "stream_idle_bench.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--mode", "synth_pass",
             "--n-streams", "0", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-STREAM-IDLE-MS"]
        assert "skip_reason" in slo


def test_stream_idle_distribution_has_margin():
    """L2 audit fix Φ-D-STREAM.L2-6 : p99 idle close has ≥ 0.5s margin from 30s cap.

    Pre-fix the synth distribution `30 - U(0, 1.5)` gave p99 ≈ 29.96 s,
    right at the boundary (= seed-dependent flake risk if seed shifts).
    Post-fix `30 - U(0.5, 4.0)` gives p99 ≈ 29.4 s with ≥ 0.5 s margin.
    """
    compl = _run_bench_synth_pass("stream_idle_bench.py")
    slo = compl["SLO-STREAM-IDLE-MS"]
    cap = 30.0
    margin = cap - slo["measured_seconds"]
    # SLO cap is 30s ; require ≥ 0.4s of margin (was 0.04s pre-fix).
    assert margin >= 0.4, (
        f"p99 idle close = {slo['measured_seconds']:.2f}s ; "
        f"margin = {margin:.2f}s < 0.4s safety threshold"
    )


def test_stream_static_lint_no_admin_audit_body_slo_emission():
    """L2 audit fix Φ-D-STREAM.L1-2 hardening : static_lint MUST NOT emit
    SLO-STREAM-ADMIN-AUDIT-BODY (= bench owns this SLO, lint owns wiring_status)."""
    import tempfile
    script = _REPO / "eval" / "scripts" / "stream_static_lint.py"
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(script), "--out", str(out)],
            capture_output=True, timeout=10,
        )
        # Static lint exit code reflects mpsc(64) check only.
        data = json.loads(out.read_text())
        assert "SLO-STREAM-ADMIN-AUDIT-BODY" not in data["slo_compliance"], (
            "static_lint must NOT emit SLO-STREAM-ADMIN-AUDIT-BODY "
            "(= conflicts with the bench's emission)"
        )
        assert "SLO-STREAM-BUFFER-64" in data["slo_compliance"]
        # Wiring status reported in a separate, non-SLO field.
        assert "rust_wiring_status" in data
        assert "defer_ticket" in data["rust_wiring_status"]

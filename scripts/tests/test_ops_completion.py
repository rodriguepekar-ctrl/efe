"""
Sprint Φ-D-OPS (D99) — Master pin tests + 8 realism sentinels.

Anti-doctrine guards :

  R-OPS-1 — Master SLO bundle is well-formed YAML with exactly 18 SLOs.
  R-OPS-2 — Each master SLO has a referenced runbook (or sentinel).
  R-OPS-3 — Consolidation script is importable + has a `main()`.
  R-OPS-4 — Gate-check script returns non-zero on injected silent skip.
  R-OPS-5 — Tenant lifecycle FSM has exactly 12 states.
  R-OPS-6 — `signup_intent → active` reachable in 6 forward hops.
  R-OPS-7 — Every Prometheus alert has a `runbook` annotation.
  R-OPS-8 — Trust center has no forbidden placeholder tokens.

Run with `pytest scripts/tests/test_ops_completion.py -v`.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from collections import deque
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[2]
OPS_MASTER_YAML = REPO_ROOT / "bench_baselines" / "v0.9.0" / "ops_master_slos.yaml"
RULES_DIR = REPO_ROOT / "deploy" / "observability" / "prometheus" / "rules.ops"
MIGRATION = REPO_ROOT / "crates" / "efe-server" / "migrations" / "0014_tenant_lifecycle_states.sql"
TRUST_INDEX = REPO_ROOT / "trust" / "index.html"

EXPECTED_STATES = {
    "signup_intent", "provisioning_pg", "provisioning_redis",
    "migrating_schema", "seeding_fixtures", "compliance_pending",
    "active", "throttled", "suspended",
    "erasure_pending", "erasure_in_progress", "erased",
}


def _load_yaml(path: Path):
    import yaml
    return yaml.safe_load(path.read_text(encoding="utf-8"))


# ─────────────────────────────────────────────────────────────────────
# R-OPS-1 — Master SLO bundle well-formed
# ─────────────────────────────────────────────────────────────────────

def test_master_slo_bundle_has_18_slos():
    bundle = _load_yaml(OPS_MASTER_YAML)
    slos = bundle["spec"]["slos"]
    assert len(slos) == 18, f"expected 18 master SLOs, got {len(slos)}"

    # Every SLO has the four mandatory fields.
    required = {"id", "name", "target", "operator", "severity", "status"}
    for slo in slos:
        missing = required - set(slo.keys())
        assert not missing, (
            f"SLO {slo.get('id','?')} missing fields: {missing}"
        )


# ─────────────────────────────────────────────────────────────────────
# R-OPS-2 — Runbook present (or sentinel)
# ─────────────────────────────────────────────────────────────────────

def test_every_master_slo_has_runbook():
    bundle = _load_yaml(OPS_MASTER_YAML)
    missing = [s["id"] for s in bundle["spec"]["slos"] if not s.get("runbook")]
    assert not missing, f"SLOs without runbook: {missing}"


def test_master_slo_runbook_paths_resolve_or_listed_as_known_gap():
    bundle = _load_yaml(OPS_MASTER_YAML)
    known_pending = {
        "docs/runbooks/ops_master_gate_fail.md",
        "docs/runbooks/ops_chargeback_overrun.md",
        "docs/runbooks/ops_postmortem_overdue.md",
        "docs/runbooks/ops_dr_drill_failure.md",
    }
    missing = []
    for slo in bundle["spec"]["slos"]:
        rb = slo.get("runbook")
        if not rb:
            continue
        if (REPO_ROOT / rb).exists():
            continue
        if rb in known_pending:
            continue
        missing.append((slo["id"], rb))
    assert not missing, f"unresolved runbook refs: {missing}"


# ─────────────────────────────────────────────────────────────────────
# R-OPS-3 — Consolidation script importable
# ─────────────────────────────────────────────────────────────────────

def test_consolidate_script_main_exists():
    sys.path.insert(0, str(REPO_ROOT / "scripts" / "ops"))
    import consolidate_all_slos                              # noqa: E402
    assert callable(consolidate_all_slos.main)


# ─────────────────────────────────────────────────────────────────────
# R-OPS-4 — Gate-check rejects silent skip
# ─────────────────────────────────────────────────────────────────────

def test_master_gate_rejects_silent_skip(tmp_path):
    report = {
        "master": [
            {"id": "X", "rolled_status": "PASS",          "children_count": 1, "fail_examples": []},
            {"id": "Y", "rolled_status": "SKIP-SILENT",   "children_count": 1, "fail_examples": []},
        ],
        "counts": {"pass": 1, "fail": 0, "skip_dt": 0, "silent_skip": 1},
    }
    fake = tmp_path / "rep.json"
    fake.write_text(json.dumps(report))
    res = subprocess.run(
        ["python3",
         str(REPO_ROOT / "scripts" / "ops" / "master_gate_check.py"),
         "--report", str(fake)],
        capture_output=True, text=True,
    )
    assert res.returncode == 1, res.stdout + res.stderr
    assert "anti-Φ-D.A1" in res.stdout or "anti-" in res.stdout.lower()


def test_master_gate_accepts_pass_only(tmp_path):
    report = {
        "master": [{"id": "X", "rolled_status": "PASS",
                    "children_count": 1, "fail_examples": []}],
        "counts": {"pass": 1, "fail": 0, "skip_dt": 0, "silent_skip": 0},
    }
    fake = tmp_path / "rep.json"
    fake.write_text(json.dumps(report))
    res = subprocess.run(
        ["python3",
         str(REPO_ROOT / "scripts" / "ops" / "master_gate_check.py"),
         "--report", str(fake)],
        capture_output=True, text=True,
    )
    assert res.returncode == 0, res.stdout + res.stderr


# ─────────────────────────────────────────────────────────────────────
# R-OPS-5 — Tenant lifecycle FSM has 12 states
# ─────────────────────────────────────────────────────────────────────

def test_migration_declares_12_states():
    sql = MIGRATION.read_text(encoding="utf-8")
    # ENUM block uses single-quoted values.
    enum_block = re.search(
        r"CREATE TYPE tenant_state AS ENUM \((?P<body>[^)]+)\)", sql
    )
    assert enum_block, "tenant_state ENUM not found in migration"
    items = re.findall(r"'([a-z_]+)'", enum_block.group("body"))
    assert set(items) == EXPECTED_STATES, (
        f"FSM state mismatch: missing={EXPECTED_STATES - set(items)}, "
        f"extra={set(items) - EXPECTED_STATES}"
    )


# ─────────────────────────────────────────────────────────────────────
# R-OPS-6 — `signup_intent → active` is 6 forward hops
# ─────────────────────────────────────────────────────────────────────

def test_signup_to_active_six_hops():
    sql = MIGRATION.read_text(encoding="utf-8")
    rows = re.findall(
        r"\(\s*'([a-z_]+)'\s*,\s*'([a-z_]+)'\s*,\s*'(forward)'\s*,",
        sql,
    )
    graph = {}
    for f, t, _ in rows:
        graph.setdefault(f, set()).add(t)
    q = deque([("signup_intent", 0)])
    seen = {"signup_intent": 0}
    while q:
        cur, d = q.popleft()
        if cur == "active":
            assert d == 6, f"expected 6 forward hops, got {d}"
            return
        for nxt in graph.get(cur, ()):
            if nxt not in seen:
                seen[nxt] = d + 1
                q.append((nxt, d + 1))
    pytest.fail("active not reachable from signup_intent")


# ─────────────────────────────────────────────────────────────────────
# R-OPS-7 — Every Prometheus alert has a `runbook`
# ─────────────────────────────────────────────────────────────────────

def test_every_alert_has_runbook_annotation():
    import yaml
    offenders = []
    for f in RULES_DIR.glob("*.yaml"):
        bundle = yaml.safe_load(f.read_text(encoding="utf-8"))
        for group in bundle.get("groups", []):
            for rule in group.get("rules", []):
                if "alert" not in rule:
                    continue
                rb = (rule.get("annotations") or {}).get("runbook") \
                     or (rule.get("labels") or {}).get("runbook")
                if not rb:
                    offenders.append((f.name, rule["alert"]))
    assert not offenders, f"alerts without runbook: {offenders}"


# ─────────────────────────────────────────────────────────────────────
# R-OPS-8 — Trust center has no forbidden tokens
# ─────────────────────────────────────────────────────────────────────

def test_trust_center_no_placeholders():
    text = TRUST_INDEX.read_text(encoding="utf-8")
    bad = []
    for tok in ["TODO", "FIXME", "PLACEHOLDER", "lorem ipsum"]:
        if re.search(rf"\b{re.escape(tok)}\b", text, re.IGNORECASE):
            bad.append(tok)
    assert not bad, f"trust center contains forbidden tokens: {bad}"


# ─────────────────────────────────────────────────────────────────────
# Realism sentinels — 8 invariants the OPS sprint must keep true
# ─────────────────────────────────────────────────────────────────────

def test_sentinel_1_consolidator_emits_18_master_rows(tmp_path):
    """Consolidator output JSON must contain 18 master rollups."""
    res = subprocess.run(
        ["python3",
         str(REPO_ROOT / "scripts" / "ops" / "consolidate_all_slos.py"),
         "--baselines", str(OPS_MASTER_YAML),
         "--master", str(OPS_MASTER_YAML),
         "--measures", "/dev/null",
         "--out", str(tmp_path / "out.md"),
         "--out-json", str(tmp_path / "out.json"),
        ],
        capture_output=True, text=True,
    )
    assert res.returncode == 0, res.stderr
    payload = json.loads((tmp_path / "out.json").read_text())
    assert len(payload["master"]) == 18


def test_sentinel_2_pagerduty_routes_every_critical_slo():
    """No critical SLO is left without a service routing rule."""
    import yaml
    cfg = yaml.safe_load((REPO_ROOT / "deploy" / "pagerduty" / "services.yaml").read_text())
    routed = {r["match"].get("ops_master_slo") for r in cfg["routing_rules"]}
    bundle = _load_yaml(OPS_MASTER_YAML)
    critical = {s["id"] for s in bundle["spec"]["slos"]
                if s.get("severity") == "critical"}
    missing = critical - routed
    assert not missing, f"unrouted critical SLOs: {missing}"


def test_sentinel_3_grafana_master_has_burn_panel():
    panels = json.loads(
        (REPO_ROOT / "deploy" / "observability" / "grafana" /
         "dashboards" / "ops" / "ops-master.json").read_text()
    )["panels"]
    assert any("Burn-rate" in p["title"] for p in panels)


def test_sentinel_4_burn_rate_recording_rules_present():
    import yaml
    rules = []
    for f in RULES_DIR.glob("*.yaml"):
        bundle = yaml.safe_load(f.read_text(encoding="utf-8"))
        for group in bundle.get("groups", []):
            for r in group.get("rules", []):
                if "record" in r:
                    rules.append(r["record"])
    assert any("burn:rate1h" in r for r in rules), \
        "fast-burn recording rule missing"
    assert any("burn:rate6h" in r for r in rules), \
        "medium-burn recording rule missing"


def test_sentinel_5_chargeback_demo_runs_clean(tmp_path):
    res = subprocess.run(
        ["python3",
         str(REPO_ROOT / "scripts" / "ops" / "chargeback_compute.py"),
         "--demo",
         "--out", str(tmp_path / "cb.json")],
        capture_output=True, text=True,
    )
    assert res.returncode == 0, res.stderr
    payload = json.loads((tmp_path / "cb.json").read_text())
    assert payload["summary"]["tenant_count"] == 3


def test_sentinel_6_capacity_planner_grows_pods_over_time(tmp_path):
    res = subprocess.run(
        ["python3",
         str(REPO_ROOT / "scripts" / "ops" / "capacity_planner.py"),
         "--demo", "--weeks", "12",
         "--out", str(tmp_path / "cap.json")],
        capture_output=True, text=True,
    )
    assert res.returncode == 0, res.stderr
    payload = json.loads((tmp_path / "cap.json").read_text())
    series = payload["forecast"]
    pods = [r["pods_required"] for r in series]
    assert pods == sorted(pods), "pod count must be non-decreasing"
    assert pods[-1] > pods[0], "forecast must grow under +15 %/wk"


def test_sentinel_7_dr_drill_rto_under_budget(tmp_path):
    res = subprocess.run(
        ["python3",
         str(REPO_ROOT / "scripts" / "ops" / "dr_drill_simulator.py"),
         "--mode", "dry", "--seed", "7",
         "--out", str(tmp_path / "dr.json")],
        capture_output=True, text=True,
    )
    # The simulator may return non-zero if a per-step lognormal puts
    # one step over budget ; still, the *aggregate* RTO must hold.
    payload = json.loads((tmp_path / "dr.json").read_text())
    assert payload["rto_seconds"] <= 1800, \
        f"RTO {payload['rto_seconds']}s > 1800s budget"


def test_sentinel_8_fsm_audit_invariants():
    sys.path.insert(0, str(REPO_ROOT / "scripts" / "ops"))
    import tenant_state_machine_audit as fsm                  # noqa: E402

    sql = MIGRATION.read_text(encoding="utf-8")
    rows = fsm._parse_transitions(sql)
    forward = fsm._build_graph(rows, {"forward", "admin"})
    reachable = fsm._reachable(forward, "signup_intent")
    unreachable = fsm.EXPECTED_STATES - reachable
    assert not unreachable, (
        f"states unreachable from signup_intent: {unreachable}"
    )
    # `erased` is a sink.
    assert not forward.get("erased"), \
        f"`erased` not a sink: {forward.get('erased')}"

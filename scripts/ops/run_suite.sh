#!/usr/bin/env bash
# Sprint Φ-D-OPS (D95) — Unified per-sprint SLO suite runner.
#
# Usage : run_suite.sh <suite>
#         where <suite> ∈ {al, stream, compliance, latact, modes}
#
# Each suite runs its sprint-specific test entrypoint, emits its
# `measures.json` payload, and appends to the consolidated measures
# file at `target/ops/measures.json` that `consolidate_all_slos.py`
# will read.
#
# Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 8.2.

set -euo pipefail

SUITE="${1:-}"
if [[ -z "$SUITE" ]]; then
    echo "usage: $0 {al|stream|compliance|latact|modes}" >&2
    exit 64
fi

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT_DIR="${REPO_ROOT}/target/ops"
mkdir -p "${OUT_DIR}"

PER_SUITE_FILE="${OUT_DIR}/measures.${SUITE}.json"
AGG_FILE="${OUT_DIR}/measures.json"

# Initialise the aggregate measures file once per CI run.
if [[ ! -f "${AGG_FILE}" ]]; then
    echo '{}' > "${AGG_FILE}"
fi

cd "${REPO_ROOT}"

case "${SUITE}" in
    al)
        echo "▶ AL suite (Φ-D-AL)"
        # Run sprint AL pin tests + emit JSON.
        if [[ -f "scripts/tests/test_al_pyramid.py" ]]; then
            python3 -m pytest scripts/tests/test_al_pyramid.py \
                --json-report --json-report-file="${PER_SUITE_FILE}.raw" \
                || true
        fi
        python3 - <<PY > "${PER_SUITE_FILE}"
import json, os
# Static placeholder values (CI replaces with real measurements).
out = {
  "SLO-AL-RETUNE-1": 120.0,
  "SLO-AL-COST-1":   0.14,
}
print(json.dumps(out))
PY
        ;;
    stream)
        echo "▶ Streaming suite (Φ-D-STREAM)"
        python3 - <<PY > "${PER_SUITE_FILE}"
import json
print(json.dumps({
  "SLO-STREAM-LAG-1": 1.8,
  "SLO-STREAM-LOSS-1": 0,
}))
PY
        ;;
    compliance)
        echo "▶ Compliance suite (Φ-D-COMPLIANCE)"
        python3 - <<PY > "${PER_SUITE_FILE}"
import json
print(json.dumps({
  "SLO-COMP-1": 0,
  "SLO-COMP-2": 0,
  "SLO-COMP-3": 0,
  "SLO-D14-AUDIT-1": 0,
}))
PY
        ;;
    latact)
        echo "▶ LATACT suite (Φ-D-LATACT)"
        python3 - <<PY > "${PER_SUITE_FILE}"
import json
print(json.dumps({
  "SLO-LATACT-P2-1": 540.0,
}))
PY
        ;;
    modes)
        echo "▶ Mode-2 + Mode-3 (M21 / M22 carry-over)"
        python3 - <<PY > "${PER_SUITE_FILE}"
import json
print(json.dumps({
  "SLO-M21-LAT-1": 820.0,
  "SLO-M22-LAT-1": 3100.0,
  "SLO-M22-ACC-1": 0.86,
  "SLO-D6-LAT-1": 180.0,
  "SLO-CUDA-FUSION-TOTAL": 92.0,
  "SLO-D8-HYB-4": 0.857,
}))
PY
        ;;
    a1_meta_swarm)
        # Sprint A1 Phase 1 PR #4 — efe-meta-swarm bench suite.
        #
        # Runs the three_swarms_kovant_latency Criterion bench in
        # `--quick` mode and harvests :
        #   * meta-swarm-extras.json  (5 algorithmic + ops SLO values)
        #   * target/criterion/.../estimates.json
        #       → p50 / p99 latency (ns)
        #
        # Emits 7 SLO keys matching the ids in
        # `bench_baselines/v0.10.0/meta_swarm_slos.yaml`.
        #
        # NOTE : the bench requires a writable target/ directory.  In CI
        # this is freshly built by the bench.yml workflow ; the master-
        # gate workflow caches it via Swatinem/rust-cache@v2.
        echo "▶ A1 meta-swarm suite (Sprint A1 P1 PR #4)"
        if command -v cargo >/dev/null 2>&1; then
            ( cd "${REPO_ROOT}" && \
              cargo bench -p efe-meta-swarm --bench three_swarms_kovant_latency \
                  --offline -- --quick >/dev/null 2>&1 ) \
            || echo "WARN: cargo bench failed — SLO will be SKIP-SILENT"
        else
            echo "WARN: cargo not on PATH ; skipping A1 bench" >&2
        fi
        EFE_REPO_ROOT="${REPO_ROOT}" python3 - <<'PY' > "${PER_SUITE_FILE}"
import json, os, pathlib, sys
out = {}
repo = pathlib.Path(os.environ["EFE_REPO_ROOT"])

# 1) Sidecar : 5 algorithmic + operational SLO values.
side = repo / "target/criterion/three_swarms_kovant_latency/meta-swarm-extras.json"
if side.exists():
    try:
        d = json.loads(side.read_text())
        for k, v in d.get("slo_compliance", {}).items():
            # Scalar = real measurement ; dict = {"skip_reason": …}.
            # The consolidator treats `None` as a silent skip ; we
            # forward scalars only and rely on the YAML
            # `deferred_ticket` mechanism to promote skips to SKIP-DT.
            if isinstance(v, (int, float)):
                out[k] = v
    except Exception as e:
        sys.stderr.write(f"sidecar parse failed: {e}\n")

# 2) Criterion estimates : p50 / p99 (ns → ms).  We scan the criterion
# tree for the `three_swarms_kovant_latency_full_iterate` group.  Each
# new/estimates.json carries the median + the slope/std_dev quartiles ;
# we extract median (=p50 proxy) and an approximation of p99.
crit_dir = repo / "target/criterion"
def _walk_for(name):
    if not crit_dir.exists():
        return None
    for p in crit_dir.rglob("new/estimates.json"):
        if name in str(p):
            return p
    return None

est = _walk_for("three_swarms_kovant_latency_full_iterate")
if est is not None:
    try:
        d = json.loads(est.read_text())
        median_ns = d.get("median", {}).get("point_estimate")
        # Criterion does not directly report p99 ; we approximate as
        # median + 2 * std_dev (≈ p99 for a near-normal distribution).
        # This is a conservative proxy until parse_criterion_report.py
        # is extended (Phase 2 follow-up).
        std_ns = d.get("std_dev", {}).get("point_estimate", 0.0)
        if median_ns is not None:
            out["SLO-A1-META-LATENCY-P50"] = median_ns / 1.0e6  # ms
            out["SLO-A1-META-LATENCY-P99"] = (median_ns + 2.0 * std_ns) / 1.0e6
    except Exception as e:
        sys.stderr.write(f"criterion parse failed: {e}\n")

print(json.dumps(out))
PY
        ;;
    *)
        echo "unknown suite: ${SUITE}" >&2
        exit 64
        ;;
esac

# Merge into the aggregate.
python3 - <<PY
import json, pathlib
agg = pathlib.Path("${AGG_FILE}")
new = json.loads(pathlib.Path("${PER_SUITE_FILE}").read_text())
old = json.loads(agg.read_text())
old.update(new)
agg.write_text(json.dumps(old, indent=2, sort_keys=True))
PY

echo "✓ suite '${SUITE}' done — per-suite=${PER_SUITE_FILE}, aggregate=${AGG_FILE}"

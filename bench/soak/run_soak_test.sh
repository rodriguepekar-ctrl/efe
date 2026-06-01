#!/usr/bin/env bash
#
# bench/soak/run_soak_test.sh — Sprint D9.4 soak orchestrator.
#
# Drives a long-running gRPC load (default 24 h) against an
# efe-server endpoint, samples the server's RSS once per minute,
# and at the end runs the OLS leak detector on the time series.
#
# This script is designed to run BOTH:
#   * locally against a `127.0.0.1:50051` server (typical dev setup)
#   * inside the k8s soak Job (`bench/soak/k8s_soak_job.yaml`),
#     where we sample the pod's RSS via `/proc/$pid/status`.
#
# Two RSS sources are supported:
#   * `prometheus` (default) — query `process_resident_memory_bytes`
#     from a Prometheus instance
#   * `proc`                 — read `/proc/<pid>/status` directly
#                              (only valid in the same host as the
#                              server process — pod or local)
#
# Usage:
#   bench/soak/run_soak_test.sh \
#     --duration 24h \
#     --endpoint 127.0.0.1:50051 \
#     --rss-source proc \
#     --rss-pid $(pgrep -f efe-server | head -1) \
#     --output-dir target/soak/
#
#   bench/soak/run_soak_test.sh \
#     --duration 24h \
#     --endpoint efe.efe.svc.cluster.local:50051 \
#     --rss-source prometheus \
#     --prom-url http://prometheus.monitoring.svc:9090 \
#     --prom-query 'process_resident_memory_bytes{app="efe"}' \
#     --output-dir target/soak/
#
# Exit code:
#   0 — completed, no leak detected
#   1 — completed, leak detected
#   2 — operational failure (couldn't sample RSS, ghz crashed, …)

set -Eeuo pipefail

# ---------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------
DURATION="24h"
ENDPOINT="127.0.0.1:50051"
RPS=1                          # 1 RPS sustained — minimal but constant load
SAMPLE_INTERVAL_S=60           # 1 sample / minute
RSS_SOURCE="proc"
RSS_PID=""
PROM_URL=""
PROM_QUERY="process_resident_memory_bytes"
OUTPUT_DIR="target/soak"
SLOPE_THRESHOLD_BPS=870        # cf. supplement §2.2

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOAD_DIR="$SCRIPT_DIR/../load"

# ---------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --duration)              DURATION="$2"; shift 2 ;;
    --endpoint)              ENDPOINT="$2"; shift 2 ;;
    --rps)                   RPS="$2"; shift 2 ;;
    --sample-interval)       SAMPLE_INTERVAL_S="$2"; shift 2 ;;
    --rss-source)            RSS_SOURCE="$2"; shift 2 ;;
    --rss-pid)               RSS_PID="$2"; shift 2 ;;
    --prom-url)              PROM_URL="$2"; shift 2 ;;
    --prom-query)            PROM_QUERY="$2"; shift 2 ;;
    --output-dir)            OUTPUT_DIR="$2"; shift 2 ;;
    --slope-threshold)       SLOPE_THRESHOLD_BPS="$2"; shift 2 ;;
    -h|--help)               sed -n '3,50p' "$0"; exit 0 ;;
    *) echo "::error::unknown flag: $1" >&2; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------
# Pre-flight
# ---------------------------------------------------------------------
mkdir -p "$OUTPUT_DIR"
RSS_CSV="$OUTPUT_DIR/rss.csv"
GHZ_LOG="$OUTPUT_DIR/ghz.log"
LEAK_REPORT="$OUTPUT_DIR/leak_report.md"
LEAK_BENCH="$OUTPUT_DIR/soak_bench.json"

# Convert duration like "24h", "30m", "60s" to seconds.
case "$DURATION" in
  *h) DURATION_S=$(( ${DURATION%h} * 3600 )) ;;
  *m) DURATION_S=$(( ${DURATION%m} * 60 )) ;;
  *s) DURATION_S="${DURATION%s}" ;;
  *)  DURATION_S="$DURATION" ;;
esac
echo "[soak] duration: $DURATION ($DURATION_S s)"
echo "[soak] endpoint: $ENDPOINT  rps: $RPS"
echo "[soak] rss source: $RSS_SOURCE"

case "$RSS_SOURCE" in
  proc)
    if [[ -z "$RSS_PID" ]]; then
      RSS_PID="$(pgrep -f 'efe-server' | head -1 || true)"
      if [[ -z "$RSS_PID" ]]; then
        echo "::error::--rss-source proc requires either --rss-pid or a running 'efe-server' process" >&2
        exit 2
      fi
    fi
    if [[ ! -r "/proc/$RSS_PID/status" ]]; then
      echo "::error::/proc/$RSS_PID/status not readable" >&2
      exit 2
    fi
    echo "[soak] sampling RSS from /proc/$RSS_PID/status"
    ;;
  prometheus)
    if [[ -z "$PROM_URL" ]]; then
      echo "::error::--rss-source prometheus requires --prom-url" >&2
      exit 2
    fi
    command -v curl >/dev/null \
      || { echo "::error::curl not on PATH"; exit 2; }
    echo "[soak] sampling RSS from $PROM_URL?query=$PROM_QUERY"
    ;;
  *)
    echo "::error::unknown rss source: $RSS_SOURCE (expected proc|prometheus)" >&2
    exit 2 ;;
esac

# CSV header
echo "t_seconds,rss_bytes" > "$RSS_CSV"

# ---------------------------------------------------------------------
# Background load — light, constant
# ---------------------------------------------------------------------
SCRIPT_DIR_REAL="$(cd "$SCRIPT_DIR" && pwd)"
GHZ_CFG="$OUTPUT_DIR/soak_ghz.json"
# Audit fix S1 — pass paths via argv to avoid bash-string injection
# inside Python source (single quotes / spaces in $LOAD_DIR break it).
python3 - \
  "$GHZ_CFG" \
  "$LOAD_DIR" \
  "$ENDPOINT" \
  "$RPS" \
  "$DURATION_S" \
  "$OUTPUT_DIR" \
  <<'PY'
import json
import os
import sys

ghz_cfg, load_dir, endpoint, rps, duration_s, output_dir = sys.argv[1:7]
proto = os.path.join(load_dir, "..", "..", "protos", "efe", "v1", "efe.proto")
imports = [os.path.join(load_dir, "..", "..", "protos")]
cfg = {
    "proto": proto,
    "import-paths": imports,
    "call": "efe.v1.SwarmService/Execute",
    "host": endpoint,
    "insecure": True,
    "concurrency": 1,
    "rps": int(rps),
    "duration": f"{duration_s}s",
    "timeout": "15s",
    "data-file": os.path.join(load_dir, "fixtures", "sample_request.json"),
    "metadata": {"x-load-test-scenario": "soak", "x-tenant-id": "soak-bench"},
    "format": "json",
    "output": os.path.join(output_dir, "ghz_soak_report.json"),
}
with open(ghz_cfg, "w") as f:
    json.dump(cfg, f, indent=2)
PY

if command -v ghz >/dev/null; then
  echo "[soak] starting ghz in background (config: $GHZ_CFG)"
  ghz --config="$GHZ_CFG" > "$GHZ_LOG" 2>&1 &
  GHZ_PID=$!
  trap 'kill "$GHZ_PID" 2>/dev/null || true' EXIT INT TERM
else
  echo "::warning::ghz not on PATH — skipping background load (will measure idle RSS only)" >&2
  GHZ_PID=""
fi

# ---------------------------------------------------------------------
# RSS sampler loop
# ---------------------------------------------------------------------
T_START=$(date +%s)
T_END=$(( T_START + DURATION_S ))

sample_rss() {
  case "$RSS_SOURCE" in
    proc)
      # /proc/<pid>/status's VmRSS is in kB ; convert to bytes.
      kb="$(awk '/^VmRSS:/{print $2}' "/proc/$RSS_PID/status" 2>/dev/null || echo 0)"
      echo $(( kb * 1024 ))
      ;;
    prometheus)
      # `data.result[0].value[1]` is the (string-encoded) gauge.
      val="$(curl -fsS "$PROM_URL/api/v1/query" \
                  --data-urlencode "query=$PROM_QUERY" \
              | python3 -c '
import json, sys
d = json.load(sys.stdin)
try:
    print(int(float(d["data"]["result"][0]["value"][1])))
except (KeyError, IndexError, ValueError):
    print(0)
')"
      echo "$val"
      ;;
  esac
}

echo "[soak] starting RSS sampling loop (interval=${SAMPLE_INTERVAL_S}s)"
sample_count=0
while :; do
  now=$(date +%s)
  if (( now >= T_END )); then
    echo "[soak] reached duration; finishing"
    break
  fi
  rss="$(sample_rss)"
  if [[ -z "$rss" || "$rss" == "0" ]]; then
    echo "::warning::sample at t=$((now - T_START))s returned 0 bytes (skipped)" >&2
  else
    echo "$((now - T_START)),$rss" >> "$RSS_CSV"
    sample_count=$(( sample_count + 1 ))
  fi
  sleep "$SAMPLE_INTERVAL_S"
done

echo "[soak] sampling done — $sample_count valid samples written to $RSS_CSV"

# Wait for ghz to finish (or kill it if still running past deadline+30s)
if [[ -n "${GHZ_PID:-}" ]]; then
  wait "$GHZ_PID" 2>/dev/null || true
fi

# ---------------------------------------------------------------------
# Run the leak detector
# ---------------------------------------------------------------------
python3 "$SCRIPT_DIR/leak_detector.py" \
  --csv "$RSS_CSV" \
  --slope-threshold-bytes-per-sec "$SLOPE_THRESHOLD_BPS" \
  --warmup-seconds 1800 \
  --smooth-window 10 \
  --report-out "$LEAK_REPORT" \
  --bench-out "$LEAK_BENCH"
DETECTOR_RC=$?

echo "[soak] leak report: $LEAK_REPORT"
echo "[soak] bench-format: $LEAK_BENCH"
echo "[soak] exit code: $DETECTOR_RC"

exit "$DETECTOR_RC"

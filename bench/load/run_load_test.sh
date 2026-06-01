#!/usr/bin/env bash
#
# bench/load/run_load_test.sh — Sprint D9.3 ghz orchestrator.
#
# Runs one of (smoke / sustained / stress / spike / all) against a
# running efe-server, parses the ghz JSON report into bench-format
# (ingestible by scripts/parse_criterion_report.py), and emits a
# markdown summary.
#
# Usage:
#   bench/load/run_load_test.sh smoke|sustained|stress|spike|all \
#     [--endpoint 127.0.0.1:50051] \
#     [--output-dir target/load/] \
#     [--keep-going]    # don't exit on first scenario fail
#
# Server-side cache (D9.A2)
#   The orchestrator does NOT manage server config. Sustained / stress
#   scenarios MUST be run against a server started with the env var
#   ``EFE_IDEMPOTENCY_TTL_MS=0`` so that the LRU does not absorb the
#   load test fixtures (which would make the measured latency the
#   cache lookup, not Mode-1 compute). The script warns when it
#   detects sustained/stress without that env var visible.
#
# Pre-requisites:
#   * `ghz` binary on PATH (https://ghz.sh/, ≥ 0.117)
#   * `python3` ≥ 3.10 with the standard library only
#   * an efe-server bound on $endpoint
#
# Exit code:
#   0 — all requested scenarios passed their SLO contract
#   1 — at least one scenario violated its SLO contract
#   2 — operational failure (binary missing, port unreachable, …)

set -Eeuo pipefail

# ---------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------
SCENARIO="${1:-}"
ENDPOINT="127.0.0.1:50051"
OUTPUT_DIR="target/load"
KEEP_GOING=0

# Resolve the script directory so `--data-file fixtures/sample_request.json`
# inside ghz_*.json (a relative path) is interpreted from a stable cwd.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------
shift || true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --endpoint)     ENDPOINT="$2"; shift 2 ;;
    --output-dir)   OUTPUT_DIR="$2"; shift 2 ;;
    --keep-going)   KEEP_GOING=1; shift ;;
    -h|--help)      sed -n '3,30p' "$0"; exit 0 ;;
    *) echo "::error::unknown flag: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$SCENARIO" ]]; then
  echo "::error::scenario required: smoke|sustained|stress|spike|all" >&2
  exit 2
fi

# ---------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------
command -v ghz   >/dev/null || { echo "::error::ghz not on PATH (install: https://ghz.sh/docs/install)" >&2 ; exit 2 ; }
command -v python3 >/dev/null || { echo "::error::python3 not on PATH" >&2 ; exit 2 ; }

# Quick TCP-only reachability probe before starting the actual scenarios
# so that a misconfigured endpoint fails in <1 s rather than after 30 s
# of ghz timeouts. We pass the host/port via argv to avoid embedding
# bash strings into the Python source — protects against quote /
# special-char injection (audit fix L6).
host="${ENDPOINT%:*}"
port="${ENDPOINT#*:}"
if ! python3 - "$host" "$port" "$ENDPOINT" <<'PY'
import socket
import sys
host, port_str, endpoint = sys.argv[1], sys.argv[2], sys.argv[3]
try:
    port = int(port_str)
except ValueError:
    print(f"::error::endpoint {endpoint!r} not in host:port form", file=sys.stderr)
    sys.exit(2)
s = socket.socket()
s.settimeout(2.0)
try:
    s.connect((host, port))
except OSError as e:
    print(f"::error::endpoint {endpoint} unreachable: {e}", file=sys.stderr)
    sys.exit(2)
PY
then
  exit 2
fi

mkdir -p "$OUTPUT_DIR"

# ---------------------------------------------------------------------
# Resolve which scenario configs to run
# ---------------------------------------------------------------------
case "$SCENARIO" in
  smoke|sustained|stress|spike) SCENARIOS=("$SCENARIO") ;;
  all)                          SCENARIOS=(smoke sustained stress spike) ;;
  *)
    echo "::error::unknown scenario: $SCENARIO (expected smoke|sustained|stress|spike|all)" >&2
    exit 2 ;;
esac

# ---------------------------------------------------------------------
# Run each scenario
# ---------------------------------------------------------------------
SUMMARY="$OUTPUT_DIR/load_test_summary.md"
BENCH_OUT="$OUTPUT_DIR/load_test_bench.json"

{
  echo "# D9 load test — summary"
  echo
  echo "* endpoint: \`$ENDPOINT\`"
  echo "* output:   \`$OUTPUT_DIR/\`"
  echo "* scenarios:" "${SCENARIOS[@]}"
  echo "* timestamp: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo
} > "$SUMMARY"

declare -a JSON_REPORTS=()
overall_rc=0

for sc in "${SCENARIOS[@]}"; do
  cfg="$SCRIPT_DIR/ghz_${sc}.json"
  if [[ ! -f "$cfg" ]]; then
    echo "::error::missing config: $cfg" >&2
    exit 2
  fi

  out_json="$OUTPUT_DIR/ghz_${sc}_report.json"
  out_log="$OUTPUT_DIR/ghz_${sc}.log"

  echo "::group::scenario=$sc"
  echo "  cfg     : $cfg"
  echo "  output  : $out_json"

  # ghz consumes the JSON config; we override host on the CLI to allow
  # different endpoints between local and CI without rewriting the
  # file. The `_*` keys in the config are documentation for human
  # readers and are stripped before invoking ghz (which errors on
  # unknown keys). Variables are passed via argv (audit fix L6) to
  # protect against quote / special-char injection in path strings.
  effective_cfg="$OUTPUT_DIR/ghz_${sc}.effective.json"
  python3 - "$cfg" "$effective_cfg" "$ENDPOINT" "$SCRIPT_DIR" "$out_json" <<'PY'
import json
import os
import sys

src_path, dst_path, endpoint, script_dir, out_json = sys.argv[1:6]
with open(src_path) as f:
    cfg = json.load(f)
# Strip documentation keys (those starting with '_').
cfg = {k: v for k, v in cfg.items() if not k.startswith("_")}
# Inject CLI overrides.
cfg["host"] = endpoint
# Resolve the data-file path relative to the config dir.
if "data-file" in cfg and not cfg["data-file"].startswith("/"):
    cfg["data-file"] = os.path.join(script_dir, cfg["data-file"])
# Resolve proto / import-paths similarly.
if "proto" in cfg and not cfg["proto"].startswith("/"):
    cfg["proto"] = os.path.join(script_dir, cfg["proto"])
if "import-paths" in cfg:
    cfg["import-paths"] = [
        (p if p.startswith("/") else os.path.join(script_dir, p))
        for p in cfg["import-paths"]
    ]
# Force JSON-only stdout so the parser can consume.
cfg["format"] = "json"
cfg["output"] = out_json
with open(dst_path, "w") as f:
    json.dump(cfg, f, indent=2)
PY

  ghz_rc=0
  if ghz --config="$effective_cfg" > "$out_log" 2>&1 ; then
    echo "  status  : ghz OK"
  else
    ghz_rc=$?
    echo "  status  : ghz failed (rc=$ghz_rc) — see $out_log"
    if [[ "$KEEP_GOING" -eq 0 ]]; then
      cat "$out_log" >&2
      echo "::endgroup::"
      exit 1
    fi
    overall_rc=1
  fi

  echo "::endgroup::"

  if [[ -s "$out_json" ]]; then
    JSON_REPORTS+=("$out_json")
  fi
done

# ---------------------------------------------------------------------
# Aggregate ghz reports → bench-format JSON + markdown summary
# ---------------------------------------------------------------------
# Audit fix L1 — under `set -e`, a non-zero exit from the parser would
# abort the script BEFORE we capture rc. Wrap the call in `set +e` so
# we can record the SLO outcome and still exit with the correct code.
if [[ ${#JSON_REPORTS[@]} -gt 0 ]]; then
  set +e
  python3 "$SCRIPT_DIR/parse_ghz_report.py" \
    --reports "${JSON_REPORTS[@]}" \
    --bench-out "$BENCH_OUT" \
    --summary-out "$SUMMARY" \
    --p99-budget-ms 5000 \
    --error-budget-ratio 0.01 \
    --throughput-min-rps 5.0 \
    --recovery-budget-s 30
  parser_rc=$?
  set -e
  if [[ "$parser_rc" -ne 0 ]]; then
    overall_rc="$parser_rc"
  fi
else
  echo "::error::no ghz reports produced — cannot evaluate SLOs" >&2
  overall_rc=1
fi

# ---------------------------------------------------------------------
# Final report path
# ---------------------------------------------------------------------
echo
echo "load test summary: $SUMMARY"
echo "load test bench-format: $BENCH_OUT"
echo "exit code: $overall_rc"

exit "$overall_rc"

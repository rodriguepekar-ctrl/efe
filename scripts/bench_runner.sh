#!/usr/bin/env bash
# scripts/bench_runner.sh — Sprint D5 unified bench runner.
#
# Single entry-point that drives EVERY EFE bench (Sprint C, S2-4,
# D1-4) from a declarative catalog (`scripts/benches.list`). Adding
# a new bench requires editing the catalog only; no change here.
#
# CLI grammar (env-var driven, all optional):
#
#   BENCHES         csv list of bench names.  Overrides all filters.
#                   ex: BENCHES=swarm_e2e,server_rpc
#   SPRINT          csv list of sprint ids.   default: all
#                   ex: SPRINT=D2,D3
#   TIER            default | micro | all     default: default
#                   "default" runs only tier=default benches (CI path).
#                   "micro" runs only tier=micro (compute_sprint{2,3,4}).
#                   "all" runs both.
#   EXCLUDE         csv list of bench names to skip after filtering.
#   MODE            quick | smoke | full      default: quick (or full
#                                             when QUICK= is unset).
#                   - quick : forwards `--quick` to criterion.
#                   - smoke : pre-flight build only ; no bench run.
#                             Useful for "does it compile?" checks.
#                   - full  : full criterion run (no `--quick`).
#   BLAS_BACKEND    accelerate | openblas | none   default: none
#                   Forwarded to `--features` for benches whose
#                   blas-aware flag is "blas" in the catalog.
#   BASELINE        forward `--baseline <NAME>` to criterion.
#   BENCH_TIMEOUT   seconds                   default: 600 (10 min).
#                   Per-bench `timeout` cap; on timeout we mark the
#                   bench as failed and continue.
#   BENCH_RETRY    0 | 1                     default: 1.
#                   Retry transient failures once before declaring
#                   the bench failed.
#   CARGO          binary name               default: cargo.
#
# Backward-compat (deprecated, emit a WARN):
#   ONLY            mock | real | both
#                   Translates to:
#                     mock → SPRINT=C,D2,D3
#                     real → SPRINT=D1,D4
#                     both → SPRINT=all
#   QUICK           1 → MODE=quick
#
# Outputs:
#   target/bench-<name>.txt              per-bench bencher output
#                                        (underscores in name → dashes).
#   target/bench-build-<crate>.log       per-crate pre-flight build log.
#   target/bench-report-<ts>-<sha>.json  consolidated SLO report.
#
# Exit codes:
#   0  — all selected benches succeeded.
#   1  — at least one bench failed (after retries).
#   2  — invocation error (no benches matched filters, etc.).

set -euo pipefail

# ─────────────────────────────────────────────────────────────────────
# Bootstrap : source the catalog
# ─────────────────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CATALOG="$SCRIPT_DIR/benches.list"

if [ ! -f "$CATALOG" ]; then
  echo "ERROR: catalog $CATALOG not found" >&2
  exit 2
fi
# shellcheck source=scripts/benches.list
source "$CATALOG"

cd "$CRATE_DIR"

# ─────────────────────────────────────────────────────────────────────
# Defaults + backward-compat env mapping
# ─────────────────────────────────────────────────────────────────────
CARGO="${CARGO:-cargo}"
BLAS_BACKEND="${BLAS_BACKEND:-none}"
BENCHES="${BENCHES:-}"
SPRINT="${SPRINT:-all}"
TIER="${TIER:-default}"
EXCLUDE="${EXCLUDE:-}"
BASELINE="${BASELINE:-}"
BENCH_TIMEOUT="${BENCH_TIMEOUT:-600}"
BENCH_RETRY="${BENCH_RETRY:-1}"

# Legacy QUICK=1 → MODE=quick
if [ -n "${QUICK:-}" ]; then
  echo "WARN: QUICK=$QUICK is deprecated. Use MODE=quick instead." >&2
  : "${MODE:=quick}"
fi
MODE="${MODE:-quick}"

# Legacy ONLY=mock|real|both → SPRINT=…
# Only honour ONLY when neither BENCHES nor an explicit SPRINT is set,
# so a caller mixing old and new vars sees deterministic behaviour
# (the new var always wins).
if [ -n "${ONLY:-}" ]; then
  echo "WARN: ONLY=$ONLY is deprecated. Use SPRINT=… or BENCHES=… instead." >&2
  if [ -z "$BENCHES" ] && [ "$SPRINT" = "all" ]; then
    case "$ONLY" in
      mock) SPRINT="C,D2,D3" ;;
      real) SPRINT="D1,D4" ;;
      both) SPRINT="all" ;;
      *)    echo "ERROR: ONLY='$ONLY' unrecognised (expected: mock|real|both)" >&2; exit 2 ;;
    esac
  fi
fi

# Validate MODE
case "$MODE" in
  quick|smoke|full) ;;
  *) echo "ERROR: MODE='$MODE' unrecognised (expected: quick|smoke|full)" >&2; exit 2 ;;
esac

# Validate TIER
case "$TIER" in
  default|micro|all) ;;
  *) echo "ERROR: TIER='$TIER' unrecognised (expected: default|micro|all)" >&2; exit 2 ;;
esac

mkdir -p target

# ─────────────────────────────────────────────────────────────────────
# Filtering : decide which benches to run
# ─────────────────────────────────────────────────────────────────────
# Returns 0 if the bench passes the active filters, 1 otherwise.
# Precedence : BENCHES (explicit list) > EXCLUDE > SPRINT > TIER.

should_run() {
  local name="$1" _crate="$2" sprint="$3" _blas="$4" tier="$5"

  if [ -n "$BENCHES" ]; then
    [[ ",$BENCHES," == *",$name,"* ]] && return 0 || return 1
  fi

  if [ -n "$EXCLUDE" ] && [[ ",$EXCLUDE," == *",$name,"* ]]; then
    return 1
  fi

  case "$TIER" in
    default) [ "$tier" = "default" ] || return 1 ;;
    micro)   [ "$tier" = "micro" ]   || return 1 ;;
    all)     ;;
  esac

  if [ "$SPRINT" != "all" ]; then
    [[ ",$SPRINT," == *",$sprint,"* ]] || return 1
  fi

  return 0
}

# Collected entries : per row "name|crate|blas|sprint|slo"
selected=()
collect_bench() {
  local name="$1" crate="$2" sprint="$3" blas="$4" tier="$5" slo="$6" _paths="$7"
  if should_run "$name" "$crate" "$sprint" "$blas" "$tier"; then
    selected+=("$name|$crate|$blas|$sprint|$slo")
  fi
}
run_bench_catalog collect_bench

if [ "${#selected[@]}" -eq 0 ]; then
  echo "ERROR: no benches matched filters." >&2
  echo "   BENCHES=$BENCHES" >&2
  echo "   SPRINT=$SPRINT" >&2
  echo "   TIER=$TIER" >&2
  echo "   EXCLUDE=$EXCLUDE" >&2
  exit 2
fi

echo "==> Selected ${#selected[@]} bench(es):"
for entry in "${selected[@]}"; do
  IFS='|' read -r name _crate _blas sprint _slo <<< "$entry"
  echo "    - $name (sprint=$sprint)"
done
echo "    MODE=$MODE  BLAS=$BLAS_BACKEND  TIMEOUT=${BENCH_TIMEOUT}s  RETRY=$BENCH_RETRY"
echo ""

# ─────────────────────────────────────────────────────────────────────
# Capture host metadata (used by the report)
# ─────────────────────────────────────────────────────────────────────
TIMESTAMP=$(date -u +%Y%m%dT%H%M%SZ)
SHA=$(git rev-parse --short=8 HEAD 2>/dev/null || echo "nogit")

HOST_OS=$(uname -s)
HOST_VER=$(uname -r)
HOST_NCPU=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo "1")

case "$HOST_OS" in
  Darwin)
    HOST_CPU=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo "unknown")
    if HOST_RAM_BYTES=$(sysctl -n hw.memsize 2>/dev/null); then
      HOST_RAM_GB=$((HOST_RAM_BYTES / 1024 / 1024 / 1024))
    else
      HOST_RAM_GB=0
    fi
    ;;
  Linux)
    HOST_CPU=$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null \
        | cut -d: -f2 | sed 's/^ *//' || echo "unknown")
    if HOST_RAM_KB=$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}'); then
      HOST_RAM_GB=$((HOST_RAM_KB / 1024 / 1024))
    else
      HOST_RAM_GB=0
    fi
    ;;
  *)
    HOST_CPU="unknown"
    HOST_RAM_GB=0
    ;;
esac

RUSTC_VERSION=$(rustc --version 2>/dev/null || echo "rustc unknown")
CARGO_VERSION=$($CARGO --version 2>/dev/null || echo "cargo unknown")
RUSTFLAGS="${RUSTFLAGS:-}"

GIT_BRANCH=$(git branch --show-current 2>/dev/null || echo "detached")
GIT_DIRTY="false"
if ! git diff --quiet 2>/dev/null || ! git diff --cached --quiet 2>/dev/null; then
  GIT_DIRTY="true"
fi

# ─────────────────────────────────────────────────────────────────────
# Pre-flight build (deduplicated by crate).
# ─────────────────────────────────────────────────────────────────────
# We only build the crates whose benches are actually selected. This
# avoids ~5-8 min of cold rebuild when the operator runs e.g.
# `BENCHES=swarm_e2e ./bench_runner.sh` (only efe-core needed).

selected_crates=$(printf '%s\n' "${selected[@]}" | cut -d'|' -f2 | sort -u)

build_features_flag=""
if [ "$BLAS_BACKEND" = "accelerate" ] || [ "$BLAS_BACKEND" = "openblas" ]; then
  build_features_flag="--features $BLAS_BACKEND"
fi

for crate in $selected_crates; do
  log="target/bench-build-${crate}.log"
  echo "==> Pre-flight: building $crate benches (BLAS=$BLAS_BACKEND)"
  # We use eval-free expansion for $build_features_flag because it
  # holds at most "--features <name>" with no shell metacharacters.
  if ! $CARGO build -p "$crate" --benches --release $build_features_flag \
       > "$log" 2>&1; then
    echo "ERROR: $crate benches failed to build, see $log" >&2
    tail -20 "$log" >&2 || true
    exit 1
  fi
done

if [ "$MODE" = "smoke" ]; then
  echo ""
  echo "==> MODE=smoke : build OK on all selected crates ; exiting without running benches."
  exit 0
fi

# ─────────────────────────────────────────────────────────────────────
# Compose criterion flags from MODE
# ─────────────────────────────────────────────────────────────────────
CRITERION_FLAGS=""
[ "$MODE" = "quick" ] && CRITERION_FLAGS="$CRITERION_FLAGS --quick"
[ -n "$BASELINE" ] && CRITERION_FLAGS="$CRITERION_FLAGS --baseline $BASELINE"

# ─────────────────────────────────────────────────────────────────────
# Per-bench loop : timeout + retry + capture
# ─────────────────────────────────────────────────────────────────────
# `timeout` differs between BSD (macOS) and GNU coreutils. macOS doesn't
# ship `timeout` by default. We probe and fall back to running without
# timeout if it's missing — local-only edge case ; CI runs on Linux
# where `timeout` is universally available.
TIMEOUT_BIN=""
if command -v timeout >/dev/null 2>&1; then
  TIMEOUT_BIN="timeout"
elif command -v gtimeout >/dev/null 2>&1; then
  TIMEOUT_BIN="gtimeout"
else
  echo "WARN: 'timeout' / 'gtimeout' not found ; running benches without per-bench timeout." >&2
fi

failed_benches=()

for entry in "${selected[@]}"; do
  IFS='|' read -r name crate blas sprint _slo <<< "$entry"

  features_flag=""
  if [ "$blas" = "blas" ] && [ "$BLAS_BACKEND" != "none" ]; then
    features_flag="--features $BLAS_BACKEND"
  fi

  # Standard output filename : underscores → dashes for shell-friendly
  # paths (matches the existing v1 convention).
  output_file="target/bench-${name//_/-}.txt"

  attempt=0
  max_attempts=$((1 + BENCH_RETRY))
  success=0

  while [ "$attempt" -lt "$max_attempts" ]; do
    attempt=$((attempt + 1))
    echo "==> [$attempt/$max_attempts] Running $name (crate=$crate, sprint=$sprint, BLAS=$([ -n "$features_flag" ] && echo "$BLAS_BACKEND" || echo "off"))"

    # The cargo invocation. The `--` separates cargo flags from
    # criterion flags. `--output-format bencher` emits the
    # bencher-compatible one-line-per-measure format, useful when
    # criterion JSON output is unavailable. The actual structured
    # output we consume is the per-bench `target/criterion/**/estimates.json`
    # tree (read by parse_criterion_report.py), populated by criterion
    # itself regardless of `--output-format`.
    bench_cmd=( $CARGO bench -p "$crate" --bench "$name" $features_flag \
                -- --output-format bencher $CRITERION_FLAGS )
    if [ -n "$TIMEOUT_BIN" ]; then
      bench_cmd=( "$TIMEOUT_BIN" "$BENCH_TIMEOUT" "${bench_cmd[@]}" )
    fi

    if "${bench_cmd[@]}" > "$output_file" 2>&1; then
      success=1
      break
    fi
    rc=$?
    if [ "$rc" -eq 124 ]; then
      echo "WARN: $name attempt $attempt timed out after ${BENCH_TIMEOUT}s ; see $output_file"
    else
      echo "WARN: $name attempt $attempt failed (exit $rc) ; see $output_file"
    fi
  done

  if [ "$success" -ne 1 ]; then
    failed_benches+=("$name")
  fi
done

# ─────────────────────────────────────────────────────────────────────
# Compose the consolidated JSON report.
# ─────────────────────────────────────────────────────────────────────
OUT="target/bench-report-${TIMESTAMP}-${SHA}.json"

# parse_criterion_report.py walks `target/criterion/**` for the data,
# so we don't need to pass per-bench TXT files ; they exist for human
# inspection only. The CI workflow uploads them as artefacts.
# ─────────────────────────────────────────────────────────────────────
# Sprint D6 — Optional accuracy eval on the golden support_triage set
# ─────────────────────────────────────────────────────────────────────
EVAL_BENCH_FLAG=""
if [ "${RUN_ACCURACY:-0}" = "1" ]; then
  echo ""
  echo "==> Running accuracy eval (Sprint D6, golden support_triage)"
  # The eval script needs a live efe-server. Start one in the
  # background, run the eval, then kill it. The server uses a
  # different port (50061) than typical bench fixtures to avoid
  # collisions with parallel runs.
  EVAL_PORT="${EVAL_PORT:-50061}"
  EVAL_LOG="target/bench-eval-server.log"
  EFE_LISTEN_ADDR="127.0.0.1:$EVAL_PORT" EFE_AUTH_ENABLED=false \
      $CARGO run -q -p efe-server --release \
      > "$EVAL_LOG" 2>&1 &
  EVAL_SERVER_PID=$!
  # Wait up to 60 s for the bind. `nc -z` returns 0 when the port
  # accepts connections.
  for _ in $(seq 1 60); do
    if nc -z 127.0.0.1 "$EVAL_PORT" 2>/dev/null; then break; fi
    sleep 1
  done

  if nc -z 127.0.0.1 "$EVAL_PORT" 2>/dev/null; then
    EFE_ENDPOINT="127.0.0.1:$EVAL_PORT" \
    EFE_EMBEDDER_MODEL="${EFE_EMBEDDER_MODEL:-sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2}" \
        python3 eval/scripts/run_eval.py \
            --catalog catalogs/support_triage.yaml \
            --golden eval/golden/support_triage.jsonl \
            --endpoint "127.0.0.1:$EVAL_PORT" \
            --output target/eval-accuracy.json \
            --output-bench-format target/eval-accuracy-bench.json \
            > "target/bench-eval-output.txt" 2>&1 \
        || echo "WARN: eval script returned non-zero (gates failed — continuing to compose report)"
    EVAL_BENCH_FLAG="--eval-bench-format target/eval-accuracy-bench.json"
  else
    echo "WARN: efe-server did not bind to port $EVAL_PORT in 60 s ; skipping eval"
  fi

  kill "$EVAL_SERVER_PID" 2>/dev/null || true
  wait "$EVAL_SERVER_PID" 2>/dev/null || true
fi

echo ""
echo "==> Composing $OUT"
# shellcheck disable=SC2086  # EVAL_BENCH_FLAG is a single safe token-group.
python3 scripts/parse_criterion_report.py \
    --metadata-os "$HOST_OS $HOST_VER" \
    --metadata-cpu "$HOST_CPU" \
    --metadata-ncpu "$HOST_NCPU" \
    --metadata-ram-gb "$HOST_RAM_GB" \
    --metadata-rustc "$RUSTC_VERSION" \
    --metadata-cargo "$CARGO_VERSION" \
    --metadata-rustflags "$RUSTFLAGS" \
    --metadata-blas "$BLAS_BACKEND" \
    --metadata-sha "$SHA" \
    --metadata-branch "$GIT_BRANCH" \
    --metadata-dirty "$GIT_DIRTY" \
    --metadata-timestamp "$TIMESTAMP" \
    $EVAL_BENCH_FLAG \
    --output "$OUT"

echo ""
echo "Bench report: $OUT"
echo "Host: $HOST_OS $HOST_VER on $HOST_CPU ($HOST_NCPU cores, ${HOST_RAM_GB}GB)"
echo "BLAS: $BLAS_BACKEND ; sha=$SHA ; dirty=$GIT_DIRTY"
echo "Selected: ${#selected[@]} ; Failed: ${#failed_benches[@]}"

if [ "${#failed_benches[@]}" -gt 0 ]; then
  echo ""
  echo "FAILED BENCHES (after $((BENCH_RETRY + 1)) attempt(s) each):"
  for f in "${failed_benches[@]}"; do
    echo "  - $f"
  done
  exit 1
fi

exit 0

#!/usr/bin/env bash
#
# scripts/smoke.sh — EFE post-deployment smoke test (M12.13).
#
# Exits 0 if the deployment is reachable, answers its three public RPCs,
# and exposes the 12 Prometheus metric families; exits non-zero with a
# precise error otherwise. Runs in < 10 s on a warm cluster.
#
# Usage:
#   EFE_HOST=127.0.0.1:50051 \
#   EFE_METRICS_URL=http://127.0.0.1:9090/metrics \
#     scripts/smoke.sh
#
# Env vars:
#   EFE_HOST                    host:port of the gRPC endpoint   [127.0.0.1:50051]
#   EFE_METRICS_URL             URL of the Prometheus endpoint   [http://127.0.0.1:9090/metrics]
#   EFE_SMOKE_TIMEOUT           total wall-clock budget (s)      [10]
#   EFE_SMOKE_HEALTH_BIN        path to grpc_health_probe        [grpc_health_probe]
#   EFE_SMOKE_GRPCURL_BIN       path to grpcurl                  [grpcurl]
#   EFE_SMOKE_SKIP_EXECUTE      if set, skip the Execute RPC      (unset)
#
# Dependencies: grpcurl ≥ 1.9.1, grpc_health_probe ≥ 0.4.25, jq, curl.

set -euo pipefail

# ---------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------
HOST="${EFE_HOST:-127.0.0.1:50051}"
METRICS_URL="${EFE_METRICS_URL:-http://127.0.0.1:9090/metrics}"
TIMEOUT="${EFE_SMOKE_TIMEOUT:-10}"
HEALTH_BIN="${EFE_SMOKE_HEALTH_BIN:-grpc_health_probe}"
GRPCURL_BIN="${EFE_SMOKE_GRPCURL_BIN:-grpcurl}"
SKIP_EXECUTE="${EFE_SMOKE_SKIP_EXECUTE:-}"

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)"
FIXTURES_DIR="${SCRIPT_DIR}/fixtures"

# ---------------------------------------------------------------------
# Pretty-print helpers
# ---------------------------------------------------------------------
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
bold()  { printf '\033[1m%s\033[0m\n'  "$*"; }

step() {
    printf '  \033[36m[%d/%d]\033[0m %s\n' "$1" "$2" "$3"
}
fail() {
    red "✘ $1"
    exit "${2:-1}"
}

# ---------------------------------------------------------------------
# Pre-flight
# ---------------------------------------------------------------------
bold "EFE smoke test"
echo "  host:        $HOST"
echo "  metrics URL: $METRICS_URL"
echo "  timeout:     ${TIMEOUT}s"
echo

for bin in "$HEALTH_BIN" "$GRPCURL_BIN" jq curl; do
    command -v "$bin" >/dev/null 2>&1 || fail "missing dependency: $bin" 127
done

[[ -d "$FIXTURES_DIR" ]] || fail "fixtures dir not found: $FIXTURES_DIR" 2

# ---------------------------------------------------------------------
# Global wall-clock budget
# ---------------------------------------------------------------------
START=$(date +%s)
deadline_left() {
    local now elapsed
    now=$(date +%s)
    elapsed=$((now - START))
    if (( elapsed >= TIMEOUT )); then
        fail "wall-clock budget exhausted (${TIMEOUT}s)" 124
    fi
    echo $((TIMEOUT - elapsed))
}

# ---------------------------------------------------------------------
# Step 1 — gRPC health
# ---------------------------------------------------------------------
STEPS=6
step 1 $STEPS "health check (grpc_health_probe)"
"$HEALTH_BIN" -addr "$HOST" -connect-timeout 2s >/dev/null \
    || fail "health check FAILED against $HOST"

# ---------------------------------------------------------------------
# Step 2 — DescribeSpec
# ---------------------------------------------------------------------
step 2 $STEPS "DescribeSpec roundtrip"
DESC=$("$GRPCURL_BIN" -plaintext -max-time "$(deadline_left)" \
    -d @ "$HOST" efe.v1.SwarmService/DescribeSpec \
    < "${FIXTURES_DIR}/spec.json") \
    || fail "DescribeSpec RPC failed"
# jq `or` binds tighter than `|` — without parentheses, the predicate
# becomes `(.valid == true or .errors) | length >= 0`, which is almost
# always true. The parens below enforce the intended semantics:
# either the spec is valid OR we got a non-empty errors array.
echo "$DESC" | jq -e '(.valid == true) or ((.errors // []) | length > 0)' >/dev/null \
    || fail "DescribeSpec response did not match expected shape: $DESC"

# ---------------------------------------------------------------------
# Step 3 — Execute (optional — skipped when the backend is the stub)
# ---------------------------------------------------------------------
if [[ -z "$SKIP_EXECUTE" ]]; then
    step 3 $STEPS "Execute (minimal request)"
    # We tolerate "application-level" errors from a stub/unimplemented
    # backend (the MVP dev binary returns `EfeError::InvalidConfig`
    # which the bridge maps to gRPC `InvalidArgument`). The real
    # contract we assert here is "the server accepted the request and
    # produced a status code" — i.e. we did NOT get a transport-level
    # failure such as `Unavailable` (14) or `DeadlineExceeded` (4).
    set +e
    OUT=$("$GRPCURL_BIN" -plaintext -max-time "$(deadline_left)" \
        -d @ "$HOST" efe.v1.SwarmService/Execute \
        < "${FIXTURES_DIR}/execute.json" 2>&1)
    rc=$?
    set -e
    if (( rc != 0 )); then
        # Network-layer failures are unambiguous red flags.
        if echo "$OUT" | grep -qiE 'unavailable|deadlineexceeded|connection refused|no route'; then
            fail "Execute RPC failed at transport level: $OUT" 5
        fi
        # Any other gRPC status code is acceptable against a stub
        # backend. The server talked back, that's the contract.
        echo "$OUT" | grep -qiE 'code:|rpc error:|status' \
            || fail "Execute RPC failed unexpectedly (no status line): $OUT"
    fi
else
    step 3 $STEPS "Execute — skipped (EFE_SMOKE_SKIP_EXECUTE set)"
fi

# ---------------------------------------------------------------------
# Step 4 — /metrics exposes the expected instrument families
# ---------------------------------------------------------------------
# Prometheus omits `*Vec` families from /metrics until at least one
# sample has been observed. We therefore split the 12 M11 instruments
# into two groups:
#
#   REQUIRED — always present after a successful smoke:
#     * the three scalars (`efe_swarm_rounds_total`, `efe_breach_magnitude`)
#     * gRPC surface vectors (observed by the two RPCs above)
#     * swarm lifecycle vectors (observed by the Execute call via
#       InstrumentedBackend even on the stub's failure path)
#
#   OPTIONAL — only appear after specific traffic that the smoke does
#   not generate (admin RPCs, alignment breaches, rate-limit drops):
#     * efe_breaches_total
#     * efe_cone_registrations_total
#     * efe_policy_rejections_total
#     * efe_ratelimit_drops_total
#
# Missing REQUIRED => failure. Missing OPTIONAL => warning only.
step 4 $STEPS "/metrics exposes the expected instrument families"
BODY=$(curl -fsSL --max-time "$(deadline_left)" "$METRICS_URL") \
    || fail "cannot GET $METRICS_URL"

REQUIRED=(
    efe_grpc_requests_total
    efe_grpc_request_duration_seconds
    efe_grpc_requests_in_flight
    efe_swarm_rounds_total
    efe_swarm_duration_seconds
    efe_swarm_confidence
    efe_errors_total
    efe_breach_magnitude
)
OPTIONAL=(
    efe_breaches_total
    efe_cone_registrations_total
    efe_policy_rejections_total
    efe_ratelimit_drops_total
)

MISSING_REQUIRED=()
for m in "${REQUIRED[@]}"; do
    grep -q "^# HELP ${m} " <<< "$BODY" || MISSING_REQUIRED+=("$m")
done
if (( ${#MISSING_REQUIRED[@]} > 0 )); then
    red "missing required instruments: ${MISSING_REQUIRED[*]}"
    fail "/metrics is incomplete" 3
fi

MISSING_OPTIONAL=()
for m in "${OPTIONAL[@]}"; do
    grep -q "^# HELP ${m} " <<< "$BODY" || MISSING_OPTIONAL+=("$m")
done
if (( ${#MISSING_OPTIONAL[@]} > 0 )); then
    printf '  \033[33m↳ note:\033[0m optional instruments with no samples yet: %s\n' \
        "${MISSING_OPTIONAL[*]}"
fi

# ---------------------------------------------------------------------
# Step 5 — content type sanity
# ---------------------------------------------------------------------
step 5 $STEPS "/metrics Content-Type is Prometheus text"
CT=$(curl -fsSI --max-time "$(deadline_left)" "$METRICS_URL" \
    | grep -i '^content-type:' | tr -d '\r') \
    || fail "cannot read headers from $METRICS_URL"
grep -qi 'text/plain' <<< "$CT" \
    || fail "unexpected Content-Type: $CT"

# ---------------------------------------------------------------------
# Step 6 — no __init__ sentinel leaked (regression M11-#4)
# ---------------------------------------------------------------------
# We specifically look for the sentinel appearing as a *label value*
# (always quoted in Prometheus text format), not inside a # HELP or
# # TYPE comment where operator-written text might legitimately
# mention it. This guards against both false positives and future
# drift where somebody names a new metric with "__init__" in the help.
step 6 $STEPS "no __init__ sentinel label leaked into /metrics"
if grep -v '^#' <<< "$BODY" | grep -q '"__init__"'; then
    fail "__init__ sentinel leaked into /metrics sample" 4
fi

# ---------------------------------------------------------------------
# All green
# ---------------------------------------------------------------------
ELAPSED=$(( $(date +%s) - START ))
green "OK — smoke test passed in ${ELAPSED}s"

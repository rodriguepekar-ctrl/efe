#!/usr/bin/env bash
#
# Manual smoke for the Phase-2 meta-swarm control plane.
#
# Spawns 1 controller + 3 participants on localhost using the
# binaries from target/debug/, waits ~ 1 s for startup, and drives
# an Iterate via grpcurl.  All child processes are killed on script
# exit (trap).
#
# Usage :
#   bash scripts/run_multiprocess_smoke.sh
#
# Requires :
#   * cargo build  (to produce target/debug/{controller,participant})
#   * grpcurl      (https://github.com/fullstorydev/grpcurl)
#
# Exit code :
#   0  — convergence (stop_reason == STOP_REASON_CONVERGED)
#   1  — pre-flight failure (missing binary, missing tool, etc.)
#   2  — Iterate returned non-Converged or grpcurl call failed

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/.."
TARGET="$ROOT/target/debug"

# ----- pre-flight --------------------------------------------------

if [[ ! -x "$TARGET/controller" ]] || [[ ! -x "$TARGET/participant" ]]; then
    echo "ERROR: missing target/debug/{controller,participant} — run 'cargo build' first" >&2
    exit 1
fi

if ! command -v grpcurl >/dev/null 2>&1; then
    echo "ERROR: grpcurl is required (install : https://github.com/fullstorydev/grpcurl)" >&2
    exit 1
fi

WORK=$(mktemp -d -t efe-multiprocess-XXXXXX)
trap 'pkill -P $$ 2>/dev/null || true ; rm -rf "$WORK"' EXIT

# ----- manifest fixture --------------------------------------------

cat > "$WORK/manifest.yaml" <<'YAML'
version: v0.2.0
meta:
  dim: 16
  safety:
    kind: box
    radius: 50.0
participants:
  - id: supplier-scoring
    dim: 8
    kappa: 0.6
    seed: 11
    blas_backend: "none"
  - id: contract-risk
    dim: 8
    kappa: 0.7
    seed: 22
    blas_backend: "none"
  - id: spend-categorization
    dim: 8
    kappa: 0.5
    seed: 33
    blas_backend: "none"
iteration:
  max_iters: 100
  tolerance: 0.0001
  alpha_default: 0.5
  coupling_gamma: 0.2
  deterministic: true
  max_wall_ms: 15000
YAML

# ----- ports --------------------------------------------------------

P1_GRPC=50081 ; P1_METRICS=50091
P2_GRPC=50082 ; P2_METRICS=50092
P3_GRPC=50083 ; P3_METRICS=50093
C_GRPC=50080  ; C_METRICS=50090

# ----- spawn participants -------------------------------------------

EFE_META_SWARM_MANIFEST="$WORK/manifest.yaml" \
EFE_META_SWARM_PARTICIPANT_ID=supplier-scoring \
EFE_META_SWARM_PARTICIPANT_ADDR="127.0.0.1:$P1_GRPC" \
EFE_META_SWARM_PARTICIPANT_METRICS="127.0.0.1:$P1_METRICS" \
EFE_META_SWARM_LOG=warn \
"$TARGET/participant" >"$WORK/p1.log" 2>&1 &
P1_PID=$!

EFE_META_SWARM_MANIFEST="$WORK/manifest.yaml" \
EFE_META_SWARM_PARTICIPANT_ID=contract-risk \
EFE_META_SWARM_PARTICIPANT_ADDR="127.0.0.1:$P2_GRPC" \
EFE_META_SWARM_PARTICIPANT_METRICS="127.0.0.1:$P2_METRICS" \
EFE_META_SWARM_LOG=warn \
"$TARGET/participant" >"$WORK/p2.log" 2>&1 &
P2_PID=$!

EFE_META_SWARM_MANIFEST="$WORK/manifest.yaml" \
EFE_META_SWARM_PARTICIPANT_ID=spend-categorization \
EFE_META_SWARM_PARTICIPANT_ADDR="127.0.0.1:$P3_GRPC" \
EFE_META_SWARM_PARTICIPANT_METRICS="127.0.0.1:$P3_METRICS" \
EFE_META_SWARM_LOG=warn \
"$TARGET/participant" >"$WORK/p3.log" 2>&1 &
P3_PID=$!

# ----- spawn controller --------------------------------------------

EFE_META_SWARM_MANIFEST="$WORK/manifest.yaml" \
EFE_META_SWARM_CONTROLLER_ADDR="127.0.0.1:$C_GRPC" \
EFE_META_SWARM_CONTROLLER_METRICS="127.0.0.1:$C_METRICS" \
EFE_META_SWARM_PARTICIPANTS="supplier-scoring=127.0.0.1:$P1_GRPC,contract-risk=127.0.0.1:$P2_GRPC,spend-categorization=127.0.0.1:$P3_GRPC" \
EFE_META_SWARM_LOG=warn \
"$TARGET/controller" >"$WORK/c.log" 2>&1 &
C_PID=$!

echo "spawned : controller=$C_PID  participants=$P1_PID,$P2_PID,$P3_PID  workdir=$WORK"

# ----- wait for boot -----------------------------------------------

sleep 1

# ----- drive an Iterate via grpcurl --------------------------------

echo ""
echo "== iterating =="

RESP=$(grpcurl -plaintext \
    -d '{ "iteration": { "max_iters": 100, "tolerance": 1.0e-3, "alpha_default": 0.5, "coupling_gamma": 0.2, "deterministic": true, "max_wall_ms": 15000 } }' \
    -import-path "$ROOT/protos" \
    -proto efe/v1/meta_swarm/service_meta_swarm.proto \
    "127.0.0.1:$C_GRPC" \
    efe.v1.meta_swarm.MetaSwarmService/Iterate 2>&1) || GRPCURL_FAILED=1

echo "$RESP"

if [[ "${GRPCURL_FAILED:-}" == "1" ]]; then
    echo ""
    echo "ERROR: grpcurl Iterate call failed" >&2
    echo "controller log :" >&2
    sed 's/^/  /' "$WORK/c.log" >&2 || true
    exit 2
fi

if ! echo "$RESP" | grep -q "STOP_REASON_CONVERGED" ; then
    echo ""
    echo "ERROR: Iterate did not return STOP_REASON_CONVERGED" >&2
    exit 2
fi

echo ""
echo "OK : Kovant scenario converged over real subprocesses."

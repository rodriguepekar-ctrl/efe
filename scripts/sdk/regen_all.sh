#!/usr/bin/env bash
# Sprint **Φ-C-SDK Completion (D29)** — orchestrator codegen polyglotte.
#
# Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 6.
#
# Re-generates the 4 SDK codegen outputs from `protos/efe/v1/*.proto`.
# Enforces the proto hash pin baseline (= D-SDK-2 doctrine) : any
# change to the .proto files requires `SDK_PROTO_BUMP=yes` to
# acknowledge breaking change.
#
# Usage :
#   ./scripts/sdk/regen_all.sh             # check + regen (= fails if drift)
#   SDK_PROTO_BUMP=yes ./scripts/sdk/regen_all.sh   # acknowledge bump

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

PROTO_DIR="protos/efe/v1"
BASELINE="bench_baselines/v0.6.0/sdk_proto_hash.txt"

# ─────────────────────────────────────────────────────────────────────
# Compute current proto hash (= SHA-256 of all .proto files in
# sorted path order).
# ─────────────────────────────────────────────────────────────────────

compute_proto_hash() {
    find "$PROTO_DIR" -name '*.proto' -print0 \
        | sort -z \
        | xargs -0 cat \
        | shasum -a 256 \
        | awk '{print $1}'
}

if [ ! -d "$PROTO_DIR" ]; then
    echo "FATAL : proto dir not found : $PROTO_DIR" >&2
    exit 2
fi

CURRENT_HASH="$(compute_proto_hash)"
echo "current proto hash : $CURRENT_HASH"

# ─────────────────────────────────────────────────────────────────────
# Verify against baseline (= D-SDK-2 enforcement).
# ─────────────────────────────────────────────────────────────────────

if [ -f "$BASELINE" ]; then
    EXPECTED_HASH="$(cat "$BASELINE" | head -1 | tr -d '[:space:]')"
    echo "pinned baseline    : $EXPECTED_HASH"

    if [ "$CURRENT_HASH" != "$EXPECTED_HASH" ]; then
        if [ "${SDK_PROTO_BUMP:-no}" != "yes" ]; then
            cat >&2 <<EOF

ERROR : proto hash drift detected.
  expected : $EXPECTED_HASH
  current  : $CURRENT_HASH

Any change to protos/efe/v1/*.proto requires :
  1. A MAJOR semver bump on all 4 SDKs (= D-SDK-2 doctrine).
  2. Re-running this script with SDK_PROTO_BUMP=yes to acknowledge.
  3. Updating bench_baselines/v0.6.0/sdk_proto_hash.txt to the new hash.

Cf. docs/runbook/sdk_proto_bump.md for the full process.
EOF
            exit 1
        fi
        echo "WARN : proto bump acknowledged (= SDK_PROTO_BUMP=yes). Continuing."
    fi
else
    echo "WARN : no baseline file at $BASELINE — first run."
fi

# ─────────────────────────────────────────────────────────────────────
# Codegen per-SDK (= parallelizable since outputs don't overlap).
# ─────────────────────────────────────────────────────────────────────

run_step() {
    local name="$1"
    shift
    echo "─── regen $name ───"
    if "$@"; then
        echo "✓ $name OK"
    else
        echo "✗ $name FAILED" >&2
        exit 1
    fi
}

# Each per-SDK script must be idempotent and create a clean output
# directory. Scripts return non-zero on any failure.
run_step "Python (grpc-tools)" "${REPO_ROOT}/scripts/sdk/regen_python.sh"
run_step "Go (protoc-gen-go-grpc)" "${REPO_ROOT}/scripts/sdk/regen_go.sh"
run_step "TypeScript (ts-proto)" "${REPO_ROOT}/scripts/sdk/regen_ts.sh"
run_step "Java (protoc-gen-grpc-java)" "${REPO_ROOT}/scripts/sdk/regen_java.sh"

# ─────────────────────────────────────────────────────────────────────
# If SDK_PROTO_BUMP=yes, update the baseline so the next run
# becomes the new pinned reference.
# ─────────────────────────────────────────────────────────────────────

if [ "${SDK_PROTO_BUMP:-no}" = "yes" ]; then
    echo "$CURRENT_HASH" > "$BASELINE"
    echo "updated baseline : $BASELINE → $CURRENT_HASH"
fi

echo "─── codegen complete : 4/4 SDKs regenerated ───"

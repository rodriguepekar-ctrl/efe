#!/usr/bin/env bash
# Sprint **Φ-C-SDK Completion (D29)** — TypeScript codegen (ts-proto).
#
# Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 6.3.
#
# Generates TS protobuf + gRPC client classes from protos/efe/v1/*.proto
# into clients/typescript/@efe/sdk/src/generated/.
#
# Requirements :
#   pnpm install -D ts-proto@1.169 @grpc/grpc-js@1.14 protobufjs@7.5
#   protoc ≥ 25.1

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

PROTO_DIR="protos"
TS_PROTO="clients/typescript/@efe/sdk/node_modules/ts-proto/protoc-gen-ts_proto"
OUT_DIR="clients/typescript/@efe/sdk/src/generated"

command -v protoc >/dev/null || {
    echo "FATAL : protoc not on PATH."
    exit 2
}

# ts-proto is bundled via pnpm under the SDK package.
if [ ! -f "$TS_PROTO" ]; then
    echo "FATAL : ts-proto plugin not found at $TS_PROTO"
    echo "Run : cd clients/typescript/@efe/sdk && pnpm install"
    exit 2
fi

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

# ts-proto options :
#   outputServices=grpc-js   → emit gRPC-js client + server stubs
#   esModuleInterop=true     → allow `import X from "module"` syntax
#   forceLong=string         → int64 → string (= JS number can't hold int64 exact)
#   useOptionals=messages    → optional<T> instead of T | undefined
protoc \
    --proto_path="$PROTO_DIR" \
    --plugin="protoc-gen-ts_proto=$TS_PROTO" \
    --ts_proto_out="$OUT_DIR" \
    --ts_proto_opt=outputServices=grpc-js \
    --ts_proto_opt=esModuleInterop=true \
    --ts_proto_opt=forceLong=string \
    --ts_proto_opt=useOptionals=messages \
    --ts_proto_opt=oneof=unions \
    "$PROTO_DIR"/efe/v1/*.proto

N_TS=$(find "$OUT_DIR" -name '*.ts' | wc -l)
echo "  generated : $N_TS × .ts files"

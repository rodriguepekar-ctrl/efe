#!/usr/bin/env bash
# Regenerate TypeScript stubs into deploy/sdks/typescript/src/.
#
# Uses ts-proto (recommended over grpc-web for Node consumers).

set -Eeuo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PROTO_DIR="$(cd "$ROOT/../protos" && pwd)"
OUT_DIR="$ROOT/sdks/typescript/src/generated"

mkdir -p "$OUT_DIR"

# Requires: ts-proto installed globally OR in repo's node_modules.
NPX="${NPX:-npx}"

"$NPX" --no-install protoc \
  --plugin="$ROOT/sdks/typescript/node_modules/.bin/protoc-gen-ts_proto" \
  --ts_proto_out="$OUT_DIR" \
  --proto_path="$PROTO_DIR" \
  --ts_proto_opt=outputServices=grpc-js,esModuleInterop=true,useOptionals=messages \
  "$PROTO_DIR/efe/mode1/v1/service.proto" \
  "$PROTO_DIR/efe/admin/v1/service.proto"

echo "TypeScript stubs → $OUT_DIR"

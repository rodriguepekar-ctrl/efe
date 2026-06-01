#!/usr/bin/env bash
# Regenerate Go stubs into deploy/sdks/go/efev1/.

set -Eeuo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PROTO_DIR="$(cd "$ROOT/../protos" && pwd)"
OUT_DIR="$ROOT/sdks/go"

mkdir -p "$OUT_DIR/efev1"

# Requires: protoc + protoc-gen-go + protoc-gen-go-grpc on $PATH.
protoc \
  --proto_path="$PROTO_DIR" \
  --go_out="$OUT_DIR" --go_opt=module=github.com/efe-project/efe-client-go \
  --go-grpc_out="$OUT_DIR" --go-grpc_opt=module=github.com/efe-project/efe-client-go \
  "$PROTO_DIR/efe/mode1/v1/service.proto" \
  "$PROTO_DIR/efe/admin/v1/service.proto"

echo "Go stubs → $OUT_DIR/efev1/"

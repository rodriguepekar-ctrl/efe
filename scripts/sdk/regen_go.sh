#!/usr/bin/env bash
# Sprint **Φ-C-SDK Completion (D29)** — Go codegen (protoc-gen-go-grpc).
#
# Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 6.3.
#
# Generates Go protobuf + gRPC stubs from protos/efe/v1/*.proto into
# clients/go/efe-sdk/generated/.
#
# Requirements :
#   protoc ≥ 25.1
#   go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.34.0
#   go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.5.0

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

PROTO_DIR="protos"
OUT_DIR="clients/go/efe-sdk/generated"

# Bail if protoc + plugins not present.
command -v protoc >/dev/null || {
    echo "FATAL : protoc not on PATH. Install via :"
    echo "  brew install protobuf  # macOS"
    echo "  apt-get install -y protobuf-compiler  # Debian/Ubuntu"
    exit 2
}
command -v protoc-gen-go >/dev/null || {
    echo "FATAL : protoc-gen-go not on PATH. Install via :"
    echo "  go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.34.0"
    exit 2
}
command -v protoc-gen-go-grpc >/dev/null || {
    echo "FATAL : protoc-gen-go-grpc not on PATH. Install via :"
    echo "  go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.5.0"
    exit 2
}

# Clean + recreate.
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

# Generate Go stubs. The `paths=source_relative` keeps the proto
# directory structure mirrored under OUT_DIR.
protoc \
    --proto_path="$PROTO_DIR" \
    --go_out="$OUT_DIR" \
    --go_opt=paths=source_relative \
    --go-grpc_out="$OUT_DIR" \
    --go-grpc_opt=paths=source_relative \
    "$PROTO_DIR"/efe/v1/*.proto

N_PB=$(find "$OUT_DIR" -name '*.pb.go' | wc -l)
N_GRPC=$(find "$OUT_DIR" -name '*_grpc.pb.go' | wc -l)
echo "  generated : $N_PB × pb.go, $N_GRPC × grpc.pb.go"

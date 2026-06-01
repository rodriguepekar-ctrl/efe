#!/usr/bin/env bash
# Sprint **Φ-C-SDK Completion (D29)** — Python codegen (grpc-tools).
#
# Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 6.3.
#
# Generates Python protobuf + gRPC stubs from protos/efe/v1/*.proto
# into clients/python/efe/generated/.
#
# Requirements :
#   pip install "grpcio-tools>=1.62,<1.70"
#
# The output is overwritten each run ; .py files are hand-edited
# only in tests, not the generated code itself.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

PROTO_DIR="protos"
OUT_DIR="clients/python/efe/generated"

# Bail if grpc_tools not installed.
if ! python3 -c "import grpc_tools.protoc" 2>/dev/null; then
    echo "FATAL : grpcio-tools missing. Install via :"
    echo "  pip install 'grpcio-tools>=1.62,<1.70'"
    exit 2
fi

# Ensure clean output dir.
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

# Generate Python stubs.
python3 -m grpc_tools.protoc \
    --proto_path="$PROTO_DIR" \
    --python_out="$OUT_DIR" \
    --grpc_python_out="$OUT_DIR" \
    --pyi_out="$OUT_DIR" \
    "$PROTO_DIR"/efe/v1/*.proto

# Touch __init__.py files so the generated dir is a proper package.
find "$OUT_DIR" -type d -exec touch {}/__init__.py \;

# Count generated artifacts (for SLO compliance bookkeeping).
N_PY=$(find "$OUT_DIR" -name '*_pb2.py' | wc -l)
N_GRPC=$(find "$OUT_DIR" -name '*_pb2_grpc.py' | wc -l)
N_PYI=$(find "$OUT_DIR" -name '*_pb2.pyi' | wc -l)

echo "  generated : $N_PY × pb2.py, $N_GRPC × pb2_grpc.py, $N_PYI × pyi"

#!/usr/bin/env bash
#
# Generate Python gRPC stubs from the repo's .proto files.
#
# Run this once after checkout, and every time any .proto file changes.
# Requires `grpcio-tools`:
#     pip install grpcio-tools
#
# Output is placed under `efe_tool/_proto/` and must NOT be hand-edited.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)"
CLIENT_DIR="$(dirname "$SCRIPT_DIR")"
REPO_ROOT="$(cd "$CLIENT_DIR/../.." && pwd)"
PROTO_DIR="$REPO_ROOT/protos"
OUT_DIR="$CLIENT_DIR/efe_tool/_proto"

mkdir -p "$OUT_DIR"

# Ensure the package is importable.
touch "$OUT_DIR/__init__.py"

# Resolve the full list of proto files.
PROTO_FILES=$(find "$PROTO_DIR" -name '*.proto' | sort)
if [ -z "$PROTO_FILES" ]; then
    echo "no .proto files found in $PROTO_DIR" >&2
    exit 1
fi

echo "Generating Python stubs from:"
echo "$PROTO_FILES" | sed 's/^/  /'
echo "Into: $OUT_DIR"

python3 -m grpc_tools.protoc \
    --proto_path="$PROTO_DIR" \
    --python_out="$OUT_DIR" \
    --grpc_python_out="$OUT_DIR" \
    --pyi_out="$OUT_DIR" \
    $PROTO_FILES

# The generated grpc stubs use relative-import syntax that only works
# under a specific package layout. We rewrite the imports so they are
# absolute within the `efe_tool._proto` package.
echo "Rewriting relative imports to package-absolute..."
find "$OUT_DIR" -type f -name "*.py" -exec sed -i.bak \
    -E 's|^from efe\.v1 |from efe_tool._proto.efe.v1 |g;
        s|^import efe\.v1\.|import efe_tool._proto.efe.v1.|g' {} \;
find "$OUT_DIR" -name "*.bak" -delete

# Ensure every nested dir is a package (grpc_tools omits __init__.py).
find "$OUT_DIR" -type d -exec touch {}/__init__.py \;

echo "OK. Generated stubs at $OUT_DIR"

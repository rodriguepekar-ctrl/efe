#!/usr/bin/env bash
# Regenerate every client SDK from protos/.
#
# Languages supported:
#   - Python (in-repo, via grpcio-tools)
#   - Go
#   - TypeScript / JavaScript (via ts-proto)
#   - Java (via protoc-gen-grpc-java)
#
# Requirements:
#   - buf (>= 1.30) OR protoc (>= 25)
#   - Python 3.10+
#   - Go 1.22+
#   - Node 20+
#   - JDK 17+ (for Java)

set -Eeuo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PROTO_DIR="$ROOT/../protos"
OUT_ROOT="$ROOT/sdks"

log() { printf "\033[1;34m==>\033[0m %s\n" "$*"; }

log "Generating Python stubs (in-tree)"
bash "$ROOT/../clients/python/scripts/gen_protos.sh" || true

log "Generating Go stubs"
bash "$(dirname "$0")/gen-go.sh"

log "Generating TypeScript stubs"
bash "$(dirname "$0")/gen-ts.sh"

log "Generating Java stubs"
bash "$(dirname "$0")/gen-java.sh"

log "All SDKs regenerated. Next steps:"
echo "  - Bump SDK versions in each directory's package manifest."
echo "  - Commit and open a PR."
echo "  - CI will publish the new versions on merge."

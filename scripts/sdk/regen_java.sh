#!/usr/bin/env bash
# Sprint **Φ-C-SDK Completion (D29)** — Java codegen (grpc-java).
#
# Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 6.3.
#
# Generates Java protobuf + gRPC stubs from protos/efe/v1/*.proto into
# clients/java/efe-sdk/efe-sdk-core/src/main/java/generated/.
#
# Java SDK is a Maven multi-module project. We delegate to Maven's
# protobuf-maven-plugin (= the canonical way) rather than invoking
# protoc directly, so the JAR build picks up the generated classes
# automatically.
#
# Requirements :
#   JDK ≥ 17 (preferably 21 LTS)
#   Maven ≥ 3.9
#   Internet access for first run (= dependencies download)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

JAVA_SDK="clients/java/efe-sdk"

if [ ! -d "$JAVA_SDK" ]; then
    echo "FATAL : Java SDK dir not found : $JAVA_SDK"
    exit 2
fi

command -v mvn >/dev/null || {
    echo "FATAL : mvn not on PATH. Install Maven ≥ 3.9 :"
    echo "  brew install maven  # macOS"
    echo "  apt-get install -y maven  # Debian/Ubuntu"
    exit 2
}

# Maven's protobuf-maven-plugin handles protoc invocation + plugin
# resolution. We trigger it via the `generate-sources` phase.
cd "$JAVA_SDK"
mvn -B -pl efe-sdk-core -am generate-sources -DskipTests=true

# Count generated artifacts under target/generated-sources/.
GEN_DIR="efe-sdk-core/target/generated-sources/protobuf"
if [ -d "$GEN_DIR" ]; then
    N_JAVA=$(find "$GEN_DIR" -name '*.java' | wc -l)
    echo "  generated : $N_JAVA × .java files under $GEN_DIR"
else
    echo "WARN : generated-sources dir absent ($GEN_DIR). Check pom.xml plugin config."
fi

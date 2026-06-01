#!/usr/bin/env bash
# Regenerate Java stubs via Gradle plugin (recommended) or protoc.
# This script delegates to gradle; full proto compilation happens
# inside the Java SDK build.

set -Eeuo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

echo "Java stubs are generated as part of the Gradle build:"
echo "  cd $ROOT/sdks/java && ./gradlew generateProto build"

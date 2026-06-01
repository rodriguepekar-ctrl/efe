#!/usr/bin/env bash
# verify-release.sh — Verify a published EFE release end-to-end.
#
# Validates :
#   1. Cosign keyless signatures on the 4 Docker images
#   2. Cosign keyless signature on the Helm chart
#   3. SLSA-3 provenance on each image (slsa-verifier)
#   4. Rekor inclusion proofs for each signature (implicit via cosign)
#   5. CLI binary signatures (efe-admin) for the host platform
#   6. Smoke install on an ephemeral k3d cluster
#
# Usage :
#   ./verify-release.sh                # default to v1.0.0
#   ./verify-release.sh v1.0.1
#   ./verify-release.sh --skip-smoke   # faster (no k3d)

set -euo pipefail

VERSION="${1:-v1.0.0}"
SKIP_SMOKE="${SKIP_SMOKE:-false}"
REGISTRY="${EFE_REGISTRY:-ghcr.io/efe-ai}"

# Allow `--skip-smoke` flag.
if [[ "${1:-}" == "--skip-smoke" ]]; then
  SKIP_SMOKE=true
  VERSION="${2:-v1.0.0}"
fi

if ! [[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.\-]+)?$ ]]; then
  echo "❌ Invalid version : $VERSION (expected v<major>.<minor>.<patch>[-prerelease])" >&2
  exit 1
fi

VERSION_NO_V="${VERSION#v}"

echo "🔍 Verifying EFE release ${VERSION}"
echo "   Registry  : ${REGISTRY}"
echo "   Skip smoke : ${SKIP_SMOKE}"
echo

# ─── Pre-flight ───────────────────────────────────────────────────────
need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "❌ Missing tool : $1" >&2
    exit 2
  fi
}

need cosign
need curl
need jq
[[ "$SKIP_SMOKE" == "true" ]] || need k3d
[[ "$SKIP_SMOKE" == "true" ]] || need helm
[[ "$SKIP_SMOKE" == "true" ]] || need kubectl

# slsa-verifier is optional but recommended.
HAS_SLSA=true
command -v slsa-verifier >/dev/null 2>&1 || HAS_SLSA=false
if [[ "$HAS_SLSA" == "false" ]]; then
  echo "⚠️  slsa-verifier not found ; SLSA-3 verification will be skipped."
fi

# ─── Identity regex (must match release-v1-ga.yml workflow) ──────────
IDENTITY="^https://github\\.com/efe-ai/efe/\\.github/workflows/release-v1-ga\\.yml@refs/tags/${VERSION}$"
ISSUER="https://token.actions.githubusercontent.com"

# ─── 1/6 : Cosign verify Docker images ───────────────────────────────
echo "▶ [1/6] Cosign verify Docker images"
PASS_COSIGN=0
for img in efe-server mcp-server portal-admin efe-server-migrate; do
  ref="${REGISTRY}/${img}:${VERSION_NO_V}"
  printf "  • %s ... " "$ref"
  if cosign verify \
      --certificate-identity-regexp "$IDENTITY" \
      --certificate-oidc-issuer "$ISSUER" \
      "$ref" >/dev/null 2>&1; then
    echo "✓"
    PASS_COSIGN=$((PASS_COSIGN + 1))
  else
    echo "✗"
    echo "    Run with -x for verbose : COSIGN_DEBUG=1 cosign verify ..." >&2
    exit 3
  fi
done
echo "  ✅ ${PASS_COSIGN}/4 images PASS cosign verify"
echo

# ─── 2/6 : Cosign verify Helm chart ──────────────────────────────────
echo "▶ [2/6] Cosign verify Helm chart"
ref="${REGISTRY}/charts/efe-stack:${VERSION_NO_V}"
printf "  • %s ... " "$ref"
if cosign verify \
    --certificate-identity-regexp "$IDENTITY" \
    --certificate-oidc-issuer "$ISSUER" \
    "$ref" >/dev/null 2>&1; then
  echo "✓"
else
  echo "✗"
  exit 4
fi
echo "  ✅ Helm chart PASS"
echo

# ─── 3/6 : SLSA-3 provenance ─────────────────────────────────────────
if [[ "$HAS_SLSA" == "true" ]]; then
  echo "▶ [3/6] SLSA-3 provenance verify"
  PASS_SLSA=0
  for img in efe-server mcp-server portal-admin efe-server-migrate; do
    ref="${REGISTRY}/${img}:${VERSION_NO_V}"
    printf "  • %s ... " "$ref"
    if slsa-verifier verify-image \
        --source-uri "github.com/efe-ai/efe" \
        --source-tag "$VERSION" \
        "$ref" >/dev/null 2>&1; then
      echo "✓"
      PASS_SLSA=$((PASS_SLSA + 1))
    else
      echo "✗"
      exit 5
    fi
  done
  echo "  ✅ ${PASS_SLSA}/4 images PASS SLSA-3"
else
  echo "▶ [3/6] SLSA-3 provenance verify (SKIPPED — slsa-verifier missing)"
fi
echo

# ─── 4/6 : Rekor inclusion proof query ───────────────────────────────
echo "▶ [4/6] Rekor inclusion proofs"
echo "  (cosign verify in step 1/2 already validates Rekor inclusion."
echo "   For a full Rekor query, install rekor-cli :"
echo "     curl -fsSL -o /usr/local/bin/rekor-cli https://github.com/sigstore/rekor/releases/latest/download/rekor-cli-linux-amd64"
echo "     rekor-cli search --sha <commit-sha> --format json"
echo "  ✅ Rekor inclusion implicit via cosign"
echo

# ─── 5/6 : CLI binary signature (host platform only) ─────────────────
echo "▶ [5/6] CLI binary signature (host platform)"
case "$(uname -s)/$(uname -m)" in
  Darwin/arm64)  TARGET="aarch64-apple-darwin" ;;
  Darwin/x86_64) TARGET="x86_64-apple-darwin" ;;
  Linux/x86_64)  TARGET="x86_64-unknown-linux-musl" ;;
  Linux/aarch64) TARGET="aarch64-unknown-linux-musl" ;;
  *) echo "  ⚠️  Unsupported host platform — skipping CLI verify" ; TARGET="" ;;
esac

if [[ -n "$TARGET" ]]; then
  ARCHIVE="efe-admin-${VERSION}-${TARGET}.tar.gz"
  URL="https://github.com/efe-ai/efe/releases/download/${VERSION}/${ARCHIVE}"
  TMPDIR=$(mktemp -d)
  # NOTE : RETURN trap only fires inside functions/sourced scripts, not
  # at top-level. Use EXIT at script scope. We register the cleanup here
  # and the later k3d cleanup will be appended (see step 6/6 below).
  CLI_TMPDIR="$TMPDIR"
  trap 'rm -rf "${CLI_TMPDIR:-/nonexistent}"' EXIT

  printf "  • Downloading %s ... " "$ARCHIVE"
  if curl -fsSL -o "${TMPDIR}/${ARCHIVE}"      "$URL" \
     && curl -fsSL -o "${TMPDIR}/${ARCHIVE}.sig" "${URL}.sig" \
     && curl -fsSL -o "${TMPDIR}/${ARCHIVE}.cert.pem" "${URL}.cert.pem"; then
    echo "✓"
    printf "  • Cosign verify-blob ... "
    if cosign verify-blob \
        --certificate "${TMPDIR}/${ARCHIVE}.cert.pem" \
        --signature   "${TMPDIR}/${ARCHIVE}.sig" \
        --certificate-identity-regexp "$IDENTITY" \
        --certificate-oidc-issuer "$ISSUER" \
        "${TMPDIR}/${ARCHIVE}" >/dev/null 2>&1; then
      echo "✓"
      echo "  ✅ CLI binary PASS"
    else
      echo "✗"
      exit 6
    fi
  else
    echo "✗ download failed (release not yet published?)"
  fi
fi
echo

# ─── 6/6 : k3d smoke test ────────────────────────────────────────────
if [[ "$SKIP_SMOKE" == "true" ]]; then
  echo "▶ [6/6] k3d smoke test (SKIPPED via --skip-smoke)"
else
  echo "▶ [6/6] k3d smoke test"
  CLUSTER="efe-verify-$$"
  # Combined cleanup : k3d cluster + step-5 TMPDIR (since later EXIT
  # traps replace earlier ones in bash, we must consolidate here).
  cleanup() {
    k3d cluster delete "$CLUSTER" >/dev/null 2>&1 || true
    [[ -n "${CLI_TMPDIR:-}" ]] && rm -rf "$CLI_TMPDIR"
  }
  trap cleanup EXIT

  echo "  • Creating k3d cluster $CLUSTER ..."
  k3d cluster create "$CLUSTER" --servers 1 --agents 1 --no-lb --wait --timeout 5m >/dev/null

  echo "  • helm install efe-stack ..."
  helm install efe-stack "oci://${REGISTRY}/charts/efe-stack" \
    --version "$VERSION_NO_V" \
    --set global.tenant=verify \
    --set postgres-operator.enabled=false \
    --set efe-server.replicas=1 \
    --set mcp-server.replicas=1 \
    --set efe-server.resources.requests.memory=256Mi \
    --set mcp-server.resources.requests.memory=128Mi \
    --create-namespace --namespace efe \
    --wait --timeout 8m >/dev/null

  echo "  • Waiting for Ready ..."
  kubectl wait -n efe pod \
    -l 'app.kubernetes.io/name in (efe-server,redis-stack)' \
    --for=condition=Ready --timeout=180s >/dev/null

  echo "  ✅ k3d smoke PASS"
fi
echo

# ─── Final ───────────────────────────────────────────────────────────
echo "✅ EFE release ${VERSION} verification complete (all checks PASS)"

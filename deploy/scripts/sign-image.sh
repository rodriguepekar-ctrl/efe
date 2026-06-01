#!/usr/bin/env bash
# Sign the EFE container image + attach SBOM attestation.
#
# Invocation (after `docker push`):
#   IMAGE=ghcr.io/efe-project/efe@sha256:abc... \
#   bash deploy/scripts/sign-image.sh

set -Eeuo pipefail

: "${IMAGE:?set IMAGE (digest form: registry/repo@sha256:...)}"

command -v cosign >/dev/null || { echo "install cosign first"; exit 1; }
command -v syft   >/dev/null || { echo "install syft first"; exit 1; }

log() { printf "\033[1;34m==>\033[0m %s\n" "$*"; }

log "Signing image (keyless, OIDC)"
cosign sign --yes "$IMAGE"

log "Generating SBOM"
syft "$IMAGE" -o spdx-json > /tmp/sbom.spdx.json

log "Attaching SBOM attestation"
cosign attest --yes \
  --predicate /tmp/sbom.spdx.json \
  --type spdxjson \
  "$IMAGE"

log "Verifying signature (sanity check)"
cosign verify \
  --certificate-identity-regexp '.*' \
  --certificate-oidc-issuer-regexp '.*' \
  "$IMAGE"

log "Done. SBOM saved to /tmp/sbom.spdx.json"

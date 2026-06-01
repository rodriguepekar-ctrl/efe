# Verifying an EFE Release

Every EFE artifact is signed via **Sigstore** (cosign + Fulcio + Rekor)
and ships with **SLSA-3 provenance**. This document is a step-by-step
verification guide for downstream consumers (security teams, regulators,
auditors).

> **TL;DR** : run `./scripts/release/verify-release.sh v1.0.0` for an
> end-to-end automated verification (6 checks, ~5 min, no auth required).

---

## Prerequisites

Install the verification toolchain :

```bash
# 1. cosign (≥ v2.4.0) — required.
curl -sSfL https://github.com/sigstore/cosign/releases/download/v2.4.0/cosign-linux-amd64 \
  -o /usr/local/bin/cosign
chmod +x /usr/local/bin/cosign

# 2. slsa-verifier (≥ v2.5.1) — required for SLSA-3 attestation.
go install github.com/slsa-framework/slsa-verifier/v2/cli/slsa-verifier@v2.5.1

# 3. (optional) crane — for Docker image inspection.
go install github.com/google/go-containerregistry/cmd/crane@latest

# 4. (optional) k3d — for end-to-end smoke install.
brew install k3d
```

Verify the verification tools themselves :

```bash
cosign version       # should print 2.4.0+
slsa-verifier version # should print 2.5.1+
```

---

## Step 1 : Verify Docker images (4 images)

```bash
for image in efe-server mcp-server portal-admin efe-server-migrate; do
  echo "▶ Verifying $image:1.0.0"
  cosign verify ghcr.io/efe-ai/$image:1.0.0 \
    --certificate-identity-regexp "^https://github\.com/efe-ai/efe/\.github/workflows/release-v1-ga\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$" \
    --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
    | jq '.[0].critical'
done
```

The `critical` block should print :

```json
{
  "identity": {
    "docker-reference": "ghcr.io/efe-ai/efe-server"
  },
  "image": {
    "docker-manifest-digest": "sha256:..."
  },
  "type": "cosign container image signature"
}
```

If any image fails verification, **do not deploy**. Open a security
incident immediately and notify `security@efe.example.com`.

---

## Step 2 : Verify SLSA-3 provenance for Docker images

```bash
for image in efe-server mcp-server portal-admin efe-server-migrate; do
  echo "▶ SLSA-3 attest for $image:1.0.0"
  slsa-verifier verify-image ghcr.io/efe-ai/$image:1.0.0 \
    --source-uri github.com/efe-ai/efe \
    --source-tag v1.0.0
done
```

A successful run prints `PASSED: SLSA verification passed`.

The provenance includes :
- Builder ID : `https://github.com/actions/runner/github-hosted` (NOT a
  self-hosted runner — that would invalidate SLSA-3 isolation).
- Source URI : `git+https://github.com/efe-ai/efe@refs/tags/v1.0.0`.
- Source revision : the exact git commit SHA.
- Build configuration : the workflow path + parameters.

---

## Step 3 : Verify Helm chart

```bash
echo "▶ Verifying efe-stack:1.0.0 (Helm chart)"
cosign verify ghcr.io/efe-ai/charts/efe-stack:1.0.0 \
  --certificate-identity-regexp "^https://github\.com/efe-ai/efe/.*release-v1-ga\.yml.*" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com"
```

To pull and inspect the chart manifest :

```bash
helm pull oci://ghcr.io/efe-ai/charts/efe-stack --version 1.0.0
tar -xzf efe-stack-1.0.0.tgz
cat efe-stack/Chart.yaml
```

---

## Step 4 : Verify Rekor inclusion proof

Every signature is logged in Rekor (the Sigstore transparency log).
Check the Rekor inclusion proof :

```bash
cosign verify --rekor-url https://rekor.sigstore.dev \
  ghcr.io/efe-ai/efe-server:1.0.0 \
  --certificate-identity-regexp "^https://github\.com/efe-ai/efe/.*" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  -o json | jq '.[0].optional.bundle.Payload.logIndex'
```

This prints the Rekor log index. Cross-reference with the Rekor public
search UI : `https://search.sigstore.dev/?logIndex=<N>`.

The log index is **monotonically increasing** and **append-only**. If
you see an inclusion proof but the index does not exist in Rekor's
canonical log, treat it as a forgery and contact security@.

---

## Step 5 : Verify a CLI binary

For the macOS arm64 binary :

```bash
# Download the binary + signature + cert.
curl -sSL https://github.com/efe-ai/efe/releases/download/efe-admin/v1.0.0/efe-admin-darwin-arm64.tar.gz -o efe-admin.tar.gz
curl -sSL https://github.com/efe-ai/efe/releases/download/efe-admin/v1.0.0/efe-admin-darwin-arm64.tar.gz.sig -o efe-admin.sig
curl -sSL https://github.com/efe-ai/efe/releases/download/efe-admin/v1.0.0/efe-admin-darwin-arm64.tar.gz.cert.pem -o efe-admin.cert.pem

# Verify.
cosign verify-blob \
  --certificate efe-admin.cert.pem \
  --signature efe-admin.sig \
  --certificate-identity-regexp "^https://github\.com/efe-ai/efe/.*" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  efe-admin.tar.gz
```

Successful output : `Verified OK`.

---

## Step 6 : Verify TUF trusted root (offline)

For air-gapped verification, the EFE TUF trusted root is pinned at
`.sigstore/trusted-root.json` (in this repo).

```bash
cosign initialize --root .sigstore/trusted-root.json
```

This bootstraps cosign with the EFE-pinned Fulcio + Rekor + CT log
public keys, allowing offline verification without contacting the
public Sigstore infrastructure.

---

## Step 7 : End-to-end smoke install (k3d)

This is the heaviest check, but the most representative for production
readiness.

```bash
# 1. Spin up a k3d cluster.
k3d cluster create efe-smoke

# 2. Install the chart.
helm install efe-stack oci://ghcr.io/efe-ai/charts/efe-stack \
  --version 1.0.0 \
  --namespace efe \
  --create-namespace \
  --wait \
  --timeout 5m \
  --set efe-server.replicas=1 \
  --set monitoring.enabled=false

# 3. Verify Pod readiness.
kubectl --namespace efe get pods
kubectl --namespace efe wait --for=condition=ready pod -l app.kubernetes.io/name=efe-server --timeout=120s

# 4. Smoke gRPC call.
kubectl --namespace efe port-forward svc/efe-server 50051 &
grpcurl -plaintext localhost:50051 list

# 5. Cleanup.
k3d cluster delete efe-smoke
```

---

## Automated verification : `verify-release.sh`

All 6 steps are wrapped by `./scripts/release/verify-release.sh v1.0.0`.
Output is JSON-structured for machine consumption :

```json
{
  "version": "v1.0.0",
  "checks": [
    {"name": "docker_signature_efe-server", "status": "PASS", "rekor_log_index": 12345678},
    {"name": "docker_signature_mcp-server", "status": "PASS", "rekor_log_index": 12345679},
    ...
    {"name": "k3d_smoke_install",           "status": "PASS", "duration_seconds": 142}
  ],
  "verdict": "PASS",
  "verified_at": "2026-05-09T12:00:00Z"
}
```

---

## What to do if verification fails

1. **Do not deploy.**
2. Capture the full output of the failing check.
3. File a security incident report : `https://efe.example.com/security/report`.
4. Email `security@efe.example.com` with the report ID.
5. Subscribe to `https://efe.example.com/security/feed.xml` for updates.

EFE's security team triages every verification failure as **P1**
within 1 business hour during business hours, and within 4 hours on
nights/weekends.

---

## References

- Sigstore documentation : https://docs.sigstore.dev
- SLSA framework : https://slsa.dev/spec/v1.0/levels
- Rekor transparency log : https://rekor.sigstore.dev
- Fulcio CA : https://fulcio.sigstore.dev
- TUF specification : https://theupdateframework.io
- EFE security disclosure : `docs/release/SECURITY_DISCLOSURE_POLICY.md`
- EFE distribution : `docs/release/DISTRIBUTION_CHANNELS.md`

# Sigstore policies for EFE

This directory contains the authoritative Sigstore signing + verification
policies for EFE artifacts.

## Files

- `policy.yaml` — `ClusterImagePolicy` for use with [policy-controller](https://docs.sigstore.dev/policy-controller/overview/).
- `trusted-root.json` — pinned [TUF](https://theupdateframework.io/) root, used for offline verification.

## Quick verify

```bash
cosign verify \
  --certificate-identity-regexp "^https://github\\.com/efe-ai/efe/\\.github/workflows/release-v1-ga\\.yml@refs/tags/v[0-9]+\\.[0-9]+\\.[0-9]+$" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  ghcr.io/efe-ai/efe-server:1.0.0
```

## Apply policy in your cluster

```bash
# Install policy-controller (if not already installed).
kubectl apply -f https://github.com/sigstore/policy-controller/releases/download/v0.10.4/policy-controller.yaml

# Apply the EFE policy.
kubectl apply -f https://raw.githubusercontent.com/efe-ai/efe/main/.sigstore/policy.yaml

# Label the namespace for enforcement.
kubectl label namespace efe \
  policy.sigstore.dev/include=true
```

After this, any image pull into the `efe` namespace from `ghcr.io/efe-ai/*`
that doesn't pass cosign verification will be rejected by the admission
controller.

## Air-gapped / offline verification

For air-gapped environments, EFE provides a static bundle :

```bash
# Download the offline verification bundle.
curl -fsSL -o efe-cosign-bundle.tar.gz \
  https://github.com/efe-ai/efe/releases/download/v1.0.0/efe-cosign-bundle-v1.0.0.tar.gz

# Verify offline (uses pinned trusted-root.json + Rekor log entries
# already bundled).
tar -xzf efe-cosign-bundle.tar.gz
cosign verify \
  --insecure-skip-tlog \
  --certificate-identity-regexp "..." \
  --certificate-oidc-issuer "..." \
  --certificate-chain efe-cosign-bundle/cert-chain.pem \
  ghcr.io/efe-ai/efe-server:1.0.0
```

## Documentation

Full verification guide : [`docs/release/VERIFY_RELEASE.md`](../docs/release/VERIFY_RELEASE.md)

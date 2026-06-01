# Quarterly compliance archive

Every quarter (Apr / Jul / Oct / Jan), `efe-slo-rollup` generates :
- `quarterly/Q{1..4}-{yyyy}.json` — machine-readable summary.
- `quarterly/Q{1..4}-{yyyy}.pdf` — human-readable, 11 pages.
- `quarterly/Q{1..4}-{yyyy}.pdf.bundle` — cosign keyless OIDC bundle.

## Verifier

```bash
# Offline — needs only the public key.
efe-slo-rollup evidence verify quarterly/Q2-2026.pdf
```

The verifier :
1. Re-computes SHA256 of the JSON sidecar.
2. Checks it matches the SHA embedded in the PDF metadata.
3. (If `cosign` is on PATH) verifies the OIDC signature against the
   public key at `deploy/cosign/slo-evidence.pub`.

## Audit chain

| Property | Implementation |
|---|---|
| Tamper-evidence | PDF metadata embeds SHA256 of JSON sidecar |
| Authenticity | cosign signature with GitHub OIDC identity |
| Non-repudiation | Rekor transparency log (append-only) |
| Time-witnessing | OIDC token carries workflow run timestamp |

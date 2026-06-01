# Runbook : bench-publish-quarterly evidence revocation (T121 mitigation)

**Trigger** : a previously-published quarterly PDF needs to be
revoked / erratised (e.g. SHA mismatch detected post-publish, baseline
data error, legal request).

## Critical invariant

**Quarterly evidence is NEVER deleted.**  Rekor transparency log entries
are append-only.  Customer auditors may already have verified the
original PDF ; deleting it breaks their chain of custody.

## Phase 0 — Decision

Open a release-eng incident.  Two valid paths :

### A — ERRATA (correctable error)

The original numbers were wrong but the methodology + signature chain
remain valid.  Issue a corrected PDF under a new path :

```
Original : /quarterly/Q2-2026.pdf
Erratum  : /quarterly/Q2-2026-rev2.pdf
```

### B — RETRACTION (signature compromised)

The signature is no longer trusted (key compromise, OIDC issue, etc.).
The PDF stays published with a `RETRACTED` notice ; auditors are
notified via a separate signed retraction notice.

## Phase 1 — ERRATA flow

```bash
# 1. Re-run the quarterly publisher with the corrected inputs.
./target/release/efe-bench-harness publish-quarterly \
    --period Q2-2026 \
    --chart-version 0.6.0 \
    --output ./bench \
    --cosign-identity "https://token.actions.githubusercontent.com"

# Phase-2 stub : the publisher emits Q2-2026.pdf — rename it to
# Q2-2026-rev2.pdf BEFORE committing.
mv ./bench/v0.6.0/quarterly/Q2-2026.pdf ./bench/v0.6.0/quarterly/Q2-2026-rev2.pdf
mv ./bench/v0.6.0/quarterly/Q2-2026.json ./bench/v0.6.0/quarterly/Q2-2026-rev2.json
mv ./bench/v0.6.0/quarterly/Q2-2026.pdf.bundle ./bench/v0.6.0/quarterly/Q2-2026-rev2.pdf.bundle
```

Append an ERRATUM entry to `/api/history.json` :

```jsonc
{
  "date": "Q2-2026-rev2",
  "url": "/quarterly/Q2-2026-rev2.pdf",
  "sha256": "<sha>",
  "overall_verdict": "pass",
  "replaces": "Q2-2026"
}
```

## Phase 2 — RETRACTION flow

Add a `RETRACTED` page :

```bash
# 1. Generate a retraction notice PDF.
cat <<EOF > /tmp/retraction-Q2-2026.txt
RETRACTION NOTICE — Q2-2026 bench evidence
Issued by : @efe-ai/release-eng
Reason    : <one-line reason>
Date      : $(date -u -Iseconds)

Affected PDF : https://efe.github.io/bench/quarterly/Q2-2026.pdf
Sha256       : <original sha>

Replacement : NONE / see Q2-2026-rev2.pdf
EOF

# 2. Sign the retraction notice with the SAME cosign keyless OIDC
#    identity used for the original evidence (shared regex).
cosign sign-blob --yes --bundle /tmp/retraction-Q2-2026.txt.bundle /tmp/retraction-Q2-2026.txt

# 3. Upload to gh-pages under /quarterly/Q2-2026.retraction.{txt,txt.bundle}.
```

## Phase 3 — Notify

- Customer success : email all customers who referenced Q2-2026 in
  their MSAs.
- Legal counsel : confirm retraction notice meets disclosure
  requirements.
- Compliance : update SOC 2 evidence binder with the retraction.

## Phase 4 — Post-mortem

Within 14 days, file `docs/postmortems/bench-quarterly-erratum-YYYY-MM-DD.md`.

## Reference

- PR #24 § 12 (quarterly evidence chain)
- PR #24 AD52, AD53 (write-once + ERRATA flow)
- PR #24 § 20 T121
- STABILITY_CONTRACT v1.6 § 24.4
- PR #22 § 13 (parallel SLO chain — same flow applies)

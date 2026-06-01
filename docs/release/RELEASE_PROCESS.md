# EFE Release Process

This document describes how EFE releases are produced, signed,
distributed, and announced. It is the operational counterpart to the
architecture document in `docs/EFE_RELEASE_V1_GA_ARCHITECTURE.md`.

**Audience** : release captains, security team, distribution maintainers.
**Cadence** : minor releases monthly, patch releases on demand, major
releases annually with a 2-month RC window.

---

## Roles

| Role               | Responsibility                                            | Backup |
|--------------------|-----------------------------------------------------------|--------|
| Release captain    | Drives the release end-to-end ; cuts the tag             | Eng manager |
| Security reviewer  | Approves Sigstore identity claims and SBOM diff           | CISO   |
| Compliance officer | Signs off on per-tenant compliance pack regeneration      | Legal  |
| Distribution lead  | Verifies all 6 channels reachable post-publish            | DevRel |
| Comms lead         | Owns blog + email + Slack + RSS announce                  | PMM    |

A release requires sign-off from at least 3 of the 5 (release captain
must be one of the three).

---

## Phases

### Phase 0 — Preparation (T-7 days)

1. **Freeze date confirmed** in the release calendar.
2. **Open the tracking issue** with the release-tracking template ;
   link it from the master release calendar.
3. **CHANGELOG audit** : confirm `## [Unreleased]` section is complete
   and accurate.
4. **CVE sweep** : run `cargo audit`, `pip-audit`, `npm audit`,
   `trivy fs .` — fix or document each finding in the tracking issue.
5. **Stakeholder survey** : ping owners of each pinned dependency for
   any in-flight blockers.
6. **Compliance review** : confirm SOC 2 / GDPR / AI Act docs are up to
   date in `docs/compliance/`.

### Phase 1 — Pre-flight (T-1 day)

Run `./scripts/release/release-prepare.sh v1.0.0`. This 8-step script
validates :

1. Working tree clean.
2. On `main` branch.
3. Up-to-date with `origin/main`.
4. Tag `v1.0.0` does not already exist locally or remote.
5. CHANGELOG entry exists for v1.0.0 (not just `[Unreleased]`).
6. Release notes file exists at `docs/release/RELEASE_NOTES_v1.0.0.md`.
7. GPG key configured for `git tag -s` (`user.signingkey`).
8. CI green on the tip of `main` (gh status, last 3 checks).

If any step fails, the script aborts. Do **not** proceed to Phase 2.

### Phase 2 — Publish (T-0)

Run `./scripts/release/release-publish.sh v1.0.0`. This :

1. Creates the master tag : `git tag -s v1.0.0 -m "EFE v1.0.0 GA"`.
2. Creates 6 sub-tags atomically :
   - `efe-admin/v1.0.0`
   - `portal-admin/v1.0.0`
   - `charts/efe-stack/v1.0.0`
   - `tf-aws/v1.0.0`
   - `tf-gcp/v1.0.0`
   - `tf-azure/v1.0.0`
3. Pushes all 7 tags : `git push origin v1.0.0 efe-admin/v1.0.0 ...`.

The push triggers the **`release-v1-ga.yml`** GitHub Actions workflow
(see `.github/workflows/release-v1-ga.yml`).

### Phase 3 — Pipeline (T+15 min)

The pipeline executes 5 sequential phases :

```
┌─────────────┐   ┌────────────────────┐   ┌──────────┐   ┌──────────────┐   ┌──────────┐
│  validate   │ → │  build + sign      │ → │  verify  │ → │  distribute  │ → │ announce │
│  (parallel) │   │  (Docker × 4 +     │   │ (cosign  │   │ (OCI · GH ·  │   │ (blog ·  │
│             │   │   Helm + CLI × 6 + │   │  + slsa- │   │  Homebrew ·  │   │  email · │
│             │   │   npm + TF)        │   │  verifier│   │  TF Reg)     │   │  Slack · │
│             │   │                    │   │  + k3d)  │   │              │   │  RSS)    │
└─────────────┘   └────────────────────┘   └──────────┘   └──────────────┘   └──────────┘
```

Expected duration : 35–45 minutes wall-clock for a clean run.

### Phase 4 — Smoke verification (T+1h)

After the pipeline completes, the release captain runs :

```bash
./scripts/release/verify-release.sh v1.0.0
```

This is a black-box verification — it pulls artifacts from the public
registries and validates Sigstore + SLSA + functional install via k3d.

### Phase 5 — Announce gate (T+2h)

Once verify passes, the announce phase runs (re-triggered manually if
needed). It posts to :

1. CMS blog (`scripts/release/publish-blog.sh`).
2. Customer mailing list via Mailgun batch.
3. Partner Slack channels.
4. Security RSS feed (`docs/security/feed.xml`).
5. VEX feed (`docs/security/vex/feed.json`) — only on yank.

### Phase 6 — Post-release (T+24h)

1. Smoke deploy to the staging cluster ; observe metrics for 24h.
2. Update the public status page entry (`status.efe.example.com`).
3. Open the next release tracking issue (`v1.0.1` or `v1.1.0`).
4. Compliance pack regeneration cron triggers at 02:00 UTC nightly.

---

## Yank procedure

If a P0/P1 defect surfaces after release, the captain runs :

```bash
./scripts/release/release-rollback.sh v1.0.0 P1 "fix breaking schema migration"
```

This wraps `gh workflow run release-rollback.yml`. The workflow :

1. Asks for a typed confirmation (the version must be retyped).
2. Marks Docker images deprecated (`crane mutate`).
3. Republishes Helm chart with `-yanked` suffix.
4. Converts GitHub Release to draft.
5. Cargo yanks the crate.
6. NPM deprecates the package.
7. Appends a VEX statement.
8. Appends an RSS event.
9. Sends a customer notification email.

**What yank cannot do** : remove Sigstore signatures from Rekor (Rekor is
append-only by design). The signatures remain valid forever, but the
release is flagged as deprecated through every other channel.

---

## Audit trail

Every step of the pipeline emits structured logs to the GitHub Actions
log retention store (90 days), and signed provenance to Rekor (forever).
The release captain attaches a one-pager summary to the tracking issue
including :

- Pipeline run URL.
- All 7 tag SHAs.
- Rekor log indices for each artifact.
- Verify-release output.
- Compliance pack regeneration timestamp.

---

## SLA

| Event                                 | Target SLA           |
|---------------------------------------|----------------------|
| Phase 1 → Phase 2                     | < 1 hour             |
| Pipeline duration                     | < 45 minutes         |
| Verify smoke                          | < 10 minutes         |
| Announce post-pipeline                | < 1 hour             |
| Compliance pack refresh post-release  | < 24 hours           |
| Yank decision → customer email        | < 1 hour (P0/P1)     |
| Yank decision → customer email        | < 24 hours (P2)      |

---

## References

- Pipeline : `.github/workflows/release-v1-ga.yml`
- Rollback workflow : `.github/workflows/release-rollback.yml`
- Cosign policy : `tools/sigstore/cosign-policy.yaml`
- SLSA policy : `tools/sigstore/slsa-policy.yaml`
- Architecture : `docs/EFE_RELEASE_V1_GA_ARCHITECTURE.md`
- Verification : `docs/release/VERIFY_RELEASE.md`
- Distribution channels : `docs/release/DISTRIBUTION_CHANNELS.md`
- Security disclosure policy : `docs/release/SECURITY_DISCLOSURE_POLICY.md`

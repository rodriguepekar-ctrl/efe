# Backup + Disaster Recovery (D17) — operator howto

> **Audience** : platform / SRE operators running EFE in production.
> **Pre-requisites** : Helm chart `backup.enabled = true`, S3 bucket
> with KMS encryption + object-lock, Vault deployed, pgBackRest
> configured on the Postgres primary.
>
> **Diátaxis position** : task-oriented howto. For concepts / why,
> see the [D17 architecture doc][arch]. For DR drill metric
> reference, see the [SLO dashboard][slo].
>
> [arch]: ../../bench/d17_backup_dr_architecture.md
> [slo]: ../reference/slo-dashboards.md

This howto walks an on-call operator through the four most common
operations of the D17 backup pipeline :

1. [Taking an ad-hoc backup](#1-take-an-ad-hoc-backup)
2. [Verifying a backup at rest](#2-verify-a-backup-at-rest)
3. [Restoring from a backup (full DR runbook)](#3-restore-from-a-backup)
4. [Running a DR drill](#4-run-a-dr-drill) (weekly)
5. **NEW (D40)** [Quarterly DR drill](#6-quarterly-dr-drill-d40) — formal
   compliance evidence (SOC 2 / ISO 27001 / NIS2 / RGPD)

For the **delete + retention** flows (4-eyes guarded), see
[Delete a backup (RGPD erasure)](#5-delete-a-backup-rgpd-erasure)
at the bottom.

---

## 1. Take an ad-hoc backup

Scheduled backups run automatically (CronJobs : Sunday 06:00 UTC
full, Monday-Saturday 06:00 UTC diff, every 6 h incremental, daily
06:30 UTC Vault snapshot, Sunday 07:00 UTC bundle sync). Use the
ad-hoc path when :

* You need a snapshot **right before** a risky operation
  (schema migration, mass tenant onboarding).
* The CronJob has been failing and you want to recover state ASAP.
* You're testing a new pgBackRest config.

### Steps

```bash
# Snapshot every source (Postgres + Vault + Bundles) with the
# default kind (full Postgres + vault_snapshot + bundle_sync).
kubectl --namespace efe exec deploy/efe-server -- \
  efe-backup snapshot --label="adhoc-$(date -u +%Y%m%dT%H%M%SZ)"

# Snapshot only Postgres, diff kind (smaller, faster).
kubectl --namespace efe exec deploy/efe-server -- \
  efe-backup snapshot --source=postgres --kind=diff

# JSON output for piping into jq / a recording job.
kubectl --namespace efe exec deploy/efe-server -- \
  efe-backup snapshot --json | jq '.snapshots[] | {source, label, ok}'
```

### What gets emitted

* The 8 D17 Prometheus instruments are bumped (architecture § 19).
  Observe in Grafana :
  * `efe_backup_runs_total{result="ok"}` increments.
  * `efe_backup_duration_seconds` records the wall-clock.
  * `efe_backup_last_success_timestamp` is set to `now()`.
* An audit row is emitted in the D14-bis HMAC-chained log
  (`event=backup.snapshot, status=ok, source=postgres, ...`).
* If `EFE_BACKUP_SIGNING_KEY_PATH` is set, the snapshot manifest
  is Ed25519-signed with the configured key (audit fix § AC3).

### Common failures

| Exit | Cause | Recovery |
|-----:|-------|----------|
| 2 | Config error (e.g. RPO ≥ RTO) | Fix Helm values + `helm upgrade` |
| 12 | Subprocess timeout | Investigate network to S3 / Vault |
| Pod CrashLoop | Binary missing | Verify `Dockerfile.prod` ships `efe-backup` (pin test) |

---

## 2. Verify a backup at rest

Verification does NOT touch the live cluster. It :

1. Reads the manifest from the backup repo.
2. Re-computes its SHA-256.
3. Verifies the Ed25519 signature against the trust store.
4. (For pgBackRest) runs `pgbackrest check`.

### Steps

```bash
# Verify the most recent backup of every source.
kubectl --namespace efe exec deploy/efe-server -- \
  efe-backup verify --backup-id=20260428T060000Z-full

# Verify only Postgres.
kubectl --namespace efe exec deploy/efe-server -- \
  efe-backup verify --backup-id=20260428T060000Z-full --source=postgres
```

### Interpreting the result

* **Exit 0** : signature valid + at-rest integrity OK. Safe to
  restore from.
* **Exit non-zero** :
  * Output `manifest verification failed: sha256_self mismatch
    (manifest tampered)` → repo corruption, do NOT restore.
  * Output `key_id ... not in active trust store` → trust store
    drift. Either rotate to the right trust file or restore the
    original.

---

## 3. Restore from a backup

The full restore runbook is `dr/restore.sh` (in the repo). DO NOT
manually call `efe-backup restore` for prod — use the runbook,
which orchestrates the **canonical 6-step pipeline** :

```text
0. pre-flight verify
1. restore Vault    (HMAC keys + OIDC issuers + trust store)
2. restore Postgres (catalog + audit_log)
3. restore bundles  (rclone S3 sync from backup bucket)
4. helm upgrade     (re-applies runtime config)
5. post-restore verify (HMAC chain + bundle sigs + OIDC + smoke)
6. smoke test       (catalog list + Execute RPC canary)
```

### Quick start

```bash
# Restore latest backup into the prod environment.
./dr/restore.sh 20260428T060000Z-full --target prod

# Point-in-time restore (PITR) — Postgres only.
./dr/restore.sh 20260428T060000Z-full \
    --target prod \
    --target-time "2026-04-28 06:30:00 UTC"

# Cross-environment restore (e.g. prod backup → staging) — REQUIRES
# 4-eyes approval + DPO email + audit-logged with RGPD risk flag.
./dr/restore.sh 20260428T060000Z-full \
    --target staging \
    --force-cross-env
```

### Exit code semantics

| Exit | Stage |
|-----:|-------|
| 0 | Restore + smoke succeeded |
| 1 | Argument parse error |
| 2 | Pre-flight failed (corrupted backup) |
| 3 | Vault restore failed |
| 4 | Postgres restore failed |
| 5 | Bundles restore failed |
| 6 | Helm upgrade failed |
| 7 | Post-restore verify failed |
| 8 | Smoke test failed |
| 11 | RTO violation (`duration > rto_target_seconds`) |

### RTO target

The script emits `dr/timing.json` with the actual duration. Compare
against `EFE_BACKUP_RTO_TARGET_S` (default 1800 s = 30 min) :

```bash
$ jq '.duration_seconds, .rto_target_seconds' dr/timing.json
1432
1800
```

If the actual duration ≥ target, an alert `D17DrRtoViolation` will
fire on the next Prometheus scrape (assuming this restore was run
as part of a drill ; ad-hoc prod restores do NOT emit drill metrics).

### Critical ordering invariant

Vault MUST be restored before Postgres, because the Postgres
restore re-verifies the audit chain HMAC against the keys living
in Vault. Restoring Postgres first means `verify-audit-chain` will
reject every row as "broken chain".

If you discover the chain is broken post-restore :

1. Check Vault snapshot age :
   ```bash
   aws s3 ls s3://efe-backups-prod/vault-snapshots/ | sort -k1,2 | tail -3
   ```
2. If keys older than DB → escalate. The chain requires keys
   AT LEAST as recent as the latest signed row.
3. As a last resort, follow the **re-anchor procedure**
   (architecture § 12.3) :
   * Document the loss in an external immutable record (paper /
     Time Machine).
   * Generate a new HMAC key.
   * Insert a `re_anchor=true` row in `audit_log` with motive.
   * Notify the DPO within 72 h (RGPD Art. 33).

---

## 4. Run a DR drill

The DR drill restores the latest backup into an **ephemeral
namespace** (`efe-dr-drill-<date>`), runs the 5-verifier suite,
measures RTO end-to-end, and destroys the namespace.

### Automatic schedule

The Helm chart provisions a CronJob :

```yaml
backup:
  drill:
    enabled: true
    schedule: "0 3 * * 0"          # Sunday 03:00 UTC
    rtoTargetSeconds: 1800         # 30 min
```

The drill runs every Sunday morning. Outcome metrics :

* `efe_dr_drill_runs_total{result="success"|"failure"}`
* `efe_dr_drill_duration_seconds`
* `efe_dr_drill_last_success_timestamp`

### Manual invocation

When investigating a recent drill failure, you can re-run on
demand :

```bash
# In-cluster CLI mode (uses the prod image, runs verifiers against
# the live cluster — does NOT do a full restore).
kubectl --namespace efe exec deploy/efe-server -- \
  efe-backup drill --target-env=drill --json | jq

# Full end-to-end drill (requires a kind / staging cluster).
./dr/setup-test-env.sh --cluster-name efe-dr-manual
./dr/restore.sh <latest-backup-id> --target drill
```

### Drill outcome → action

| Outcome | Action |
|---------|--------|
| `success: true, within_rto: true` | Healthy. No action. |
| `success: true, within_rto: false` | RTO regression. Profile the slowest restore stage (Postgres typically). DT-D17.X bench candidate. |
| `success: false, verifier=audit_chain` | HMAC chain broken in restore — trust store / Vault snapshot misalignment. See § 3 critical ordering. |
| `success: false, verifier=bundle_signatures` | Embedder bundle Ed25519 sig invalid. Rotate trust store and re-sign. |
| `success: false, verifier=oidc_issuers` | JWKS endpoint unreachable. Check IdP health, NetworkPolicy egress. |

---

## 5. Delete a backup (RGPD erasure)

Deletion is **4-eyes guarded** — requires two distinct JWT tokens
with non-empty `sub` claims, plus a free-form reason that is
audit-logged with RGPD risk flagging.

### Steps

```bash
# Tokens are issued by the OIDC IdP via `efe-onboard get-token`
# (D15 path) and stored in K8s Secrets that the operator pod
# mounts.

# Primary requestor (must have permission `backup:delete:request`).
export EFE_BACKUP_PRIMARY_TOKEN=$(cat /var/run/secrets/efe/ops-lead-token)

# Secondary approver (must have `backup:delete:approve` + distinct
# `sub` from primary).
export EFE_BACKUP_SECONDARY_TOKEN=$(cat /var/run/secrets/efe/dba-lead-token)

efe-backup delete \
    --backup-id=20260420T060000Z-full \
    --reason="GDPR erasure for tenant T-XYZ (request ticket SEC-1234)"
```

### What gets logged

A row in the D14-bis HMAC chain :

```json
{
  "schema_version": "1.0",
  "timestamp": "2026-04-28T14:35:00Z",
  "event": "backup.delete",
  "label": "20260420T060000Z-full",
  "primary_sub": "ops-lead@example.com",
  "secondary_sub": "dba-lead@example.com",
  "reason": "GDPR erasure for tenant T-XYZ (request ticket SEC-1234)",
  "rgpd_risk": false
}
```

### Trust boundary (audit § AH8)

The CLI does NOT verify JWT signatures. Trust delegation is to the
upstream surfacing path :

* In CronJob context, tokens come from K8s Secrets that
  SealedSecrets / external-secrets controllers populate from a
  verified IdP-provisioned source.
* In manual ops, the operator runs them through
  `efe-server admin verify-token` (D15 path) before injecting
  into env vars.

The CLI enforces only the **structural state machine** : two
distinct tokens, non-empty subs, non-empty reason. In-process
signature verification is deferred to D17-bis (DT-D17.1).

---

## 6. Quarterly DR drill (D40)

> **Status** : Sprint D40 (Φ-F), GA v1.0.0.
> **Cadence** : 1st of Jan/Apr/Jul/Oct at 04:00 UTC (4× per year).
> **Audience** : platform/SRE operators + DPO + compliance officer.

The **quarterly drill** is the formal compliance evidence layer
that the weekly drill (§4) deliberately does not provide. It
**rejoue the runbook end-to-end** so the operator runbook in this
document stays opera­tio­nally accurate quarter after quarter.

### 6.1 Why a second drill cadence ?

The weekly drill covers **technical regression** : restore + 5
verifiers (HMAC chain, bundle signatures, OIDC, catalog smoke,
Execute smoke). It runs against the **existing** cluster's
`drill` namespace.

The quarterly drill covers **business regression** + **runbook
freshness** + **compliance evidence** :

* **Business regression** — runs a Hello-World workflow
  end-to-end (register catalog → register class → register
  sentence → resolve catalog → execute → assert HMAC chain
  integrity). Catches regressions where the binaries are up but
  the **business path** is broken.
* **Runbook freshness** — bootstraps a kind cluster from-scratch
  + Helm-installs the EFE chart + restores from S3. If the doc
  drifts from the code, the drill fails. The doc stays
  operational by construction.
* **Compliance evidence** — emits an immutable evidence pack
  (`outcome.json` v2 + logs + Ed25519 attestation) uploaded to
  S3 Object Lock in `COMPLIANCE` mode for 7 years. Satisfies
  SOC 2 CC9.1 / ISO 27001 A.17.1.3 / NIS2 Art. 21 / RGPD Art. 32.

### 6.2 Running a quarterly drill manually

```bash
# 1. Pre-requisites : kind, helm, kubectl, jq, openssl.
brew install kind helm kubectl jq openssl  # macOS
# apt-get install kind helm kubectl jq openssl  # Linux

# 2. Pick the latest successful backup from the catalog.
BACKUP_ID=$(efe-backup list --since=7d --json | jq -r '.[0].label')

# 3. Run the drill (bootstrap + restore + Hello-World + verify + evidence).
./dr/run_quarterly_drill.sh \
    --cluster-name efe-drill-q-$(date +%Y%m) \
    --backup-id "$BACKUP_ID" \
    --evidence-dir ./.ci-evidence/dr-drill-quarterly/$(date +%Y)-Q$(( ($(date +%-m) - 1) / 3 + 1 ))
```

The script runs **6 phases** :

| Phase | Action | Time budget |
|------:|--------|------------:|
| 1 | Kind cluster + MinIO + cnpg setup (reuses `dr/setup-test-env.sh`) | 5 min |
| 2 | Helm install of EFE chart (`backup.enabled=true`) | 3 min |
| 3 | Restore from `BACKUP_ID` (calls `dr/restore.sh --force-cross-env`) | 6 min |
| 4 | **Hello-World workflow** business end-to-end (NEW D40) | 60 s SLO D40-DR-2 |
| 5 | Verifier suite (`efe-backup drill --target-env=drill`) | 1 min |
| 6 | Evidence emit (`outcome.json` v2 + Ed25519 attestation) + cleanup | 1 min |
| **Total** | RTO target ≤ 45 min (SLO D40-DR-1) | ~16 min typical |

### 6.3 Running via Helm CronJob (production)

The CronJob `efe-dr-drill-quarterly` is rendered by the chart
when `backup.drillQuarterly.enabled=true` (default ON). Schedule :
`"0 4 1 1,4,7,10 *"` (1st of each quarter at 04:00 UTC).

```bash
# Inspect the schedule.
kubectl --namespace efe get cronjob efe-dr-drill-quarterly

# Manual trigger for a one-off compliance run.
kubectl --namespace efe create job \
    --from=cronjob/efe-dr-drill-quarterly \
    drill-q$(date +%Y%m) -n efe

# Watch progress.
kubectl --namespace efe logs -f job/drill-q$(date +%Y%m)
```

### 6.4 Reading the evidence pack

```bash
# After a successful drill, the evidence pack lives at :
ls -la .ci-evidence/dr-drill-quarterly/2026-Q2/
# → outcome.json (canonical v2 artefact)
# → kind-cluster.log + helm-install.log + restore.log
# → helloworld.log + verifier-suite.log + timing.json
# → attestation.txt (4-eyes signed Ed25519)

# Pretty-print the outcome.
jq '.' .ci-evidence/dr-drill-quarterly/2026-Q2/outcome.json
```

The `outcome.json` schema is pinned at v2.0.0 (see
`docs/compliance/dr-drill-outcome-2.0.0.schema.json`) and validated
by `scripts/tests/test_dr_drill_runbook.py`.

### 6.5 SLO contract

| SLO | Target | Field |
|-----|--------|-------|
| `D40-DR-1` | RTO ≤ 45 min | `outcome.json:within_rto == true` |
| `D40-DR-2` | Hello-World latency ≤ 60 s | `helloworld.elapsed_secs <= 60` |
| `D40-DR-3` | Drill success rate ≥ 95 % over 4 last quarters | rolling avg |
| `D40-DR-4` | Drill freshness ≤ 90 d | latest `finished_at` within 90 d |

### 6.6 Compliance frameworks

The quarterly drill provides formal evidence for :

* **SOC 2 CC9.1** — Risk Mitigation (business continuity).
* **ISO 27001 A.17.1.3** — Verify continuity.
* **NIS2 Art. 21.2(c)** — Backup management + DR + crisis management.
* **RGPD Art. 32.1(c) + (d)** — Restoration + regular testing.

Full mapping in `docs/compliance/dr_drill_quarterly_evidence.md`.

### 6.7 Failed-drill handling

A failed drill **still emits** an evidence pack (auditors need
the failure record). PagerDuty alert `D40DrillQuarterlyFailed`
fires immediately ; on-call ops responds within 4 h ;
post-mortem + remediation drill within 14 days. See
`docs/compliance/dr_drill_quarterly_evidence.md` § 6 for the
full incident-response playbook.

### Common quarterly-drill failures

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| Phase 1 fails (`kind create cluster` errors) | Docker daemon down / OOM | Check `docker info`. Restart Docker. |
| Phase 4 (Hello-World) `verdict=DIVERGENCE` | Embedder bundle Ed25519 sig stale post-restore | Rotate trust store, re-sign. |
| Phase 4 elapsed > 60 s | CI runner under-provisioned | Bump runner tier, OR set `--keep` and run manually on a beefier host. |
| `success_overall=false` but `within_rto=true` + verifiers OK | Hello-World assertion failed silently | Read `helloworld.log` for the failed step. |
| Evidence pack missing in S3 | NetworkPolicy egress to S3 not allowed for drill ns | Add `efe-dr-drill-quarterly` egress rule. |

### Reference

* [`docs/EFE_BACKUP_EXTENSION_ARCHITECTURE.md`](../../EFE_BACKUP_EXTENSION_ARCHITECTURE.md)
  — sprint D40 architecture (1 970 LOC).
* [`docs/compliance/dr_drill_quarterly_evidence.md`](../../compliance/dr_drill_quarterly_evidence.md)
  — full compliance mapping per framework.
* [`.ci-evidence/dr-drill-quarterly/README.md`](../../../.ci-evidence/dr-drill-quarterly/README.md)
  — evidence pack layout reference.
* [`dr/run_quarterly_drill.sh`](../../../dr/run_quarterly_drill.sh)
  — script source (~440 LOC bash).

---

## Common questions

### Q : My backup CronJob exits 0 but no metrics show in Grafana.

Verify the `efe-backup` binary IS in the prod image :

```bash
$ kubectl --namespace efe exec deploy/efe-server -- ls /usr/local/bin/ | grep efe-backup
efe-backup
```

If absent, the image is mis-built — re-run `docker build` with the
correct stages (`docker/Dockerfile.prod` post-D17-AC1 fix). The
pin test `test_audit_ac1_efe_backup_in_dockerfile_prod` should
catch this in CI.

### Q : The drill RTO is consistently > 25 min.

The Postgres restore is usually the bottleneck. Profile via :

```bash
# Inside the drill pod log :
kubectl --namespace efe-dr-drill-<date> logs job/efe-dr-drill | \
    grep -E 'Step.*duration|pgbackrest restore'
```

Common culprits : WAL replay backlog (delete old WAL via retention),
S3 download throughput (cross-region restore), pgBackRest
`process-max=4` not enough for the data volume (bump to 8).

### Q : Can I restore a single tenant's data without affecting others ?

Not in v0.1.18. Postgres restore is cluster-wide. Tenant-level
restore is documented as a future sprint candidate (D18+).
Workaround : restore to a separate cluster, dump the tenant's
catalog rows, replay into prod via `efe-onboard upgrade-catalog`.

### Q : The 4-eyes flow refuses both tokens with "distinct sub" error.

The two JWTs must have different `sub` claims. If both come from
the same operator (e.g. they generated both themselves to bypass
4-eyes), the CLI refuses. This is by design — it enforces
two-person authorisation. Get a second operator to mint their own
token via the IdP.

---

## Reference

* [D17 architecture document](../../bench/d17_backup_dr_architecture.md)
* [D17 audit report](../../audits/d17_backup_dr_audit_report.md)
* [D14-bis HMAC chain howto](idp-integration-keycloak.md) (related : audit chain context)
* [Helm chart values reference](../reference/helm-values.md)
* [Prometheus metric reference (D17 § 19)](../../bench/d17_backup_dr_architecture.md#19-métriques-prometheus)

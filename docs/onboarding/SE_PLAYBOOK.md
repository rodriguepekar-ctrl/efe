# Solutions Engineer Playbook — 30-Minute Tenant POC

> **Sprint provenance** : GLOSS A (D8) — Go-Live & Onboarding Self-Service.
> Cf. [`docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md`](../EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md) § 8 (« Flux d'onboarding « 30 min POC » — déroulé chirurgical »).
> **Audience** : Solutions Engineers running a first-time tenant deploy from a fresh laptop.
> **Target time-to-classify** : ≤ 30 min on AWS, ≤ 45 min on Azure / GCP (cf. R-07).

This playbook is the **single source of truth** for the field SE
workflow. It supersedes ad-hoc Notion / Confluence pages. The contract
is : after a clean `terraform apply` + `helm install` + the steps below,
the prospect tenant runs its first classify call on a production-grade
EFE deployment, with audit chain + compliance attestations in place,
HMAC-chained from event 0.

---

## 0. Pre-flight checklist (≤ 5 min)

Run these on a fresh laptop **before** opening the prospect's screen-share :

```bash
# Required tooling
$ terraform version    # ≥ 1.8
$ helm version         # ≥ 3.14
$ kubectl version      # ≥ 1.29
$ efe-admin version    # ≥ v0.1.0
$ aws --version        # or gcloud / az depending on target cloud

# Sanity — connectivity & creds
$ aws sts get-caller-identity            # must succeed
$ kubectl cluster-info --context $TARGET # if cluster exists
```

If `efe-admin` is missing :

```bash
brew tap efe-ai/efe && brew install efe-admin
```

Cf. [`crates/efe-admin/README.md`](../../crates/efe-admin/README.md) §
« Installation ».

### Required artifacts in your toolbox

- The prospect's **catalog bundle** (`.jsonl`, anchor-per-line schema).
  If they haven't sent one, use `eval/golden/support_triage.jsonl` as a
  generic placeholder (≤ 50 anchors) and explain it is the demo catalog.
- The prospect's **DPO email** — required for `compliance attest`.
- A **SE bearer token** for the cluster (issued by your team lead via
  `efe-admin keys create` against the bootstrap tenant).

---

## 1. Provision infra (≤ 12 min on AWS)

Choose the cloud module under `terraform/modules/{aws,gcp,azure}`. Each
module has a `README.md` listing the variables. Below is the AWS path
(by far the fastest — Azure & GCP take longer because of LB warm-up).

```bash
cd terraform/modules/aws
terraform init

cat > terraform.tfvars <<EOF
region            = "eu-west-3"
tenant_slug       = "acme"
instance_type     = "m6i.large"        # control plane + worker default
worker_count_min  = 2
worker_count_max  = 6
data_residency    = "EU_ONLY"
allowed_egress    = ["api.prospect.com"]
backup_retention  = 35
EOF

terraform plan -out=acme.tfplan
terraform apply acme.tfplan
```

When `terraform apply` completes :

```bash
# Output reads required for next steps
TENANT_KUBECONFIG="$(terraform output -raw kubeconfig)"
TENANT_DB_URL="$(terraform output -raw postgres_connection_string)"
TENANT_REDIS="$(terraform output -raw redis_endpoint)"
TENANT_S3="$(terraform output -raw audit_bucket)"

export KUBECONFIG="$TENANT_KUBECONFIG"
kubectl cluster-info       # smoke
```

> **Caveat (R-07)** : if you're on Azure (`az aks create`) or GCP
> (`gcloud container clusters create`), expect 12-18 min versus AWS's
> 8-10 min. Set expectations with the prospect.

---

## 2. Deploy the EFE stack (≤ 5 min)

```bash
cd ../../../charts/efe-stack
helm dependency update

# 2a. Generate a tenant-specific values file
cat > values-acme.yaml <<EOF
global:
  region: eu-west-3
  data_residency: EU_ONLY
  tenant_slug: acme

efe-server:
  image:
    tag: v1.0.0
  replicas: 2
  resources:
    requests: { cpu: 500m, memory: 1Gi }
    limits:   { cpu: 2,   memory: 4Gi }

mcp-server:
  image:
    tag: v1.0.0
  replicas: 1

portal-admin:
  enabled: false   # enable in GLOSS-C ; not part of MVP self-service flow

postgres-operator:
  storage: 50Gi
  backup: { enabled: true, schedule: "0 2 * * *" }

redis-stack:
  replicas: 1
EOF

# 2b. Install (helm-unittest already ran in CI ; we trust the chart)
helm install efe-stack . \
    --namespace efe-acme \
    --create-namespace \
    --values values-acme.yaml \
    --wait \
    --timeout 8m
```

If the install times out, jump to **§ 7 Troubleshooting → RB-08
(migration failure)**.

---

## 3. Create the tenant (≤ 2 min)

```bash
# 3a. Configure the CLI alias
mkdir -p ~/.config/efe-admin
cat > ~/.config/efe-admin/config.toml <<EOF
[default]
cluster        = "https://efe-acme.example.com"
auth_method    = "token"
token_file     = "~/.config/efe-admin/acme.token"
output_format  = "table"
confirm        = "interactive"
EOF
echo "$SE_BEARER_TOKEN" > ~/.config/efe-admin/acme.token
chmod 600 ~/.config/efe-admin/acme.token
export EFE_ADMIN_CLUSTER=default

# 3b. Provision the tenant
efe-admin tenant create acme \
    --plan enterprise \
    --max-rps 1000 \
    --max-tokens-per-day 1000000 \
    --data-residency EU_ONLY \
    --contact-email ops@acme.com \
    --yes

# 3c. Confirm
efe-admin tenant get acme
```

Expected output for `tenant get` :

```text
┌──────────────┬──────────┐
│ key          │ value    │
├──────────────┼──────────┤
│ name         │ acme     │
│ plan         │ enterpr… │
│ status       │ active   │
│ max_rps      │ 1000     │
│ residency    │ EU_ONLY  │
└──────────────┴──────────┘
```

---

## 4. Compliance attestation (≤ 3 min)

This step is **mandatory** before any classify call. EFE enforces a hard
Postgres gate (`mode3_compliance_gate` trigger from migration
`0013_audit_chain_hmac.sql`) that rejects any request from a tenant
that has not attested AI Act Art. 14 **and** GDPR Art. 22.

```bash
# Make sure you have the DPO's signed authorization in writing before
# running this. The attestation is *immutable* — it cannot be retracted,
# only superseded by a later version signed by a DPO of record.
efe-admin compliance attest acme \
    --ai-act-art14 \
    --gdpr-art22 \
    --signed-by dpo@acme.com \
    --dpo-contact dpo@acme.com \
    --yes

efe-admin compliance verify acme
```

`compliance verify` returns `ai_act_art14: true, gdpr_art22: true,
signed_by: dpo@acme.com, attested_at: …`. If any flag returns `false`,
**do not proceed** — the prospect MUST attest first.

> **Anti-pattern** : never sign as yourself. The attestation log is
> reviewed by ENISA-grade auditors ; the `signed_by` must match a
> natural person at the prospect with DPO authority.

---

## 5. Seed the catalog (≤ 5 min for a 500-anchor bundle)

```bash
# 5a. Validate locally first (catches PII + schema issues before upload)
python3 eval/scripts/catalog_hygiene.py \
    --input ./tenants/acme/support-triage.jsonl \
    --strict

# 5b. Upload
efe-admin catalog seed acme \
    --bundle ./tenants/acme/support-triage.jsonl \
    --bundle-version 1.0.0 \
    --catalog default \
    --yes

# 5c. List to confirm
efe-admin catalog list acme
```

If the upload is slow, increase your client connection timeout :

```bash
EFE_ADMIN_HTTP_TIMEOUT=10m efe-admin catalog seed acme …
```

Anchors are sharded server-side ; the seed call is a single tool
invocation under the hood, but the embedder runs at ~50 anchors/s.
A 500-anchor bundle takes about 10 s + network roundtrip.

---

## 6. Smoke-test (≤ 2 min)

```bash
# 6a. Issue a classify-scope key
efe-admin keys create acme --scope classify --expires-in 30d

# 6b. Run the embedded smoke test
efe-admin smoke-test acme
```

Pass criteria :

- 100 % of sentences classified (no `pii_rejected` errors)
- `p99_latency_ms` ≤ 5000
- `top1_accuracy` ≥ 0.80

```text
✓ smoke-test acme [12/12 OK | p50=420ms p99=1830ms | top1=0.92]
```

If accuracy is below threshold, jump to **§ 7 Troubleshooting →
RB-13 (low accuracy)**.

---

## 7. Hand-off (≤ 3 min)

Send the prospect :

1. **API endpoint** — `https://efe-acme.example.com` (from `terraform
   output ingress_url`).
2. **API key** — printed by `keys create` (shown ONCE — record it
   securely in the prospect's password manager).
3. **Audit query example** :

   ```bash
   efe-admin audit query acme --since 2026-05-12T00:00:00Z --limit 10
   ```

4. **Docs links** :
   - SDK Quickstart (Python / Go / TS / Java) — `clients/{python,go,ts,java}/README.md`
   - DPO Quickstart — [`docs/onboarding/DPO_QUICKSTART.md`](DPO_QUICKSTART.md)
   - Runbooks — [`docs/runbooks/`](../runbooks/)

5. **Status page** — `https://status.efe.example.com` (the prospect
   subscribes via email or RSS).

### Confirm the handoff in writing

Send a recap email :

```
Subject : EFE deployment acme — handoff confirmed

Hi,

The EFE deployment for acme is live :

- Endpoint   : https://efe-acme.example.com
- Plan       : enterprise (1000 RPS, 1M tokens/day, EU_ONLY residency)
- Compliance : AI Act Art. 14 + GDPR Art. 22 attested by dpo@acme.com
- Audit      : HMAC chain initialised, chain_position 0 = create event

Please rotate the API key within 30 days using :
    efe-admin keys rotate acme --key-id <ID>

For incidents : refer to docs/runbooks/ ; for support : sales@efe.ai

Best,
<SE name>
```

---

## 8. Troubleshooting

Each known incident has a dedicated runbook under `docs/runbooks/`. The
mapping is :

| Symptom | Runbook |
|---|---|
| Postgres disk full | [`RUNBOOK_RB_01_DB_DISK_FULL.md`](../runbooks/RUNBOOK_RB_01_DB_DISK_FULL.md) |
| Crash loop on `efe-server` | [`RUNBOOK_RB_06_CRASH_LOOP.md`](../runbooks/RUNBOOK_RB_06_CRASH_LOOP.md) |
| p99 latency > 5 s | [`RUNBOOK_RB_07_LATENCY_P99.md`](../runbooks/RUNBOOK_RB_07_LATENCY_P99.md) |
| Migration failed during `helm install` | [`RUNBOOK_RB_08_MIGRATION_FAILED.md`](../runbooks/RUNBOOK_RB_08_MIGRATION_FAILED.md) |
| Audit chain broken (gap detected) | [`d13_classify_drift.md`](../runbooks/d13_classify_drift.md) |
| Postgres replica failover stuck | [`d14_postgres_failover.md`](../runbooks/d14_postgres_failover.md) |
| Tenant onboarding failed (DB / sandbox) | [`d15_tenant_onboarding_failed.md`](../runbooks/d15_tenant_onboarding_failed.md) |
| OIDC JWKS stale | [`d15_oidc_jwks_stale.md`](../runbooks/d15_oidc_jwks_stale.md) |

If none of these match, escalate to `#efe-oncall` on Slack with the
output of :

```bash
efe-admin health --format json > health.json
kubectl -n efe-acme get events --sort-by=.metadata.creationTimestamp \
    | tail -100 > events.log
```

and attach both files.

---

## 9. Decommissioning a POC

When a POC concludes (e.g. prospect didn't convert) :

```bash
# 9a. Export the audit log for archival
efe-admin audit export acme \
    --since 2026-05-12T00:00:00Z \
    --to ./archive/acme-audit.jsonl

# 9b. Suspend (reversible)
efe-admin tenant suspend acme --reason "POC concluded — pending decision"

# 9c. After legal review, delete (irreversible, 7-year audit retention)
efe-admin tenant delete acme --purge-audit-after-days 2555 --yes

# 9d. Terraform destroy
cd terraform/modules/aws
terraform destroy -auto-approve
```

The audit chain remains queryable for the retention window even after
`tenant delete` — only the runtime data is purged. This satisfies GDPR
Art. 17 « right to erasure with proportional retention for legitimate
interest » as documented in `docs/EFE_M29_DATA_RIGHTS_ARCHITECTURE.md`.

---

## 10. Service-level commitments

These are the SE-facing SLO targets for the onboarding workflow itself.
They are tracked in [`docs/audits/gloss_sprint_a_closure.md`](../audits/gloss_sprint_a_closure.md) § 4.

| Metric | Target | Source |
|---|---|---|
| `terraform apply` (AWS) | ≤ 12 min | Architecture § 13.1 |
| `helm install` (cloud) | ≤ 8 min | § 13.1 |
| `efe-admin tenant create` E2E | ≤ 3 s | § 13.1 |
| `efe-admin catalog seed` (500 anchors) | ≤ 5 min | § 13.1 |
| `efe-admin smoke-test` p99 | ≤ 5 s | § 13.1 |
| Total time-to-classify (AWS) | ≤ 30 min | § 0.4 DoD criterion 1 |
| Total time-to-classify (Azure / GCP) | ≤ 45 min | R-07 mitigation |

Track your actual durations in the run log (`runs/<tenant>-<date>.log`)
and submit to the field-ops weekly review.

---

## 11. Field tips

- **Always run `efe-admin smoke-test` last** — it produces a clean
  pass/fail signal for the prospect's recorded screenshot.
- **Never `--yes` past compliance** — Art. 14 + Art. 22 attestation is
  a legal commitment of the prospect ; have them confirm verbally on
  the call before you flip the flag.
- **Pin Helm chart version** — `helm install efe-stack . --version
  X.Y.Z` to avoid drift if the umbrella chart is bumped mid-deploy.
- **Tokens older than 1 h fail** — the SE bearer token has a 1 h TTL
  by default. Re-issue with `efe-admin keys create` against the
  bootstrap tenant if you cross that boundary mid-session.
- **`--format json` is your friend** for screen-sharing — strip ANSI
  colors, easier to copy/paste into post-call notes.

---

## 12. Doc maintenance

This playbook is co-located with the architecture under
`docs/onboarding/`. Update it whenever any of these change :

- New `efe-admin` subcommand added → update § 3 / § 4 / § 5 / § 6
- New Terraform variable required → update § 1
- New Helm value mandatory → update § 2
- New runbook added → update § 8

PRs touching this file must include the field-ops team as reviewer
(see `.github/CODEOWNERS`).

— **fin du document** —

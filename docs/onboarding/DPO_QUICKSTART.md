# DPO Quickstart — EFE Tenant Data Protection

> **Sprint provenance** : GLOSS A (D8).
> Cf. [`docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md`](../EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md) § 13.4.
> **Audience** : tenant Data Protection Officer (DPO) or equivalent authority signing on EU AI Act + GDPR conformity for the EFE deployment.
> **Length** : ≤ 15 min read.

This guide explains, in plain language, what the EFE deployment does
with your data, what regulatory obligations it carries on your behalf,
and what you (the DPO) must sign before any classification call is
allowed.

---

## 1. What is EFE doing with my data ?

EFE (« Essaim Fluide Éphémère ») is a **classification engine** : it
receives short text segments (sentences, paragraphs, support tickets,
…) from your applications and returns a label + confidence score
drawn from a catalog you control.

Three modes operate, with strictly different data-handling profiles :

| Mode | Description | Data leaves your tenant ? |
|---|---|:---:|
| **Mode 1** (vector swarm) | Pure-Rust embedding + nearest-centroid match | **NO** — fully local to the cluster |
| **Mode 2** (LLM-routed anchors) | Anchor selection via routed LLM call | **YES, partial** — only the anchors (no raw input) leave |
| **Mode 3** (multi-turn) | LLM dialog with multiple turns | **YES, partial** — input + intermediate turns leave |

All modes share the same **audit chain** (HMAC-chained, tamper-evident)
and the same **PII pipeline** (8 regex patterns + circuit breaker —
cf. [`docs/EFE_M29_DATA_RIGHTS_ARCHITECTURE.md`](../EFE_M29_DATA_RIGHTS_ARCHITECTURE.md)).

By default a tenant is provisioned with **Mode 1 only**. Modes 2 and 3
require a separate opt-in flag and a fresh attestation.

---

## 2. Regulatory framework

EFE is engineered against the following regulations. The mapping below
is the **assertable** subset — i.e. what we can demonstrate in audit.

### 2.1 GDPR — Regulation (EU) 2016/679

| Article | EFE response |
|---|---|
| **Art. 5** (principles) | Purpose limitation enforced via tenant residency + scoped API keys. |
| **Art. 13/14** (information) | This document is the standard information notice ; copy into your privacy policy. |
| **Art. 17** (erasure) | `efe-admin learning gdpr-erase` flushes tenant data + tombstones audit. |
| **Art. 22** (no fully-automated decisions) | **Hard gate** — every Mode-3 call requires `mode3_compliance_gate = true` in `tenants` table. The trigger blocks decisions when the flag is unset. |
| **Art. 25** (data protection by design) | Zero-trust per-tenant isolation (D11) ; data residency pinning (`EU_ONLY` / `US_ONLY`). |
| **Art. 28** (processor obligations) | DPA available under `docs/legal/dpa-template.md` (separate sprint). |
| **Art. 30** (RoPA) | Per-tenant RoPA pack at `docs/compliance/ropa-template.md` (M19). |
| **Art. 32** (security of processing) | Encrypted at rest (KMS BYOK option), TLS 1.3 in transit, audit chain HMAC-SHA-256. |
| **Art. 33** (breach notification) | 72-hour notification pipeline via PagerDuty + email-to-DPO. |
| **Art. 35** (DPIA) | DPIA template `docs/compliance/dpia-template.md` ; required if you enable Mode-2 or Mode-3. |

### 2.2 EU AI Act — Regulation (EU) 2024/1689

| Article | EFE response |
|---|---|
| **Art. 14** (human oversight, high-risk systems) | **Hard gate** — `ai_act_art14_attested = true` required to seed catalogs ; tenants self-classify their risk level. |
| **Art. 9** (high-risk systems risk management) | DPIA template covers risk register + mitigation log. |
| **Art. 13** (transparency) | All Mode-2 / Mode-3 calls receive a `transparency_token` ; downstream apps must surface to end users. |
| **Art. 50** (provider transparency) | EFE is positioned as a **deployer** in your supply chain ; you remain the provider for your application. |

### 2.3 Other frames

- **SOC 2 Type 2** — CC1-CC9 covered ; report available under NDA.
- **ISO 27001** — Annex A mapped ; statement of applicability under `docs/compliance/soa-iso27001.md` (M19).
- **HIPAA** — BAA available for `US_ONLY` + Mode-1 tenants only.
- **CCPA** — Resident-request flow handled via `efe-admin learning gdpr-erase` (same primitive as Art. 17).
- **EU AI Act Code of Practice** — voluntary alignment, tracked in `docs/compliance/eu_ai_code_of_practice.md`.

---

## 3. The mandatory attestation

Before any classification call is accepted, the tenant must have a
**signed attestation** recording the natural person assuming Art. 14 +
Art. 22 conformity.

The SE running the onboarding will execute :

```bash
efe-admin compliance attest acme \
    --ai-act-art14 \
    --gdpr-art22 \
    --signed-by dpo@acme.com \
    --dpo-contact dpo@acme.com \
    --yes
```

You will be asked, **verbally, on the SE handoff call**, to confirm :

1. You have read the present document.
2. You assume Art. 14 and Art. 22 conformity for all classification
   calls issued by the `acme` tenant.
3. The contact email `dpo@acme.com` (or whatever email is set) is a
   working DPO mailbox monitored ≥ 1 business day.

The attestation is **immutable** — it cannot be retracted. It can be
**superseded** by a later attestation signed by a different DPO of
record (e.g. on personnel change). The audit chain shows both
attestations side by side.

> **What signing implies, in plain language** : you confirm that the
> end-user of the classification result is informed (or will be
> informed by your downstream application) that an automated decision
> is being made, and that they retain the right to human review under
> Art. 22.

### What if I haven't read this document yet ?

Stop the SE call. You're not obliged to sign on the spot. Take this
document offline, route it through your legal review, come back when
you're comfortable. The cluster is operational but **classification
calls will reject** until the attestation is signed (HTTP 403 with
`{error: "ai_act_art14_pending"}`).

---

## 4. Your day-2 operations

After onboarding, four primitives matter to you :

### 4.1 Verify the attestation state

```bash
efe-admin compliance verify acme
```

Returns the current `ai_act_art14`, `gdpr_art22` flags + `signed_by` +
`attested_at`. Audit this monthly.

### 4.2 Query the audit log

```bash
efe-admin audit query acme \
    --since 2026-05-01T00:00:00Z \
    --mode MODE_3 \
    --limit 100
```

Every classification event is recorded with :

- `request_id` (Mode-3 multi-turn calls share a `dialog_id`)
- `tenant_id`, `api_key_id`, `mode`, `latency_ms`, `result`
- `chain_position` (HMAC-chained from 0)
- `prev_hmac`, `current_hmac` (BLAKE-256 over canonical fields)

If `chain_position` jumps (e.g. 100 → 102), the gap is a **chain
break** — escalate immediately (see § 5 incidents).

### 4.3 Export for archival

```bash
efe-admin audit export acme \
    --since 2026-05-01T00:00:00Z \
    --to ./archive/acme-2026-05.jsonl
```

Default retention is **7 years** (configurable per tenant). Export is
JSONL with the canonical schema documented in
[`docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md`](../EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md).

### 4.4 Exercise a subject right (Art. 17 erasure)

When a data subject exercises their right to erasure (or CCPA opt-out) :

```bash
# 4.4a. Determine the audit window
efe-admin audit query acme --since <date> --limit 1000 \
    | grep -i "<pseudonymous-id>" > requests.jsonl

# 4.4b. Schedule erasure
efe-admin learning gdpr-erase acme \
    --request-id <CRM-ticket-id> \
    --subject-key <pseudonymous-id> \
    --rationale "Art. 17 § 1(a) — no longer necessary"
```

The cascade fires the **saga** documented in
[`docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md`](../EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md) § 9
(GDPR delete steps : audit_start → drop_sessions → tombstone_audit →
purge_redis → purge_s3 → audit_complete). You receive an email when
the cascade completes.

> **Important** : the audit chain is **tombstoned**, not deleted, to
> preserve regulator-visible integrity. The subject's PII is removed ;
> a SHA-256 anchor remains for chain continuity.

---

## 5. Incidents

| Symptom | What to do | Runbook |
|---|---|---|
| Attestation rejected (HTTP 403 `ai_act_art14_pending`) | Re-attest with full flags | `RUNBOOK_RB_05_MISSING_ATTESTATION.md` |
| Audit chain gap detected | Stop classify, open Sev1 with field-ops | `d13_classify_drift.md` |
| PII leak suspected (regex bypassed) | Switch tenant to Mode-1 only, contact `security@efe.ai` within 72 h (Art. 33) | `general_secret_rotation_playbook.md` |
| Mode-3 surprise behavior | Disable Mode-3 via `efe-admin tenant update acme --mode3-enabled false` | `RUNBOOK_RB_11_MODE3_DISABLE.md` |

For any **suspected personal data breach** : email
`security@efe.ai` + page the on-call DPO (number on the contract).
You have 72 h to notify the supervisory authority — the EFE side of
the joint controller agreement is documented in the DPA.

---

## 6. Data residency

When the tenant is created with `--data-residency EU_ONLY`, the
following guarantees apply :

- Postgres primary is in an EU region (eu-west-3 by default).
- Postgres replicas are in EU regions only.
- Redis instances run in the same EU region.
- Audit S3 bucket is in `eu-west-3` with cross-region replication
  **disabled**.
- LLM calls (Mode-2 / Mode-3) route via the EU LLM router pool only
  (cf. `crates/efe-llm-router/`) — never US OpenAI endpoints.

You can verify residency post-deploy with :

```bash
efe-admin tenant get acme | grep residency
```

> Changing residency post-creation requires a **tenant migration** —
> contact `support@efe.ai`. It is not a runtime toggle.

---

## 7. Sub-processor list (per-tenant)

EFE deployments use a minimal sub-processor set, all listed in the DPA
addendum :

| Sub-processor | Purpose | Region | Optional ? |
|---|---|---|:---:|
| AWS / GCP / Azure | IaaS (compute, storage, KMS) | Per tenant residency | Required |
| Anthropic / OpenAI / Together AI | LLM routing (Mode-2 / Mode-3) | EU / US — routed per residency | Optional (disable Mode-2/3) |
| Sentry | Error reporting | EU | Optional (`--telemetry off`) |
| Grafana Cloud | Observability | EU | Optional (self-host instead) |
| Stripe | Billing (post-GA) | US (Stripe HQ) | Optional (`--billing manual`) |

The full per-tenant sub-processor pack is generated at provisioning
time and stored under `s3://efe-audit-<tenant>/subprocessors.json`.

---

## 8. Questions you should ask the SE

Before signing the attestation, confirm with the SE :

1. ✅ What region is the Postgres primary in ? (residency proof)
2. ✅ Is Mode-3 enabled ? (Art. 22 hard gate active ?)
3. ✅ What's the audit retention configured to ? (default 7 years)
4. ✅ Who has admin scope (`efe:admin:*`) ? (typically only your team)
5. ✅ Is the key revocation tested ? (ask the SE to demo `keys revoke`
   then `keys list --all`)
6. ✅ Is the PII pipeline enabled and tested on a sample sentence ?
7. ✅ What is the breach-notification SLA ? (72 h, contractual)

If the SE cannot answer any of these, **do not sign** — request a
follow-up call with the field-ops lead.

---

## 9. Glossary

- **Attestation** — Cryptographically-recorded statement by a DPO that the tenant assumes Art. 14 + Art. 22 conformity. Immutable.
- **Audit chain** — Sequence of events HMAC-SHA-256 chained from event 0. Tamper-evident.
- **Catalog** — Set of anchors (labelled vectors) the classification engine matches against.
- **DPIA** — Data Protection Impact Assessment, required by GDPR Art. 35 for high-risk processing.
- **Hard gate** — Postgres-level trigger that rejects writes / reads when a precondition is unmet. Not bypassable by the application layer.
- **Mode 1 / 2 / 3** — Classification modes with increasing LLM involvement and data sensitivity.
- **PII pipeline** — Regex + circuit-breaker layer that masks personal data before storage.
- **Residency** — Geographic constraint on data storage and processing.
- **Saga** — Distributed transaction with compensating actions, used for GDPR Art. 17 cascades.
- **Tombstone** — Audit record marking a deletion without removing the chain link.

---

## 10. Reference documents

For deeper reading :

| Topic | Document |
|---|---|
| Full compliance posture | [`docs/EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md`](../EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md) |
| Data rights (Art. 17, CCPA) | [`docs/EFE_M29_DATA_RIGHTS_ARCHITECTURE.md`](../EFE_M29_DATA_RIGHTS_ARCHITECTURE.md) |
| Audit chain HMAC scheme | [`docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md`](../EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md) |
| Active learning + DPO consent | [`docs/EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md`](../EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md) |
| Agent runtime Art. 9 mapping | [`docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md`](../EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md) |
| Self-service onboarding flow | [`docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md`](../EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md) |

---

## 11. Contact

- **DPO operations** — `dpo-ops@efe.ai` (response ≤ 1 business day)
- **Security incidents** — `security@efe.ai` + page on-call (24/7)
- **Privacy questions** — `privacy@efe.ai`
- **Public DPO contact** — `dpo@efe.ai` (per Art. 37 § 7)

If your supervisory authority issues a regulatory request, forward it
to `dpo-ops@efe.ai` with full headers — EFE will participate in
responding under joint-controller terms.

— **fin du document** —

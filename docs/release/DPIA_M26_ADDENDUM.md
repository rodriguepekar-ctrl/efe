# DPIA M26 Addendum — Production Graduation

**Supplements** : `docs/release/DPIA_M25_TEMPLATE.md`.
**Version** : 2.1.0.
**Status** : MANDATORY for any tenant moving from EXPERIMENTAL → STABLE.
**Re-sign cadence** : 12 months (replaces M25 cadence).

---

## Section 1 — Identification (additions)

| Field | Value |
|---|---|
| Channel | `EXPERIMENTAL` or `STABLE` (set by `efe-admin agent graduate confirm`) |
| Graduation acknowledged at | `<YYYY-MM-DD>` |
| Caliper Steering Committee contact | `csc@efe.example.com` |

## Section 2 — Description of processing (M26 deltas)

The following M26 changes augment the M25 description :

- **REST gateway** (`/v1/agent/*` on `efe-server`) — public-facing surface
  ; OIDC bearer auth ; per-subject rate limit. Tenant data flows through
  this gateway from portal-admin and from operator CLIs.
- **Admission webhook** — validates `AgentRun` and `AgentRunTemplate`
  CRs at K8s API ingress. Reads `tenant_feature` (DPIA + flags) only ;
  no write operations.
- **vsock-relayer** — privileged DaemonSet bridging in-microVM HTTP
  calls to in-cluster services. mTLS + JWT validation ; per-microVM
  rate limit ; audit relay.
- **Tenant graduation workflow** — captures the production-readiness
  acknowledgement signed by the tenant security lead (separate from
  the DPIA itself, which is signed by the DPO).
- **AI Act Art. 26 quarterly post-market monitoring** — tenant-scoped
  JSON-LD report submitted within 30 days of each calendar quarter
  end. Counter-signed by the DPO.

## Section 3 — Risk assessment additions

| Risk | Likelihood | Severity | Mitigation |
|---|:---:|:---:|---|
| REST gateway forgery (forged OIDC JWT) | low | high | OIDC ES256 + audience binding + per-subject rate limit |
| Webhook bypass (direct etcd CR creation) | very low | high | `failurePolicy: Fail` + controller runtime checks (defense-in-depth) |
| vsock-relayer compromise (token extraction) | low | medium | per-microVM rate limit + JWT pinning + relayer pod security context strict |
| Cosign signature replay (old vulnerable image) | very low | high | Rekor inclusion proof verified at boot ; image whitelist `min_signed_at` enforced |
| Per-tenant namespace escape | very low | high | NetworkPolicies enforce ns isolation ; Pod Security Standards = `restricted` |

## Section 4 — Lawful basis (unchanged)

Inherits from M25.

## Section 5 — Data subject rights (M26 enhancements)

- **Art. 17 cascade with pseudonymization** : after the 7-year retention
  window, audit chain rows are pseudonymized rather than deleted (chain
  integrity preserved ; `actor_id` and personal-data fields in
  `payload_json` are replaced with deterministic hashes).
- **Art. 22 (automated decision-making)** : the agent runtime never
  makes automated decisions affecting data subjects. Write tools require
  per-call human approval (Art. 14). M26 webhook enforces the
  `require_approval_for_writes: true` invariant on all templates.
- **Art. 26 transparency** : the post-market quarterly report is shared
  with the customer's DPO ; the tenant operator can request the report
  via `efe-admin agent post-market export --tenant <slug> --quarter ...`.

## Section 6 — DPO sign-off (split per channel)

### 6.1 DPO sign-off — EXPERIMENTAL

(Inherited from M25 ; no change.)

### 6.2 DPO sign-off — STABLE graduation

I, the undersigned DPO, confirm that I have :

1. Reviewed the M26 architecture document
   (`docs/EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md`) including
   the threat model update (§ 26).
2. Reviewed the runbook
   (`docs/runbooks/M26_AGENT_RUNTIME_PRODUCTION_RUNBOOK.md`) and the
   DR playbook (`docs/runbooks/M26_DR_PLAYBOOK.md`).
3. Read at least one quarterly post-market monitoring report for this
   tenant (Art. 26).
4. Verified the audit chain via `efe-admin audit verify <tenant>
   --module=agent_runtime`.
5. Reviewed the most recent pen test report for this tenant (or the
   cluster-wide report covering this tenant's templates).
6. Acknowledged that the STABLE channel commits the operator to
   monthly Caliper Steering Committee participation for the first
   12 months post-graduation.

**DPO** : `<name>` `<email>`
**Date** : `<YYYY-MM-DD>`
**SHA-256 of signed PDF** : `sha256:<computed-after-signing>`

## Section 7 — Tenant security lead acknowledgement (NEW for M26)

In addition to DPO sign-off, the tenant's security lead signs a
**production graduation acknowledgement** confirming :

1. The tenant has run M25 EXPERIMENTAL for ≥ 30 days.
2. The tenant has run ≥ 100 successful agent runs.
3. The tenant has zero `sandbox_escape_attempt_detected` events in the
   last 90 days.
4. The tenant's failure rate is < 5 % over the last 30 days.
5. The tenant has set up :
   - At least 2 named operators with `agent_run.create` role.
   - At least 1 named operator with `agent_run.terminate` role.
   - A backup operator on-call rotation.
   - Approval response SLA (1 h Pro / 30 min Enterprise).
6. The tenant's compliance team will participate in the quarterly
   audit chain review.

**Security lead** : `<name>` `<email>`
**Date** : `<YYYY-MM-DD>`
**Acknowledgement SHA-256** (passed to `efe-admin agent graduate confirm`) : `sha256:<computed>`

## Section 8 — Annual re-review checklist additions

At T+12 months post-graduation, the DPO must :
- [ ] Re-confirm risk assessment items in Section 3.
- [ ] Review the past 4 quarterly Art. 26 post-market reports.
- [ ] Run `efe-admin audit verify` and attach the outcome.
- [ ] Review the past year's pen test findings and resolution status.
- [ ] Review the past year's CSC meeting notes for risk factors.
- [ ] Re-sign with `efe-admin agent dpia refresh --tenant <slug> --confirm`.

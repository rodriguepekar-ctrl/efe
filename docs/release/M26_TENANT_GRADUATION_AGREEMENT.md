# M26 Tenant Graduation Agreement — EXPERIMENTAL → STABLE

**Date** : `<YYYY-MM-DD>`
**Tenant slug** : `<slug>`
**Tenant legal entity** : `<entity>`
**Effective** : on signature + `efe-admin agent graduate confirm`
**Term** : 12 months, auto-renewing on annual DPIA + post-market re-review

---

## 1. Purpose

This agreement governs the migration of the agent runtime feature
("Caliper") for the named tenant from the EXPERIMENTAL channel
(design-partner pilot, no SLA commitments) to the STABLE channel
(production with SLAs and pricing).

## 2. SLA commitments

| Metric | Pro tier | Enterprise tier |
|---|---|---|
| Run provisioning P99 | < 5 s | < 5 s |
| Approval response P95 | < 1 h | < 30 min |
| Cost cap → SIGTERM P99 | < 60 s | < 60 s |
| API availability (REST gateway) | ≥ 99.5 % | ≥ 99.5 % |
| Admission webhook availability | ≥ 99.9 % | ≥ 99.9 % |
| MTTR for P0 incidents | < 30 min | < 30 min |
| MTTR for P1 incidents | < 4 h | < 2 h |

Service credits per the Master Service Agreement § 8 apply when SLAs
are missed for two consecutive months.

## 3. Pricing

| Tier | Monthly fee | Included runs | Overage |
|---|---|---|---|
| Pro | $299 | 100 | $0.50/run |
| Enterprise | $999 | 1,000 | $0.20/run |

The first **90 days** post-graduation are free (early-adopter benefit).

## 4. Operational responsibilities

The tenant operator commits to :

1. Maintaining a current DPIA addendum (re-signed every 12 months).
2. Maintaining at least 2 named operators with `agent_run.create` role
   and 1 with `agent_run.terminate` role.
3. Responding to approval requests within the SLA (Section 2).
4. Notifying EFE within 24 h of any internal security incident affecting
   the agent runtime (e.g. compromised operator account).
5. Participating in monthly Caliper Steering Committee meetings during
   the first 12 months post-graduation.
6. Submitting a quarterly self-attestation (1-page form) confirming
   continued compliance with operational responsibilities.

## 5. EFE responsibilities

EFE commits to :

1. Maintaining the SLAs in Section 2.
2. Running the cluster-wide pen test bundle quarterly + sharing
   tenant-relevant findings.
3. Maintaining the cosign + Rekor signature pipeline.
4. Running quarterly DR drills.
5. Submitting the AI Act Art. 26 quarterly post-market monitoring report
   countersigned by the DPO.
6. Notifying the tenant within 4 h of any P0 incident affecting their
   data or runs.

## 6. Termination

Either party may terminate this agreement with 90-day notice. Upon
termination :
- New AgentRuns are blocked.
- In-flight runs are allowed to complete.
- A 30-day "data retrieval" window allows the tenant to export their
  audit chain + post-market reports.
- After the retrieval window, the tenant's data is deleted per the
  GDPR Art. 17 cascade procedure.

## 7. Sunset clause

The STABLE channel itself is subject to the M26 sunset clause :
**< 3 active tenants at 24 months OR aggregate run volume < 10K/quarter
at 18 months** triggers a CSC review for phased deprecation. If
deprecation is approved, the tenant gets a 12-month migration window.

## 8. Signatures

**Tenant security lead** : `<name>`
Signature : `<wet ink or e-signature>`
Date : `<YYYY-MM-DD>`

**Tenant DPO** : `<name>`
Signature : `<wet ink or e-signature>`
Date : `<YYYY-MM-DD>`

**EFE engineering lead** : `<name>`
Signature : `<wet ink or e-signature>`
Date : `<YYYY-MM-DD>`

**EFE DPO** : `<name>`
Signature : `<wet ink or e-signature>`
Date : `<YYYY-MM-DD>`

---

## 9. Acknowledgement hash

After signing, compute :

```bash
sha256sum signed-agreement.pdf
```

Pass to `efe-admin agent graduate confirm --tenant <slug> --acknowledgement-hash sha256:<hex> --confirm`.
The hash is recorded in `tenant_feature.m26_graduation_acknowledgement_hash`
and surfaces in the auditor portal.

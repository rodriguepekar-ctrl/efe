# DPIA Addendum — M25 Agent Runtime

**Template version** : 2.0.0 (matches EFE 2.0).
**Last reviewed** : 2027-Q4.
**Mandatory** : YES — required before any tenant can create AgentRuns.
**Re-sign cadence** : every 12 months.

---

> **Instructions for DPOs** : print this template, fill in the per-tenant
> sections, sign, scan to PDF, compute SHA-256 of the PDF
> (`sha256sum signed.pdf`), then run :
>
> ```bash
> efe-admin agent dpia sign \
>   --tenant <slug> \
>   --addendum-hash sha256:<64-hex-from-sha256sum> \
>   --valid-days 365 --confirm
> ```
>
> Keep the signed PDF in your compliance vault. EFE auditors will request
> proof during the annual review (R-100 compliance pack).

---

## Section 1 — Identification

| Field | Value |
|---|---|
| Tenant slug | `<fill>` |
| Tenant legal entity | `<fill>` |
| Data Protection Officer | `<name + email>` |
| Effective date | `<YYYY-MM-DD>` |
| Expiry (12 months from sign) | `<YYYY-MM-DD>` |
| EFE platform version | `2.0.0` |

## Section 2 — Description of processing

The EFE M25 agent runtime introduces an **autonomous, LLM-driven agent**
sandboxed in a per-run Firecracker microVM. The agent executes
operator-defined tasks (`AgentRunTemplate`) and may invoke a curated,
operator-controlled set of tools (`tool_whitelist`). Mutating tools
require **per-call human approval** (Art. 14 of EU AI Act).

**Categories of personal data processed** :
- (Optional, tenant-controlled) data accessed by the agent's tools
  during run execution. The tenant operator selects which tools the
  agent may call ; this DPIA addendum applies to the maximal scope
  defined by the tenant's `m25_allowed_profiles`.
- Agent-core internal logs (LLM messages) — pre-redacted by the
  agent-core PII redactor + the M21 LLMRouter DLP layer.
- Audit log of all run lifecycle events.

**Categories of data subjects** :
- (Indirect) end-users whose data the operator's tools may access.
- Tenant operators (subject + role + IP, in the audit log).

**Recipients** :
- LLM provider (per `default_provider` ; OpenAI / Anthropic / …).
- Storage : tenant's EFE Postgres (RLS-isolated).

## Section 3 — Risk assessment

| Risk | Likelihood | Severity | Mitigation |
|---|:---:|:---:|---|
| Agent exfiltrates personal data via LLM API | low | high | LLM router DLP + agent-core PII redactor + tool whitelist + NetworkPolicy egress restrictions |
| Agent escapes sandbox | very low | critical | Firecracker microVM (KVM-isolated) + seccomp + read-only rootfs + cosign-signed images + image whitelist |
| Agent hallucinates tool calls | medium | low | 3-layer whitelist enforcement (control plane + agent core + mcp-server) ; non-whitelisted calls rejected + audited |
| Operator approves a harmful write inadvertently | medium | medium | Per-call approval + ≥ 10 char rationale + 24 h expiry + single-use tokens |
| Cost runaway | medium | low | Per-run hard cap (SIGTERM at cap) + tenant daily budget + watchdog every 5 s |
| Audit log tampered | very low | critical | Per-tenant HMAC chain + automated `efe-admin audit verify` cron |

## Section 4 — Lawful basis (GDPR Art. 6)

[ ] (a) Consent of the data subject
[ ] (b) Performance of a contract
[ ] (c) Compliance with a legal obligation
[ ] (d) Vital interests
[ ] (e) Public interest
[X] (f) Legitimate interests pursued by the controller

If (f) — describe the legitimate interest balancing test :

> `<DPO fills>`

## Section 5 — Data subject rights

The tenant declares :
- [X] Right of access (Art. 15) supported via `efe-admin audit list --tenant <slug> --module=agent_runtime --subject <email>`
- [X] Right to erasure (Art. 17) supported via cascade purge of `agent_run`, `agent_step`, `agent_state`, `agent_memory`, with audit chain rows masked but retained for compliance
- [X] Right to object (Art. 21) — operator can disable M25 per tenant at any time by flipping `m25_agent_runtime_enabled = FALSE`
- [X] Right not to be subject to automated decision-making (Art. 22) — the agent does NOT make decisions affecting data subjects without prior operator approval (writes blocked until approval token granted)

## Section 6 — DPO sign-off

I, the undersigned, confirm that I have :
1. Reviewed the M25 architecture document
   (`docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md`).
2. Reviewed the runbook (`docs/runbooks/M25_AGENT_RUNTIME_RUNBOOK.md`).
3. Performed the risk assessment in Section 3.
4. Confirmed the tenant's profile + provider allow-list is appropriate.

**Signed** : `<DPO signature>`
**Date** : `<YYYY-MM-DD>`
**SHA-256 of signed PDF** : `sha256:<computed-after-signing>`

---

## Section 7 — Required attachments

- [ ] Signed PDF version of this template
- [ ] Tenant's `AgentRunTemplate` definitions (YAML)
- [ ] Tenant's `tenant_feature.m25_*` settings (CSV export of relevant columns)
- [ ] Tenant's allowed-tool list (subset of profile defaults, if narrowed)

## Section 8 — Annual re-review checklist

At T+12 months, the DPO must :
- [ ] Re-confirm risk assessment items in Section 3.
- [ ] Review actual-vs-allowed scope (any unexpected provider switches?).
- [ ] Run `efe-admin audit verify` and attach the outcome.
- [ ] Re-sign with `efe-admin agent dpia refresh --tenant <slug> --confirm`.

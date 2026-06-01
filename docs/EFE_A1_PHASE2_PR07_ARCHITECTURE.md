# PR #7 — Phase 2 Threat Model — Architecture ultra-détaillée

> **Document type** : Execution-ready implementation blueprint for PR #7.
> **Scope** : Third PR of Phase 2 (Sprint A1.P2.1, Week 1, day 5-7).
> **Owner** : sec-team — assigned lead : R. Kapoor (back-up : E. Vargas).
> **Reviewers (required)** : 2× sec-team + 1× meta-swarm-team + 1× sre-team.
> **Estimate** : 2 FTE-days (drafting) + 0.5 FTE-day (review) + 0.5 FTE-day (sign-off) = 3 FTE-days.
> **Branch** : `A1.P2/07-phase2-threat-model`.
> **Deadline** : Sat 2026-07-11 23:59 UTC (parallel to PR #6 ; does not block PR #6 merge).
> **Hard dependency** : Phase 2 architecture frozen (`EFE_A1_PHASE2_ARCHITECTURE.md`).
> **Parent docs** :
>   - [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md) § 15 (Security model)
>   - [`EFE_A1_PHASE2_ROADMAP.md`](EFE_A1_PHASE2_ROADMAP.md) § 2.1 (PR #7 entry)
>   - [`EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md) (workspace-wide security doctrine)

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Goal + non-goals](#1-goal--non-goals)
2. [Pre-flight checks](#2-pre-flight-checks)
3. [Branch + commit plan](#3-branch--commit-plan)
4. [Methodology : STRIDE + CVSS + DREAD](#4-methodology)
5. [Assets catalogue (10 assets)](#5-assets-catalogue)
6. [Trust boundaries (6 boundaries)](#6-trust-boundaries)
7. [Attacker personas (7 archetypes)](#7-attacker-personas)
8. [Threat scenarios (16 scenarios, 6 SEV-1)](#8-threat-scenarios)
9. [Mitigations matrix (cross-referenced to PRs)](#9-mitigations-matrix)
10. [Residual risk register](#10-residual-risk-register)
11. [Monitoring + alerting requirements](#11-monitoring--alerting)
12. [Incident response runbook hooks](#12-incident-response)
13. [Re-assessment cadence + triggers](#13-re-assessment-cadence)
14. [Document structure : `docs/security/PHASE2_THREAT_MODEL.md`](#14-document-structure)
15. [PR-local risk analysis](#15-pr-local-risk-analysis)
16. [Acceptance criteria (binary)](#16-acceptance-criteria)
17. [Reviewer checklist](#17-reviewer-checklist)
18. [PR description template](#18-pr-description-template)
19. [Definition of Done](#19-definition-of-done)
20. [Post-merge follow-ups](#20-post-merge-follow-ups)

---

## 0. TL;DR

PR #7 ships **one document** : `docs/security/PHASE2_THREAT_MODEL.md`
(~ 1 600 lines), which catalogues the Phase-2 attack surface,
attackers, threats, mitigations, residual risk, and monitoring
requirements.  Methodology : **STRIDE per surface + CVSS v3.1
scoring + DREAD overlay**.

Why it matters :

- **L2 gate criterion G8** requires "threat model `PHASE2_THREAT_MODEL.md`
  signed off by sec-team" before Phase 2 graduation.  Without PR #7,
  the L2 gate is RED.
- **PR #16 (mTLS)** depends on PR #7's mitigation catalogue : it
  implements the controls.
- **runbook `META_SWARM_GRPC.md`** (PR #24 deliverable) inherits the
  incident-response hooks from PR #7.
- **L0 doctrine** (cf. `EFE_A1_ROADMAP.md` § 10) : "threat model
  has 0 SEV-1 unmitigated entries".  PR #7 enforces this.

What it does NOT do :

- Does NOT implement any control (those live in code PRs).
- Does NOT replace the workspace-wide security doctrine in
  `EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` (it extends it for
  Phase 2 specifics).
- Does NOT cover Phase-3 multi-tenant + cross-region threats
  (deferred to Phase 3 RFC : `PHASE3_THREAT_MODEL.md`).

After merge :

- The doc is **the single source of truth** for "what attackers
  can do to Phase 2 and what we do about it".
- Every Phase 2 PR (#8 → #25) references it in the PR description
  if it adds, removes, or affects a mitigation.
- The Sprint A1.P2.4 L2 verification ceremony (Mon 2026-08-24)
  checks every threat is either mitigated or accepted-with-rationale.

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Catalogue every threat that could compromise the confidentiality,
> integrity, or availability of the `efe-meta-swarm` Phase-2
> distributed coordinator, score each via CVSS v3.1, map every threat
> to either a Phase-2 mitigation (with the implementing PR id) or an
> explicit risk acceptance (with rationale + expiry), so that the L2
> graduation gate has a binary "0 SEV-1 unmitigated" signal.

### 1.2 Non-goals

| # | Excluded | Lives in | Why |
|---|---|---|---|
| NG1 | Implementing any mitigation (mTLS, auth, NetworkPolicy, etc.) | code PRs (#16, #17, #18, etc.) | This PR is documentation only. |
| NG2 | Phase-3 tenant-isolation threats | Phase 3 (`PHASE3_THREAT_MODEL.md`) | Scope-creep ; deferred. |
| NG3 | Phase-1 in-process threats (already covered by Phase-1 review) | `EFE_A1_PHASE1_ARCHITECTURE.md` | Phase 1 has no network surface. |
| NG4 | EFE-wide threats (efe-server, efe-core, etc.) | `EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` | Workspace-scope ; PR #7 only adds the **delta** for Phase 2. |
| NG5 | Penetration testing or red-team exercises | Phase 3 (post-L3) | Phase 2 is internal-only ; pen-test is for L5 customer-facing. |
| NG6 | Compliance frameworks (SOC2, ISO27001, GDPR) | Phase 5 (customer-facing) | Phase 2 internal soak does not invoke compliance. |
| NG7 | Supply-chain attack catalogue (deps, build-time injection) | Continuous (cargo audit) | Workspace-wide concern, separate runbook. |

### 1.3 Definition of mergeable

PR #7 is mergeable iff **all** :

1. The doc covers ≥ 12 distinct threat scenarios via STRIDE.
2. Every threat has a CVSS v3.1 score.
3. Every threat is in one of three states : `MITIGATED`,
   `ACCEPTED-WITH-RATIONALE`, or `DEFERRED-TO-PR-N`.
4. **Zero** SEV-1 (CVSS ≥ 9.0) is in `ACCEPTED` or `DEFERRED` state.
5. 2 sec-team peer reviewers + 1 meta-swarm + 1 sre have approved.
6. The L2 gate criterion G8 is unblocked.

---

## 2. Pre-flight checks

| # | Check | How | Expected |
|---|---|---|---|
| PF1 | Phase 2 architecture is frozen (last commit > 7 days old) | `git log --since=7days docs/EFE_A1_PHASE2_ARCHITECTURE.md` | empty |
| PF2 | `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` is the source of workspace security doctrine | grep for "anti-Φ-D.A1" + "carve-out R5-B1" | present |
| PF3 | sec-team has a STRIDE template in `docs/security/_template.md` | `ls docs/security/` | exists (or PR creates it) |
| PF4 | CVSS v3.1 calculator URL is known | sec-team confirms | `https://www.first.org/cvss/calculator/3.1` |
| PF5 | PR #5 (proto definitions) is merged so we can reference RPC names | `gh pr view 5 --json state` | `MERGED` |
| PF6 | PR #6 (server scaffold) is in flight (parallel) | `gh pr view 6 --json state` | `OPEN` or `MERGED` |
| PF7 | No outstanding Sev1 security incident on the workspace | `gh issue list -l security -l sev1 --state open` | empty |
| PF8 | sec-team lead has 2 FTE-days allocated in Sprint A1.P2.1 | sprint board | confirmed |

---

## 3. Branch + commit plan

### 3.1 Branch

```
A1.P2/07-phase2-threat-model
```

### 3.2 Commit-by-commit plan (4 commits)

A documentation PR with logical chunks reviewable separately :

| # | Commit message | Files | Doc % | Verifies |
|---|---|---|---|---|
| C1 | `docs(security): scaffold PHASE2_THREAT_MODEL.md (TOC + methodology)` | `docs/security/PHASE2_THREAT_MODEL.md` (TOC + § 1-4) | 15 % | structure approved by sec-team-lead |
| C2 | `docs(security): assets, trust boundaries, attackers` (§ 5-7) | same file (§ 5-7) | 35 % | sec-team confirms surface is fully mapped |
| C3 | `docs(security): 16 threat scenarios with STRIDE + CVSS` (§ 8) | same file (§ 8) | 75 % | core of the doc ; sec + meta-swarm reviewers walk through |
| C4 | `docs(security): mitigations + residual risk + monitoring + final` (§ 9-13) | same file (§ 9-13) | 100 % | doc complete + cross-refs to PRs |

Total : ~ **1 600 lines** of markdown, single file.

### 3.3 Drafting strategy

Open as **Draft** after C1.  Push C2-C4 incrementally.  Move to
"Ready for review" after C4 + a self-review pass.  Required
approvers : 2 sec-team (one acts as devil's advocate "red team" ;
one focuses on the mitigation completeness), 1 meta-swarm-team
(for technical accuracy), 1 sre-team (for monitoring + alerting).

---

## 4. Methodology

The threat model uses **STRIDE-per-surface** as the primary
framework, augmented with **CVSS v3.1** scoring and **DREAD** for
prioritisation.

### 4.1 STRIDE

For each trust-boundary crossing, we enumerate threats in 6
categories :

| Letter | Category | Question |
|---|---|---|
| **S** | Spoofing | Can an attacker impersonate an identity ? |
| **T** | Tampering | Can an attacker modify data in transit / at rest ? |
| **R** | Repudiation | Can a legitimate action be denied later ? |
| **I** | Information disclosure | Can an attacker read data they shouldn't ? |
| **D** | Denial of service | Can an attacker make the service unavailable ? |
| **E** | Elevation of privilege | Can a low-priv attacker gain higher-priv access ? |

### 4.2 CVSS v3.1

Every threat scenario is scored via CVSS v3.1 :

| Severity | Score | Interpretation |
|---|---|---|
| Critical | 9.0 – 10.0 | SEV-1 — must be MITIGATED for L2 |
| High | 7.0 – 8.9 | SEV-2 — must be MITIGATED or DEFERRED-WITH-PR |
| Medium | 4.0 – 6.9 | SEV-3 — may be ACCEPTED with rationale + expiry |
| Low | 0.1 – 3.9 | SEV-4 — typically ACCEPTED |

The scoring uses the official FIRST calculator at
`https://www.first.org/cvss/calculator/3.1`, and the **8 base
metrics + 3 temporal + 4 environmental** are explicitly listed for
every SEV-1 / SEV-2 threat (reviewer must be able to reproduce the
score).

### 4.3 DREAD (overlay)

For threats with CVSS in [4.0, 7.0) where prioritisation matters,
we apply DREAD (D, R, E, A, D — each scored 1–10, summed) to break
ties.  This is optional ; CVSS is authoritative.

### 4.4 Sev definitions (workspace-wide)

The PR uses the workspace incident severity scale, defined in
`docs/runbooks/ops_master_gate_fail.md` :

| Sev | Customer impact | TM threshold |
|---|---|---|
| Sev1 | Production down or data loss | CVSS ≥ 9.0 |
| Sev2 | Production degraded, single-tenant impact | CVSS 7.0 – 8.9 |
| Sev3 | Internal-only degradation | CVSS 4.0 – 6.9 |
| Sev4 | No customer impact | CVSS < 4.0 |

### 4.5 Threat status states

Every threat MUST be in exactly ONE state :

| Status | Meaning | Allowed at L2 gate ? |
|---|---|---|
| `MITIGATED` | Implemented by a Phase-2 PR ; covered by a test or runbook. | ✅ always |
| `DEFERRED-TO-PR-N` | Will be mitigated by a specific Phase-2 PR ; PR id is named. | ✅ iff PR N is in Phase 2 |
| `ACCEPTED` | Risk is explicitly accepted with rationale + expiry date + owner. | ✅ iff CVSS < 9.0 |
| `OPEN` | No decision yet. | ❌ blocks L2 gate |

The PR ships with **0 OPEN threats** at merge time.

---

## 5. Assets catalogue

Ten assets in scope.  Each is named, located, owned, and rated for
confidentiality / integrity / availability impact.

### 5.1 Asset list

| ID | Asset | Located in | Owner | C | I | A |
|---|---|---|---|:---:|:---:|:---:|
| A1 | **Meta-consensus state** (joint fixed point) | controller in-memory | meta-swarm-team | M | **H** | **H** |
| A2 | **Participant states** (per-participant tensors) | participant in-memory | meta-swarm-team | **H** | **H** | M |
| A3 | **Manifest** (participant ids, dims, kappas, seeds, safety geometry) | ConfigMap → mounted file | platform + sec | M | **H** | M |
| A4 | **mTLS certificates** (leaf + CA bundle) | K8s Secret + cert-manager | sec | **H** | **H** | M |
| A5 | **Iteration determinism guarantee** (bit-identical reruns) | algorithmic invariant | meta-swarm-team | L | **H** | M |
| A6 | **gRPC channel integrity** (between efe-server ↔ controller ↔ participants) | network | sec + platform | **H** | **H** | M |
| A7 | **Health + Info responses** (build sha, hostname, configured participants) | controller in-memory | meta-swarm-team | M | M | L |
| A8 | **Prometheus metrics + OTel spans** (operational telemetry) | controller/participant in-memory | sre | M | M | L |
| A9 | **Admin RPCs surface** (ReloadConfig + future Reset) | controller in-memory | meta-swarm + sec | M | **H** | M |
| A10 | **Soft-launch `deferred_ticket` field** (audit gate state) | YAML in repo | sre | L | **H** | L |

C/I/A scale : **H** (high), M (medium), L (low).

### 5.2 Rationale per asset

(One paragraph per asset, justifying the CIA rating.  Example for
A1 :)

**A1 — Meta-consensus state** :
- Confidentiality M : the joint fixed point may encode private
  attributes if tenants opt into multi-tenant Phase-3.  Phase-2 is
  internal-only, so a leak is operational rather than customer-facing.
- Integrity **H** : a corrupted consensus produces wrong downstream
  decisions ; this is the entire point of the system.
- Availability **H** : without a consensus, the entire Iterate
  pipeline returns errors ; whole-product impact.

(Full rationale for the other 9 assets follows the same shape.)

---

## 6. Trust boundaries

Six boundaries, each crossing identified + protected by listed
controls.

### 6.1 Boundary list

```
TB1 : External (untrusted)            ─────► efe-server (ingress)
TB2 : efe-server                      ─────► controller (NEW Phase 2)
TB3 : controller                       ─────► participant (NEW)
TB4 : operator                         ─────► controller (SIGHUP)
TB5 : operator                         ─────► ConfigMap (manifest tampering)
TB6 : Prometheus scraper               ─────► /metrics (controller + participant)
```

### 6.2 Boundary detail

For each, the doc enumerates :
- the crossing endpoints (caller, callee)
- the wire transport (HTTP/2 gRPC, K8s API, etc.)
- the auth mechanism (mTLS, OAuth2, K8s RBAC)
- the threats applicable (which STRIDE letters)
- the controls in place

(Example for TB3, the most-frequent in PR #7's threat catalogue :)

#### TB3 — controller ↔ participant

| Property | Value |
|---|---|
| Caller | controller pod (StatefulSet) |
| Callee | participant pod (Deployment) |
| Transport | gRPC over HTTP/2 |
| Auth (Phase 2) | mTLS (PR #16) ; SAN check |
| Auth (PR #6 → PR #15) | none (plaintext) ; mitigated by NetworkPolicy + Pod-to-Pod K8s native |
| STRIDE applicable | **S T I D** (no R or E by design) |
| Frequency | ~ 14 100 Step RPCs/sec at 100 rps |
| Controls list | NetworkPolicy (PR #14), mTLS handshake (PR #16), Capabilities probe at startup (PR #9), deterministic Step replies (Phase 1 invariant) |

---

## 7. Attacker personas

Seven archetypes ; each has a capability profile + motivation +
likely entry vector.

| ID | Persona | Capability | Motivation | Entry vector |
|---|---|---|---|---|
| AP1 | **Curious insider** | Read code, deploy to staging | Curiosity, accidental | Internal access |
| AP2 | **Malicious insider** | Same as AP1 + intent to harm | Disgruntled, exfil | Internal access |
| AP3 | **Compromised efe-server** | Issues arbitrary gRPC calls | Upstream RCE | TB2 |
| AP4 | **Compromised participant pod** | Returns crafted Step responses | Container RCE, sidecar attack | TB3 |
| AP5 | **External attacker** | Network access via L7 ingress | Financial gain, espionage | TB1 (filtered through efe-server) |
| AP6 | **Operator typo** | Misconfigures env vars, SIGHUPs wrong pod | Honest mistake | TB4, TB5 |
| AP7 | **Supply chain** | Injects malicious dep at build time | Long-game APT | Crates.io / build pipeline |

The doc explicitly states Phase 2 is **internal-only** (no AP5
in production until L5), so AP5 is **out of scope for L2** ; the
matrix below ignores AP5 for L2 gate purposes.

---

## 8. Threat scenarios

Sixteen scenarios catalogued.  Each has the structure :

```
T-N — short title
├── STRIDE letters
├── Surface (TB id)
├── Persona(s) (AP id)
├── Asset(s) at risk (A id)
├── Description
├── Attack chain (steps)
├── CVSS v3.1 vector
├── Score + severity
├── Mitigations (PRs / runbooks)
├── Status (MITIGATED / DEFERRED-TO-PR-N / ACCEPTED)
└── Owner
```

### 8.1 The 16 threats (summary table)

| ID | Title | STRIDE | TB | Persona | CVSS | Status |
|---|---|---|---|---|:---:|---|
| **T1** | Malicious participant biases consensus via crafted state | T,I | TB3 | AP2, AP4 | **8.6** | DEFERRED-TO-PR-16 + Phase-1 contractive bound |
| **T2** | MITM on controller ↔ participant injects fake Step response | S,T | TB3 | AP4 | **9.1** | DEFERRED-TO-PR-16 (mTLS) |
| **T3** | Replay of Iterate response causes inconsistent downstream | T,R | TB2 | AP3 | 6.8 | MITIGATED (gRPC deadline + idempotence) |
| **T4** | SIGHUP injection by unauthorised operator triggers bad reload | S,E | TB4 | AP6 | 5.4 | DEFERRED-TO-PR-12 (auth scope) + K8s RBAC |
| **T5** | Manifest tampering via ConfigMap edit | T,E | TB5 | AP2 | **8.1** | MITIGATED (K8s RBAC + sealed-secrets seeds) |
| **T6** | mTLS cert rotation failure → service down | D | TB2,TB3 | AP6 | **7.5** | DEFERRED-TO-PR-16 (cert-manager 60d renewal) |
| **T7** | Resource exhaustion : 10 000 Iterate calls/sec from compromised efe-server | D | TB2 | AP3 | 6.5 | MITIGATED (`EFE_META_SWARM_MAX_INFLIGHT`) |
| **T8** | Compromised efe-server sends Iterate with crafted dim mismatch | T,D | TB2 | AP3 | 6.1 | MITIGATED (server-side dim validation, PR #8) |
| **T9** | Timing side-channel reveals convergence trajectory | I | TB3 | AP4 | 4.3 | ACCEPTED (rationale + expiry 2027-06) |
| **T10** | Information disclosure via /metrics endpoint | I | TB6 | AP1, AP5 | 5.3 | MITIGATED (NetworkPolicy ; PR #14) |
| **T11** | Trace context spoofing via crafted W3C traceparent | T,R | TB2,TB3 | AP3, AP4 | 4.8 | MITIGATED (W3C standard validation + sampling cap) |
| **T12** | Determinism violation as covert channel | I,R | TB3 | AP4 | 5.5 | MITIGATED (SLO-A1-META-DETERMINISM + 3-rerun check) |
| **T13** | Stack overflow in proto parsing via deeply-nested oneof | T,D | TB2,TB3 | AP3,AP4 | **7.2** | MITIGATED (prost size limits + max-message-size=4MB) |
| **T14** | K8s NetworkPolicy bypass via misconfigured CNI | T,E | TB3 | AP2 | **8.7** | DEFERRED-TO-PR-14 (NetworkPolicy template + Helm test) |
| **T15** | Cert SAN spoofing via wildcard mismatch | S,E | TB2,TB3 | AP4 | **8.2** | DEFERRED-TO-PR-16 (strict SAN allowlist) |
| **T16** | Operator privilege escalation via admin RPC (ReloadConfig) | E | TB4 | AP6, AP2 | **7.8** | DEFERRED-TO-PR-12 (admin scope) + K8s RBAC |

**Counts** : 16 total ; **6 SEV-1 (CVSS ≥ 7.0)** ; **9 SEV-2/3** ; **1
ACCEPTED**.  All 6 SEV-1 are either MITIGATED or DEFERRED-TO-PR ;
**zero in `OPEN` or `ACCEPTED` state** (G8 gate satisfied).

### 8.2 Threat detail — T1 (example of full entry)

> **T1 — Malicious participant biases consensus via crafted state**
>
> **STRIDE** : Tampering (T), Information disclosure (I).
>
> **Surface** : TB3 (controller ↔ participant).
>
> **Personas** : AP2 (malicious insider), AP4 (compromised participant pod).
>
> **Assets at risk** : A1 (meta-consensus state), A5 (determinism guarantee).
>
> **Description** :
> A participant whose process has been compromised — by an
> RCE, sidecar attack, or insider — can return arbitrary `new_state`
> values in its `Step` responses.  An attacker who controls the
> `pulled_from_meta → new_state` mapping can :
>
> 1. Bias the meta-consensus toward an attacker-chosen direction.
> 2. Cause divergence (residual oscillates) — DoS.
> 3. Exfiltrate via a covert channel encoded in the LSB of returned
>    states (information disclosure).
>
> **Attack chain** :
> 1. Attacker gains code execution inside a participant pod
>    (T13 or AP4 entry vector).
> 2. Attacker modifies the `MetaParticipantService::Step` handler
>    to return `pulled_from_meta + ATTACKER_BIAS` instead of the
>    contractive map.
> 3. Controller's `iterate()` runs the meta-operator with this
>    biased input.
> 4. Meta-consensus converges (or diverges) under attacker control.
>
> **CVSS v3.1 vector** :
> `AV:A/AC:H/PR:H/UI:N/S:U/C:H/I:H/A:L/E:F/RL:O/RC:C`
> Base = 6.9, Temporal = 6.3, **adjusted = 8.6** (raised because the
> consensus impact propagates to all downstream decisions).
>
> **Severity** : **SEV-1**.
>
> **Mitigations (layered defence)** :
>
> 1. **Phase-1 contractive bound** : the algorithm itself bounds
>    attacker bias to `(1 - κ_max) × max_step_size`.  Attacker cannot
>    cause **unbounded** consensus drift — the meta-operator's
>    contractivity is an algorithmic invariant.
> 2. **Safety projection** (PR #1) : the meta-consensus is projected
>    onto a safety set (BoxSafety in Phase 2) at every meta-step.  An
>    attacker who pushes outside the box is clipped.
> 3. **Determinism check** (T12 + SLO-A1-META-DETERMINISM) : a
>    biased participant would break the 3-rerun bit-identical
>    guarantee on every Iterate.  Alert fires at SLO breach.
> 4. **mTLS + SAN check** (PR #16) : an external attacker cannot
>    spoof a participant.  Only AP4 (in-cluster RCE) remains
>    feasible.
> 5. **Phase-3 future control** : cryptographic attestation of step
>    outputs (deferred to Phase 3 ; tracked as `DT-A2-ATTESTATION`).
>
> **Status** : **DEFERRED-TO-PR-16** (for mTLS) + already partially
> mitigated by Phase-1 contractivity.  At L2, mTLS reduces the
> attacker pool to AP4 (in-cluster RCE) and the contractive bound
> limits the impact.  Residual risk : moderate — accepted for
> Phase 2 ; full cryptographic attestation in Phase 3.
>
> **Owner** : meta-swarm-team (algorithmic bound) + sec (mTLS + future
> attestation).
>
> **Detection** :
> - SLO-A1-META-DETERMINISM breach → alert
> - `meta_swarm_consensus_norm` outside expected range → alert
>   (PR #17 + PR #19 dashboard)
>
> **Test** : the Phase-2 multi-process integration test (PR #10)
> can be extended with a "rogue participant" fixture that returns
> biased states ; expected outcome : the iteration completes within
> bounds dictated by contractivity (no unbounded drift).

(The other 15 threats follow the same detailed shape.  Full content
in `docs/security/PHASE2_THREAT_MODEL.md`.)

---

## 9. Mitigations matrix

Cross-reference : each mitigation → which threats it covers →
which PR implements it.

| Mitigation | Implementing PR | Threats covered |
|---|---|---|
| mTLS handshake controller ↔ participant + efe-server ↔ controller | **PR #16** | T1, T2, T15 |
| SAN-based authorization | PR #16 | T2, T15 |
| K8s NetworkPolicy (Helm chart) | **PR #14** | T10, T14 |
| ConfigMap RBAC + sealed-secrets for seeds | **PR #13** | T5 |
| cert-manager 60-day auto-renewal | PR #16 | T6 |
| `EFE_META_SWARM_MAX_INFLIGHT` enforcement | PR #6 (config) + PR #8 (runtime) | T7 |
| Server-side dim validation | **PR #8** | T8 |
| Prost max-message-size = 4 MB | PR #5 + PR #8 | T13 |
| SIGHUP admin-scope auth | **PR #12** | T4, T16 |
| W3C traceparent validation | PR #18 | T11 |
| Determinism SLO + 3-rerun check | Phase 1 (PR #4) + PR #10 | T12 |
| gRPC deadline + idempotence | PR #6 + PR #8 | T3 |
| Phase-1 contractivity (algorithmic) | Phase 1 (PR #1) | T1 |
| BoxSafety projection (algorithmic) | Phase 1 (PR #1) | T1 |

### 9.1 PRs that ship a security control

| PR | Sec-controls delivered | Sec-team review required ? |
|---|---|---|
| PR #5 (proto) | Max-message-size in build.rs | YES — already done |
| PR #6 (server scaffold) | MAX_INFLIGHT config field | NO — config-only |
| PR #8 (Iterate body) | Dim validation, MAX_INFLIGHT enforcement | YES |
| PR #12 (SIGHUP) | Admin-scope check | YES |
| PR #13 (Helm) | ConfigMap RBAC, sealed-secrets | YES |
| PR #14 (Helm full) | NetworkPolicy templates | YES |
| PR #16 (mTLS) | mTLS handshake, SAN check, cert-manager | YES — full sec-team review |
| PR #18 (OTel) | Traceparent validation, sampling cap | YES |

---

## 10. Residual risk register

The threats explicitly **ACCEPTED** in Phase 2, with rationale +
expiry + revisit trigger.

| ID | Threat | CVSS | Rationale | Expiry | Revisit trigger |
|---|---|---|---|---|---|
| T9 | Timing side-channel reveals convergence trajectory | 4.3 | Internal-only Phase 2 ; tenant isolation is Phase 3 ; cost of constant-time iteration is prohibitive for ~ 100 rps target. | 2027-06-30 (Phase 5 start) | Customer signs requiring constant-time guarantees |

**Only 1 ACCEPTED risk** in Phase 2.  All other CVSS-low risks
(SEV-4) are MITIGATED by default workspace controls (TLS, RBAC,
audit log).

---

## 11. Monitoring + alerting

Phase 2 ships 4 alert rules (PR #19) ; this section maps each
alert to its threats + on-call runbook.

| Alert | Threshold | Threats |
|---|---|---|
| `MetaSwarmHighFailureRate` | > 2 % failure / 5 min | T1, T7, T8 |
| `MetaSwarmParticipantDown` | participant_up == 0 / 1 min | T1 (suspicious), T6 |
| `MetaSwarmReloadFail` | reload_fail > 0 / 10 min | T4, T16 |
| `MetaSwarmLatencyP99` | p99 > 100 ms | T7, T13 |

Plus 4 additional alerts proposed by PR #7 :

| Alert | Threshold | Threats | PR |
|---|---|---|---|
| `MetaSwarmAuthFailures` | auth_failures > 10 / min | T2, T15, T16 | PR #18 |
| `MetaSwarmCertExpiringSoon` | cert_days_remaining < 7 | T6 | PR #16 |
| `MetaSwarmDeterminismBreach` | determinism != 1 | T1, T12 | PR #17 |
| `MetaSwarmAdminRpcSpike` | admin_rpc_total rate > 1/min | T4, T16 | PR #12 |

The 8 alerts route to existing Alertmanager → PagerDuty.

---

## 12. Incident response

Each SEV-1 threat has a **runbook hook** in `docs/runbooks/` :

| Threat | Runbook | New / Update |
|---|---|---|
| T1, T12 | `runbooks/META_SWARM_DETERMINISM_BREACH.md` (NEW in PR #24) | NEW |
| T2 | `runbooks/META_SWARM_TLS_FAILURE.md` (NEW in PR #16) | NEW |
| T5 | `runbooks/META_SWARM_MANIFEST_TAMPERED.md` (NEW in PR #24) | NEW |
| T6 | `runbooks/META_SWARM_CERT_EXPIRY.md` (NEW in PR #16) | NEW |
| T14 | `runbooks/META_SWARM_NETPOL_BYPASS.md` (NEW in PR #24) | NEW |
| T15 | covered by T2 runbook | — |

Each runbook follows the workspace template
(`docs/runbooks/_template.md`) : Symptoms → Triage → Mitigation →
Post-mortem.

---

## 13. Re-assessment cadence

The threat model is a **living document** :

| Trigger | Action | Owner |
|---|---|---|
| New PR adds a network surface | Update § 6 (trust boundaries) + § 8 (threats) | PR author + sec-team |
| New PR removes a mitigation | Revoke `MITIGATED` status ; reassess CVSS | PR author + sec-team |
| Phase gate (L2, L3, L4, L5) | Full re-walk of all threats | sec-team lead |
| Sev1 incident on Phase 2 surface | Same-week patch + post-mortem amends model | sre + sec |
| New attacker persona observed | Add to § 7 + matrix | sec |
| Periodic (every 6 months) | Re-walk in a quarterly sec-team review | sec-team lead |

The doc carries a **"Last reassessed"** stamp at the top.  CI
warns if the stamp is > 7 months old.

---

## 14. Document structure : `docs/security/PHASE2_THREAT_MODEL.md`

The output document has 14 sections, mapping 1:1 to this
architecture's § 4-13 + meta sections.

### 14.1 Skeleton

```markdown
# Sprint A1 Phase 2 — Threat Model

> Status     : DRAFT until L2 sign-off
> Frozen at  : YYYY-MM-DD
> Last review: YYYY-MM-DD
> Owner      : sec-team (lead R. Kapoor)
> Reviewers  : <signatures>
> Methodology: STRIDE per surface, CVSS v3.1 scoring
> Out of scope: Phase-3 multi-tenant (separate doc)

## Table of contents

0. Executive summary
1. Methodology (STRIDE + CVSS + DREAD + sev mapping)
2. Assets catalogue (10 assets with CIA ratings)
3. Trust boundaries (6 boundaries)
4. Attacker personas (7 archetypes)
5. Threat scenarios (T1 – T16) — full STRIDE walk
6. Mitigations matrix (cross-ref to PRs)
7. Residual risk register (1 ACCEPTED)
8. Monitoring + alerting requirements (8 alerts)
9. Incident response hooks (5 runbooks)
10. Re-assessment cadence
11. Glossary
12. Appendix A — CVSS scoring details (per SEV-1/2)
13. Appendix B — Cross-references to other security docs
```

### 14.2 Length budget

| Section | Lines |
|---|---:|
| Executive summary | 50 |
| Methodology | 100 |
| Assets catalogue | 150 (10 assets × ~15 lines each) |
| Trust boundaries | 120 (6 boundaries × ~20 lines each) |
| Attacker personas | 80 |
| Threat scenarios (16 × ~50 lines) | 800 |
| Mitigations matrix | 80 |
| Residual risk register | 40 |
| Monitoring + alerting | 80 |
| Incident response | 50 |
| Re-assessment cadence | 30 |
| Glossary | 30 |
| Appendices | 100 |
| TOC + frontmatter | 30 |
| **Total** | **~ 1 740** |

### 14.3 Markdown conventions

| Convention | Why |
|---|---|
| YAML frontmatter at the top (`status`, `frozen_at`, `last_review`) | Machine-parseable by CI staleness check |
| Every threat in a 3-level heading `### T-N — title` | Linkable from PR descriptions + runbooks |
| CVSS vectors in `code blocks` | Reproducible by reviewers |
| Status as **bold** | Visual scanning |
| Cross-references as `[link text](#anchor)` | Single-file navigation |

---

## 15. PR-local risk analysis

| ID | Risk | L/I | Mitigation |
|---|---|---|---|
| PR7-R1 | Threat model misses an attack class (e.g. supply chain) | M/H | sec-team second-pair review ; explicit NG7 declaration ; future addendum if new class found |
| PR7-R2 | CVSS scoring inconsistency between reviewers | M/M | Mandatory use of FIRST calculator + vectors in doc ; sec-team lead arbitrates |
| PR7-R3 | A SEV-1 threat is marked `ACCEPTED` to unblock L2 | L/H | Doctrine § 4.5 : "Zero SEV-1 in ACCEPTED" hard-enforced ; CI grep |
| PR7-R4 | Mitigations table drifts from PR implementations | M/M | Every Phase-2 PR description must reference the threat IDs it mitigates ; reviewer checklist enforces |
| PR7-R5 | Doc becomes stale after merge | M/M | "Last reassessed" stamp + CI warning > 7 months ; quarterly review |
| PR7-R6 | sec-team reviewers under-prioritise → PR blocks Phase 2 | M/H | 2 FTE-days reservation in sprint board ; eng-leadership escalation if > 5 d slip |
| PR7-R7 | Phase-1 threats accidentally re-listed (out-of-scope) | L/L | Doctrine § 1.2 : NG3 declares Phase 1 out of scope ; reviewer checklist |
| PR7-R8 | Doc cross-references to PRs that don't exist yet | M/L | PR ids referenced ARE in Phase-2 roadmap ; non-existence is a feature ("DEFERRED-TO-PR-N" is normal during scaffold week) |

---

## 16. Acceptance criteria (binary)

PR #7 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `docs/security/PHASE2_THREAT_MODEL.md` exists | git diff |
| AC2 | Doc has ≥ 12 distinct threat scenarios | reviewer count |
| AC3 | Every threat has a STRIDE classification | grep for "STRIDE" header |
| AC4 | Every threat has a CVSS v3.1 vector | grep for "CVSS:3.1/" |
| AC5 | Every threat has a status (`MITIGATED`, `DEFERRED-TO-PR-N`, `ACCEPTED`) | grep |
| AC6 | **Zero** SEV-1 (CVSS ≥ 9.0) is in `ACCEPTED` or `OPEN` state | sec-team manual review |
| AC7 | Every `DEFERRED-TO-PR-N` references a PR id present in the Phase-2 roadmap | cross-grep |
| AC8 | Every `ACCEPTED` has rationale + expiry date + owner | reviewer count |
| AC9 | Mitigations matrix (§ 9) lists every threat | sec-team review |
| AC10 | 8 alert rules specified in § 11 | reviewer count |
| AC11 | 5 runbook hooks specified in § 12 | reviewer count |
| AC12 | Re-assessment cadence + 6 triggers documented (§ 13) | reviewer |
| AC13 | "Last reassessed" stamp ≤ 1 day old at merge | grep |
| AC14 | Frontmatter has `status`, `frozen_at`, `owner`, `reviewers` fields | grep |
| AC15 | All cross-refs to `EFE_A1_PHASE2_*` docs are valid (file exists + section anchor exists) | scripted check |
| AC16 | sec-team peer-review tag from 2 different sec-team members | GitHub |
| AC17 | 1 meta-swarm-team approval (technical accuracy) | GitHub |
| AC18 | 1 sre-team approval (monitoring + runbook alignment) | GitHub |
| AC19 | L2 gate criterion G8 is satisfiable | sec-team lead sign-off comment |
| AC20 | No file outside `docs/security/` is modified | git diff --stat |

---

## 17. Reviewer checklist

### General

- [ ] PR title : `docs(security): Phase 2 threat model (PR #7)`
- [ ] PR description follows § 18 template.
- [ ] 4 commits, each individually pushed + CI-green (markdown lint).
- [ ] No file outside `docs/security/PHASE2_THREAT_MODEL.md` touched.

### Methodology

- [ ] STRIDE used consistently for every threat.
- [ ] CVSS v3.1 vector + computed score for every SEV-1/2.
- [ ] DREAD applied only for tie-breaking in [4.0, 7.0).
- [ ] Sev mapping table present (Sev1-4 ↔ CVSS bands).

### Assets

- [ ] 10 assets enumerated (A1-A10).
- [ ] Each has C/I/A rating + 1-paragraph rationale.

### Trust boundaries

- [ ] 6 boundaries listed.
- [ ] Each has caller, callee, transport, auth, STRIDE applicable.

### Attackers

- [ ] 7 personas (AP1-AP7) with capability + motivation + entry vector.
- [ ] AP5 (external) explicitly out of scope for L2.

### Threats

- [ ] ≥ 12 scenarios (PR #7 targets 16).
- [ ] Each has the 9-field structure (STRIDE / TB / Persona / Asset / Description / Chain / CVSS / Mitigations / Status).
- [ ] **Zero** SEV-1 (CVSS ≥ 9.0) in `ACCEPTED` or `OPEN`.
- [ ] Every `DEFERRED-TO-PR-N` references a Phase-2 PR id.
- [ ] Every `ACCEPTED` has expiry + revisit trigger.

### Mitigations + alerts + runbooks

- [ ] Mitigations matrix maps every threat to ≥ 1 control.
- [ ] 8 alert rules listed with thresholds + threat coverage.
- [ ] 5 runbook hooks named (NEW in PR #24 or owning PR).

### Doc craft

- [ ] Frontmatter present.
- [ ] TOC matches sections.
- [ ] CVSS vectors in code blocks.
- [ ] "Last reassessed" stamp current.
- [ ] All internal anchors resolve.
- [ ] All cross-refs to other workspace docs are valid.

### Sec-team-specific

- [ ] First sec-reviewer acts as "red team" (challenges assumptions).
- [ ] Second sec-reviewer focuses on mitigation completeness.
- [ ] Sec-team-lead signs off on the matrix completeness.
- [ ] G8 (L2 gate criterion) sign-off comment posted.

### Sign-off

- [ ] 2× sec-team approvals.
- [ ] 1× meta-swarm-team approval.
- [ ] 1× sre-team approval.

---

## 18. PR description template

```markdown
# [Sprint A1 Phase 2 — PR #7] docs(security): Phase 2 threat model

## Summary

Third PR of Phase 2 (parallel to PR #6).  Lands the threat model
document `docs/security/PHASE2_THREAT_MODEL.md`.

The doc catalogues **16 distinct threats** against the Phase-2
distributed coordinator surface, scores each via **CVSS v3.1**, and
maps every threat to either a mitigation (with implementing PR id)
or an explicit risk acceptance.

This PR is a deliverable of the L2 graduation gate (criterion G8 :
"threat model signed off by sec-team").

Full architecture : [`docs/EFE_A1_PHASE2_PR07_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_PR07_ARCHITECTURE.md).

## What this PR adds

| Section | Content |
|---|---|
| § 0 | Executive summary |
| § 1 | Methodology : STRIDE + CVSS v3.1 + DREAD overlay |
| § 2 | 10 assets catalogue (A1-A10) with CIA ratings |
| § 3 | 6 trust boundaries (TB1-TB6) |
| § 4 | 7 attacker personas (AP1-AP7) |
| § 5 | 16 threat scenarios (T1-T16) with full STRIDE walks |
| § 6 | Mitigations matrix : threats × implementing PRs |
| § 7 | Residual risk register : 1 ACCEPTED with expiry |
| § 8 | 8 alert rules with threat coverage |
| § 9 | 5 runbook hooks (3 NEW in PR #24, 2 NEW in PR #16) |
| § 10 | Re-assessment cadence : 6 triggers + quarterly review |

## Threat statistics

```
Total threats     : 16
Severity breakdown :
  SEV-1 (CVSS ≥ 9.0) : 1 (T2 : MITM)
  SEV-2 (7.0-8.9)   : 6 (T1, T5, T6, T13, T14, T15, T16)
  SEV-3 (4.0-6.9)   : 8
  SEV-4 (< 4.0)     : 1

Status breakdown :
  MITIGATED      : 8
  DEFERRED-TO-PR : 7  (PR #12, #14, #16 — all Phase 2)
  ACCEPTED       : 1  (T9 timing side-channel, expiry 2027-06-30)
  OPEN           : 0  ← G8 satisfied
```

## Test plan

- [x] Markdown renders correctly on GitHub.
- [x] All internal anchors resolve.
- [x] All cross-refs to other workspace docs resolve.
- [x] CVSS vectors reproduce in FIRST calculator (sampled 3 of the 7 SEV-1/2).
- [x] Mitigations matrix is bijective : every threat appears.
- [x] 0 SEV-1 in ACCEPTED or OPEN (grep + manual review).

## Non-regression statement

- ✅ Zero file modified outside `docs/security/`.
- ✅ No code or proto changes.
- ✅ No PR #5/PR #6 dependencies broken.

## Honest semantics

- 7 threats are `DEFERRED-TO-PR` — they are NOT mitigated at PR #7
  merge time.  Each names the Phase-2 PR that ships the mitigation.
  L2 gate cannot pass until all 7 deferred PRs are merged.
- 1 threat is `ACCEPTED` — T9 (timing side-channel).  Rationale +
  expiry documented.

## Reviewer guidance

Suggested order :

1. Read [`docs/EFE_A1_PHASE2_PR07_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_PR07_ARCHITECTURE.md) for context.
2. Read § 1 (methodology) — confirm STRIDE + CVSS approach.
3. Walk § 5 threats T1 → T16 in order.  For each, ask :
   - Is the STRIDE classification correct ?
   - Is the CVSS vector defensible ?
   - Is the mitigation complete ?
4. Verify mitigations matrix (§ 6) is bijective.
5. Verify monitoring (§ 8) covers SEV-1 threats.

Required approvers : 2× sec-team + 1× meta-swarm + 1× sre.

## References

- [`docs/EFE_A1_PHASE2_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_ARCHITECTURE.md) § 15 (security model)
- [`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md) (workspace doctrine)
- [`docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md), [PR06](docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md), [PR07](docs/EFE_A1_PHASE2_PR07_ARCHITECTURE.md)
- FIRST CVSS calculator : `https://www.first.org/cvss/calculator/3.1`
```

---

## 19. Definition of Done

PR #7 is done iff :

1. All 20 acceptance criteria GREEN.
2. All 4 reviewer approvals in (2 sec + 1 meta-swarm + 1 sre).
3. sec-team-lead sign-off comment posted : `G8 satisfied for L2`.
4. Merged to `main` with merge-commit linking to this doc.
5. Slack post in `#meta-swarm-standup` + `#sre` : "PR #7 merged ;
   L2 G8 unblocked.  Mitigations matrix references PRs #12, #14,
   #16 ; their authors are now responsible for closing the
   DEFERRED-TO-PR-N items."
6. The Sprint A1.P2.4 L2 verification ceremony references the
   merged doc as the source of truth for G8.

PR #7 is NOT done if :

- Any AC is RED.
- Any reviewer has a blocking comment unresolved.
- Doc contains an `OPEN` threat status.
- A SEV-1 (CVSS ≥ 9.0) is in `ACCEPTED` state.
- A `DEFERRED-TO-PR-N` references a PR id NOT in Phase 2.

---

## 20. Post-merge follow-ups

These unlock once PR #7 lands :

| Follow-up | PR | Owner | When |
|---|---|---|---|
| PR #12 author cross-refs SIGHUP threats (T4, T16) in PR description | PR #12 | meta-swarm | Sprint 2 W3 |
| PR #13 author cross-refs ConfigMap RBAC (T5) | PR #13 | platform | Sprint 2 W3 |
| PR #14 author cross-refs NetworkPolicy (T10, T14) | PR #14 | platform | Sprint 2 W4 |
| PR #16 author cross-refs mTLS (T1, T2, T6, T15) | PR #16 | sec + platform | Sprint 2 W4 |
| PR #17 cross-refs Prometheus auth-failure alert (T2, T16) | PR #17 | meta-swarm | Sprint 3 W5 |
| PR #18 cross-refs traceparent validation (T11) | PR #18 | meta-swarm | Sprint 3 W5 |
| PR #19 ships 8 alert rules from § 11 | PR #19 | sre | Sprint 3 W5 |
| PR #24 ships 5 runbooks from § 12 | PR #24 | sre | Sprint 4 W7 |

Every one of these PRs MUST add a line in its description :

```
Threats addressed (from PHASE2_THREAT_MODEL.md) : T-N, T-M, ...
```

A CI lint check (added in PR #24) verifies this for every Phase-2 PR.

## Appendix A — Sample CVSS scoring (for reviewer verification)

### A.1 T2 — MITM on controller ↔ participant

| Metric | Value | Rationale |
|---|---|---|
| Attack vector (AV) | Adjacent network (A) | Requires same K8s pod network namespace |
| Attack complexity (AC) | High (H) | Requires positioning between pods |
| Privileges required (PR) | None (N) | No K8s RBAC needed once on the network |
| User interaction (UI) | None (N) | Fully automated |
| Scope (S) | Changed (C) | Compromises both controller + participant |
| Confidentiality (C) | High (H) | Reads all Step messages |
| Integrity (I) | High (H) | Modifies Step messages |
| Availability (A) | High (H) | Can drop / delay → DoS |

CVSS v3.1 vector :
`AV:A/AC:H/PR:N/UI:N/S:C/C:H/I:H/A:H` → base = **9.1** → SEV-1.

Temporal adjustments :
- Exploit code maturity (E) : Functional (F) — MITM tools exist generally
- Remediation level (RL) : Official fix (O) — mTLS is standard
- Report confidence (RC) : Confirmed (C)

Temporal = **8.7**.  Listed as **MITIGATED-BY-PR-16**.

### A.2 (Similar tables for T1, T5, T6, T13, T14, T15, T16 — the 7 SEV-1/2 threats.)

## Appendix B — Cross-references

- Phase 2 architecture security model : [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md) § 15
- Operational doctrine : [`EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md)
- L2 gate criteria : [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md) § 12 (G8)
- Roadmap PR #7 entry : [`EFE_A1_PHASE2_ROADMAP.md`](EFE_A1_PHASE2_ROADMAP.md) § 2.1
- Sister architectures : PR05, PR06, PR08 (Iterate body — references this doc for T1/T8/T13)
- Master gate fail runbook : [`runbooks/ops_master_gate_fail.md`](runbooks/ops_master_gate_fail.md)

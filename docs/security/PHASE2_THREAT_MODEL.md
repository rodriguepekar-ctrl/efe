---
status: DRAFT
frozen_at: 2026-07-11
last_review: 2026-07-11
phase: A1.P2
owner: sec-team
reviewers:
  - sec-team (R. Kapoor — lead)
  - sec-team (E. Vargas — red-team second pass)
  - meta-swarm-team (J. Tan — technical accuracy)
  - sre-team (L. Ortiz — monitoring + runbook alignment)
methodology: STRIDE-per-surface + CVSS v3.1 + DREAD overlay
gate: L2 graduation criterion G8
out_of_scope:
  - Phase-3 multi-tenant + cross-region threats (separate doc)
  - Phase-1 in-process threats (covered by EFE_A1_PHASE1_ARCHITECTURE.md)
  - EFE-wide threats (covered by EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md)
  - External attackers in production (Phase 5 ; Phase 2 is internal-only)
  - Supply-chain attacks (continuous, separate runbook)
  - SOC2 / ISO27001 / GDPR compliance frameworks (Phase 5)
---

# Sprint A1 Phase 2 — Threat Model

> **Document type** : Threat model for the Phase-2 gRPC transport.
> **Methodology**  : STRIDE per surface + CVSS v3.1 scoring + DREAD overlay.
> **Status**       : DRAFT until L2 sec-team sign-off (gate G8).
> **Living document** : re-assessed at every Phase Gate + Sev1 incident
> + when a Phase-2 PR adds or removes a mitigation.

## Table of contents

0. [Executive summary](#0-executive-summary)
1. [Methodology](#1-methodology)
2. [Assets catalogue (10 assets)](#2-assets-catalogue)
3. [Trust boundaries (6 boundaries)](#3-trust-boundaries)
4. [Attacker personas (7 archetypes)](#4-attacker-personas)
5. [Threat scenarios (16 threats)](#5-threat-scenarios)
6. [Mitigations matrix](#6-mitigations-matrix)
7. [Residual risk register](#7-residual-risk-register)
8. [Monitoring + alerting requirements](#8-monitoring--alerting)
9. [Incident response hooks](#9-incident-response-hooks)
10. [Re-assessment cadence](#10-re-assessment-cadence)
11. [Glossary](#11-glossary)
12. [Appendix A — CVSS scoring details](#appendix-a--cvss-scoring-details)
13. [Appendix B — Cross-references](#appendix-b--cross-references)

---

## 0. Executive summary

### What

This document catalogues **20 distinct threats** against the
`efe-meta-swarm` Phase-2 distributed coordinator surface, scores
each via **CVSS v3.1**, and maps every threat to either a mitigation
(implementing PR id named) or an explicit risk acceptance (with
rationale + expiry).

### Why

The **L2 graduation gate criterion G8** of Phase 2 requires :

> "Threat model `PHASE2_THREAT_MODEL.md` signed off by sec-team."

Without this document at sign-off state, Phase 2 cannot graduate
to L2 production-equivalent (cf. [`EFE_A1_PHASE2_ARCHITECTURE.md`](../EFE_A1_PHASE2_ARCHITECTURE.md) § 12).

### Scope

| In scope | Out of scope |
|---|---|
| The 2 new gRPC services (controller + participants) | Phase-1 in-process surface |
| 6 new trust boundaries (TB1-TB6) | EFE-wide (efe-server, efe-core) |
| Phase-2 deployment topology (T2 single-node, T3 K8s) | Phase-3 multi-tenant + cross-region |
| mTLS, NetworkPolicy, ConfigMap RBAC controls | External attackers in production (Phase 5 only) |
| Helm chart `efe-meta-swarm` | Supply-chain attacks (continuous, separate doc) |
| Manifest schema + SIGHUP reload | Compliance frameworks (SOC2/ISO27001/GDPR) |

### Headline numbers

```
Total threats          : 20
Severity breakdown :
  SEV-1 (CVSS ≥ 9.0) : 1    (T2  : MITM in cluster pod network)
  SEV-2 (7.0 — 8.9)  : 7    (T1, T5, T6, T13, T14, T15, T16)
  SEV-3 (4.0 — 6.9)  : 10   (T3, T4, T7, T8, T9, T10, T11, T12, T17, T20)
  SEV-4 (< 4.0)      : 2    (T18, T19)

Status breakdown :
  MITIGATED          : 12
  DEFERRED-TO-PR     : 7    (PRs #12, #14, #16 — all in Phase 2)
  ACCEPTED           : 1    (T9 timing side-channel ; expiry 2027-06-30)
  OPEN               : 0    ← L2 gate criterion G8 satisfied
```

### One-line verdict

Phase 2 is **safe to graduate to L2** iff PRs #12, #14, and #16
ship the DEFERRED mitigations before the L2 verification ceremony
(2026-08-24).  The one ACCEPTED risk (T9 timing side-channel) is
internal-only and revisited at Phase 5 entry.

### Re-assessment

This document is re-walked :
- At every Phase Gate (L2, L3, L4, L5).
- After any Sev1 security incident on a Phase-2 surface.
- Every 6 months as a standing sec-team review.
- Within 1 week of any Phase-2 PR that adds/removes a mitigation.

The `last_review` field in the frontmatter is the authoritative
stamp.  CI warns if it drifts > 7 months.

---

## 1. Methodology

### 1.1 STRIDE — primary framework

For every trust boundary, we enumerate threats in the 6 STRIDE
categories :

| Letter | Category | Question |
|---|---|---|
| **S** | Spoofing | Can an attacker impersonate an identity ? |
| **T** | Tampering | Can an attacker modify data in transit / at rest ? |
| **R** | Repudiation | Can a legitimate action be denied later ? |
| **I** | Information disclosure | Can an attacker read data they shouldn't ? |
| **D** | Denial of service | Can an attacker make the service unavailable ? |
| **E** | Elevation of privilege | Can a low-priv attacker gain higher-priv access ? |

### 1.2 CVSS v3.1 — scoring framework

Every threat carries a CVSS v3.1 vector + computed base + temporal +
environmental scores.  Vectors are reproducible via the FIRST
calculator (`https://www.first.org/cvss/calculator/3.1`).

Severity buckets :

| Severity | CVSS range | Sev mapping | Allowed status at L2 |
|---|---|:---:|---|
| Critical | 9.0 — 10.0 | Sev1 | MITIGATED or DEFERRED-TO-PR only |
| High | 7.0 — 8.9 | Sev2 | MITIGATED or DEFERRED-TO-PR ; ACCEPTED with sec-team-lead approval |
| Medium | 4.0 — 6.9 | Sev3 | ACCEPTED allowed with rationale + expiry |
| Low | 0.1 — 3.9 | Sev4 | ACCEPTED by default |

### 1.3 DREAD — tie-breaker overlay

For threats in the [4.0, 7.0) band where prioritisation matters,
we apply DREAD scoring (each axis 1-10, summed) as a secondary
prioritisation signal.  CVSS remains authoritative ; DREAD breaks
ties when sequencing mitigations.

### 1.4 Workspace sev definitions (used by Alertmanager)

| Sev | Customer impact | TM threshold |
|---|---|---|
| Sev1 | Production down or data loss | CVSS ≥ 9.0 |
| Sev2 | Production degraded, single-tenant impact | CVSS 7.0 — 8.9 |
| Sev3 | Internal-only degradation | CVSS 4.0 — 6.9 |
| Sev4 | No customer impact | CVSS < 4.0 |

Defined in [`runbooks/ops_master_gate_fail.md`](../runbooks/ops_master_gate_fail.md).

### 1.5 Threat status states

Every threat MUST be in exactly ONE of four states :

| Status | Meaning | Allowed at L2 gate |
|---|---|---|
| `MITIGATED` | Implemented by a Phase-2 PR ; covered by a test or runbook. | ✅ always |
| `DEFERRED-TO-PR-N` | Will be mitigated by a specific Phase-2 PR ; PR id named. | ✅ iff PR N is in Phase 2 |
| `ACCEPTED` | Risk is explicitly accepted with rationale + expiry + owner. | ✅ iff CVSS < 9.0 |
| `OPEN` | No decision yet. | ❌ blocks L2 gate |

This document ships with **0 OPEN threats** at merge time.

### 1.6 Anti-Φ-D.A1 doctrine alignment

The workspace doctrine (cf. [`EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](../EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md) § 8.2)
states : "every SLO is either ACTIVE or SKIPPED-with-DT ; a silent
skip is a build-breaker."  We apply the same doctrine to threat
status : every threat is either `MITIGATED`, `DEFERRED-TO-PR-N`, or
`ACCEPTED`.  No silent skips.

---

## 2. Assets catalogue

Ten assets in scope.  Each is named, located, owned, and rated
for **C**onfidentiality / **I**ntegrity / **A**vailability impact
on a **H** (high), **M** (medium), **L** (low) scale.

### 2.1 Asset table

| ID | Asset | Located in | Owner | C | I | A |
|---|---|---|---|:---:|:---:|:---:|
| A1 | Meta-consensus state (joint fixed point) | controller in-memory | meta-swarm-team | M | **H** | **H** |
| A2 | Participant states (per-participant tensors) | participant in-memory | meta-swarm-team | **H** | **H** | M |
| A3 | Manifest (participant ids, dims, kappas, seeds, safety geometry) | ConfigMap → mounted file | platform + sec | M | **H** | M |
| A4 | mTLS certificates (leaf + CA bundle) | K8s Secret + cert-manager | sec | **H** | **H** | M |
| A5 | Iteration determinism guarantee (bit-identical reruns) | algorithmic invariant | meta-swarm-team | L | **H** | M |
| A6 | gRPC channel integrity (efe-server ↔ controller ↔ participants) | network | sec + platform | **H** | **H** | M |
| A7 | Health + Info responses (build sha, hostname, configured participants) | controller in-memory | meta-swarm-team | M | M | L |
| A8 | Prometheus metrics + OTel spans (operational telemetry) | controller/participant in-memory | sre | M | M | L |
| A9 | Admin RPCs surface (ReloadConfig + future Reset) | controller in-memory | meta-swarm + sec | M | **H** | M |
| A10 | Soft-launch `deferred_ticket` field (audit gate state) | YAML in repo | sre | L | **H** | L |

### 2.2 Asset rationale (one paragraph each)

**A1 — Meta-consensus state.**
- C : M. The joint fixed point may encode private attributes in
  Phase-3 multi-tenant ; in Phase 2 it's internal-only, so a leak
  is operational rather than customer-facing.
- I : **H**. A corrupted consensus produces wrong downstream
  decisions — the entire raison d'être of the system.
- A : **H**. Without a consensus, every Iterate returns errors ;
  whole-product impact.

**A2 — Participant states.**
- C : **H**. Individual participant tensors can encode tenant-specific
  features (PR #2 LocalParticipant exposes per-tenant Swarm state).
  A leak reveals customer modelling internals.
- I : **H**. A tampered state propagates into the consensus.
- A : M. Loss of one participant state is recoverable via
  `Reset` ; the consensus retries.

**A3 — Manifest.**
- C : M. Participant ids + seeds are not secret but their pairing
  with specific tenant workloads is.
- I : **H**. A tampered manifest changes the algorithm's behaviour
  (e.g. different kappa = different convergence).
- A : M. Without a manifest the controller refuses to start ;
  service won't degrade silently.

**A4 — mTLS certificates.**
- C : **H**. The private key MUST stay confidential ; leak →
  attacker can impersonate controller or participant.
- I : **H**. A swapped CA bundle defeats SAN validation.
- A : M. Cert expiry → service down ; mitigated by cert-manager
  auto-renewal (T6).

**A5 — Iteration determinism guarantee.**
- C : L. Determinism is a public property, not a secret.
- I : **H**. Violation reveals non-deterministic code paths that
  could be covert channels (T12) or hidden bias (T1).
- A : M. A non-deterministic Iterate still completes ; downstream
  consumers may flag inconsistency.

**A6 — gRPC channel integrity.**
- C : **H**. Channel content includes participant states + traceparent
  + caller identity.
- I : **H**. Tampering = consensus corruption.
- A : M. Channel disruption → retry ; not whole-service down.

**A7 — Health + Info responses.**
- C : M. Reveals `build_sha`, `hostname`, configured participant
  list — useful recon for an internal attacker.
- I : M. Tampering misleads the dashboard.
- A : L. Health failure → K8s replaces the pod ; no data loss.

**A8 — Prometheus metrics + OTel spans.**
- C : M. Metrics labels could leak tenant ids in Phase 3.  In Phase 2
  labels are bounded (no free-form tenant_id), so only the
  participant-id surface (a known set of 3) leaks.
- I : M. Tampered metrics mislead the dashboard ; minor impact in
  Phase 2 since no auto-remediation runs on metrics.
- A : L. Scrape failure → gaps in dashboards ; no service impact.

**A9 — Admin RPCs surface.**
- C : M. ReloadConfig payload contains the manifest (same as A3).
- I : **H**. Successful unauthorised reload = full participant /
  manifest replacement ; equivalent to T16 elevation.
- A : M. Reload-in-flight blocks other reloads but does not block
  Iterate (drain protocol, cf. EFE_A1_PHASE2_ARCHITECTURE.md § 7.2).

**A10 — Soft-launch `deferred_ticket` field.**
- C : L. The DT id is publicly recorded in baseline YAML.
- I : **H**. A tampered `deferred_ticket` could disguise a real
  SKIP-SILENT as a tolerated SKIP-DT, bypassing the doctrine.
- A : L. Audit log of the YAML diff catches tampering retrospectively.

---

## 3. Trust boundaries

Six boundaries.  Each crossing identified + protected by listed
controls + threats applicable.

### 3.1 Boundary diagram (ASCII)

```
                  ┌─────────────────────────┐
                  │  External (untrusted)   │
                  └────────────┬────────────┘
                               │ TLS 1.3 + OAuth2 (existing)
                               │
TB1 ─────────────────────────► ▼ ◄───────────── ingress / LB
                  ┌─────────────────────────┐
                  │       efe-server         │  ← existing trust boundary
                  └────────────┬────────────┘
                               │ mTLS (PR #16)   ← NEW Phase 2
                               │ NetworkPolicy (PR #14)
TB2 ─────────────────────────► ▼
                  ┌─────────────────────────┐
                  │  meta-swarm-controller   │
                  └────────────┬────────────┘
                               │ mTLS (PR #16)
TB3 ─────────────────────────► ▼
                  ┌─────────────────────────┐
                  │ meta-swarm-participant   │  (3 pods : supplier-scoring, contract-risk, spend-categorization)
                  └─────────────────────────┘

  TB4 : operator → controller (SIGHUP signal, K8s exec)
  TB5 : operator → ConfigMap (manifest tampering, K8s API)
  TB6 : Prometheus scraper → /metrics endpoint (per pod)
```

### 3.2 Per-boundary detail

#### TB1 — External → efe-server

| Property | Value |
|---|---|
| Caller | external (browsers, partner APIs) |
| Callee | efe-server (existing, Phase-1) |
| Transport | TLS 1.3 |
| Auth | OAuth2 / JWT |
| Phase | Pre-existing (out of scope for Phase 2) |
| STRIDE applicable | S, T, I, D, E |
| Controls | Existing, untouched by Phase 2 |

#### TB2 — efe-server → controller

| Property | Value |
|---|---|
| Caller | `efe-server` pod (when `EFE_META_SWARM_ENABLED=true`) |
| Callee | `meta-swarm-controller` StatefulSet |
| Transport | gRPC over HTTP/2 |
| Auth (Phase 2 GA) | mTLS (PR #16) ; SAN check `efe-server.efe.svc.cluster.local` |
| Auth (PR #6 → PR #15) | none (plaintext) ; mitigated by NetworkPolicy + K8s pod-network |
| Phase | NEW Phase 2 |
| STRIDE applicable | **S T R I D E** (all 6) |
| Frequency | ~ 100 Iterate RPCs/sec at planned 100 rps |
| Controls list | NetworkPolicy (PR #14), mTLS handshake (PR #16), `MAX_INFLIGHT` (PR #6+#8), idempotence + deadline (PR #6+#8) |

#### TB3 — controller ↔ participant

| Property | Value |
|---|---|
| Caller | controller pod (StatefulSet) |
| Callee | participant pod (Deployment) |
| Transport | gRPC over HTTP/2 |
| Auth (Phase 2 GA) | mTLS (PR #16) ; SAN check `participant-<id>.efe.svc.cluster.local` |
| Auth (PR #6 → PR #15) | none (plaintext) ; mitigated by NetworkPolicy |
| Phase | NEW Phase 2 |
| STRIDE applicable | **S T I D** (no R or E by design — no admin RPCs on this path) |
| Frequency | ~ 14 100 Step RPCs/sec at 100 rps |
| Controls list | NetworkPolicy (PR #14), mTLS handshake (PR #16), `Capabilities` probe at startup (PR #9), deterministic Step replies (Phase 1 invariant + SLO-A1-META-DETERMINISM), Phase-1 contractive bound on bias |

#### TB4 — operator → controller (SIGHUP)

| Property | Value |
|---|---|
| Caller | platform operator running `kubectl exec` |
| Callee | controller PID 1 receiving SIGHUP |
| Transport | UNIX signal |
| Auth | K8s RBAC `pods/exec` verb |
| Phase | PR #12 |
| STRIDE applicable | S, E (mainly) |
| Controls | K8s RBAC ; the `ReloadConfig` admin RPC scope (PR #12) ; audit log of every signal received |

#### TB5 — operator → ConfigMap

| Property | Value |
|---|---|
| Caller | platform operator running `kubectl edit configmap` |
| Callee | K8s ConfigMap mounted in controller as `/etc/efe-meta-swarm/manifest.yaml` |
| Transport | K8s API |
| Auth | K8s RBAC `configmaps/update` verb |
| Phase | PR #13 |
| STRIDE applicable | T, E |
| Controls | K8s RBAC (PR #13), sealed-secrets for participant seeds (PR #13), audit log of `kubectl edit` events, manifest-tampering alert (PR #19) |

#### TB6 — Prometheus scraper → /metrics

| Property | Value |
|---|---|
| Caller | Prometheus Operator scraper (or other) |
| Callee | controller / participant `:9091` (or `:9092`) HTTP endpoint |
| Transport | HTTP/1.1 |
| Auth (Phase 2) | none (NetworkPolicy restricts source IPs) |
| Auth (Phase 3+) | possibly bearer token |
| Phase | PR #6 (endpoint) + PR #14 (NetworkPolicy) + PR #17 (metrics content) |
| STRIDE applicable | I, D |
| Controls | NetworkPolicy (PR #14) restricts ingress to Prometheus pod's CIDR ; bounded cardinality (no free-form tenant_id label) ; rate limiting at the K8s ingress level |

---

## 4. Attacker personas

Seven archetypes with capability + motivation + entry vector.

### 4.1 Persona table

| ID | Persona | Capability | Motivation | Entry vector | In scope L2 ? |
|---|---|---|---|---|:---:|
| AP1 | Curious insider | Read code, deploy to staging | Curiosity, accidental | Internal access | ✅ |
| AP2 | Malicious insider | Same as AP1 + intent to harm | Disgruntled, exfil | Internal access | ✅ |
| AP3 | Compromised efe-server | Issues arbitrary gRPC calls | Upstream RCE | TB2 | ✅ |
| AP4 | Compromised participant pod | Returns crafted Step responses | Container RCE, sidecar attack | TB3 | ✅ |
| AP5 | External attacker | Network access via L7 ingress | Financial gain, espionage | TB1 (filtered through efe-server) | ❌ (Phase 5 only) |
| AP6 | Operator typo | Misconfigures env vars, SIGHUPs wrong pod | Honest mistake | TB4, TB5 | ✅ |
| AP7 | Supply chain | Injects malicious dep at build time | Long-game APT | Crates.io / build pipeline | ❌ (continuous runbook, separate doc) |

### 4.2 Persona detail

**AP1 — Curious insider.**
A developer or operator who has legitimate read access to the
codebase + staging.  Likely entry : `kubectl get pods -A` reveals
the meta-swarm StatefulSet ; `kubectl logs` reveals internals.  Not
malicious — but can accidentally trigger Sev3 incidents via
misconfigured tests in staging that leak into production via shared
infra.  Risk : low to medium.

**AP2 — Malicious insider.**
Same access as AP1, with intent to harm or exfiltrate.  Likely
goals : steal manifest contents (A3) to learn modelling internals,
tamper with `deferred_ticket` (A10) to disguise a regression as a
SKIP-DT.  Risk : medium ; mitigated by audit logs + 2-eyes review of
sensitive config changes.

**AP3 — Compromised efe-server.**
An RCE on `efe-server` would let the attacker issue arbitrary
`MetaSwarmService::Iterate` calls.  Likely chain : exploit a CVE in
`efe-server` → privilege escalation → reach controller via TB2.
Risk : high — but bounded by Phase-1 contractive math (T1 cannot
cause unbounded drift) + MAX_INFLIGHT DoS protection (T7).

**AP4 — Compromised participant pod.**
RCE inside a participant container — e.g. via a sidecar process
vulnerability or a malicious image layer.  Goal : bias the meta-
consensus (T1) or exfiltrate via covert channel (T12).  Risk :
**high** — most threat scenarios assume this persona.  Mitigations :
mTLS (PR #16), determinism check (SLO-A1-META-DETERMINISM),
contractivity bound.

**AP5 — External attacker.**
Outside the cluster, reaching ingress.  In Phase 2, meta-swarm is
internal-only ; `efe-server` is the only externally-reachable
service.  AP5 must compromise `efe-server` first → becomes AP3.
Therefore **AP5 is out of scope for Phase 2 / L2**.  In scope at
Phase 5 (customer-facing SLA).

**AP6 — Operator typo.**
An honest mistake : `SIGHUP` to the wrong pod, malformed env var,
deploy without TLS_REQUIRED check, etc.  Risk : medium ; mitigated
by config-time validation (PR #6) + Helm chart guards (PR #14).

**AP7 — Supply chain.**
A malicious crate in `~/.cargo/registry`, a build-script RCE, or a
compromised CI runner.  Out of scope for this document ; handled by
the workspace `cargo audit` runbook + signed crates feature
(`DT-WORKSPACE-SIGNED-CRATES`).

---

## 5. Threat scenarios

Sixteen scenarios.  Each follows the 9-field structure :

```
T-N — short title
├── STRIDE letters
├── Surface (TB id)
├── Personas (AP id)
├── Assets at risk (A id)
├── Description (paragraph)
├── Attack chain (numbered steps)
├── CVSS v3.1 vector + score
├── Severity (SEV-1/2/3/4)
├── Mitigations (layered defence, with implementing PRs)
├── Status
├── Owner
└── Detection
```

### 5.1 Summary table

| ID | Title | STRIDE | TB | Persona | CVSS | Status |
|---|---|---|---|---|:---:|---|
| **T1** | Malicious participant biases consensus | T,I | TB3 | AP2, AP4 | **8.6** | DEFERRED-TO-PR-16 |
| **T2** | MITM on controller ↔ participant | S,T | TB3 | AP4 | **9.1** | DEFERRED-TO-PR-16 |
| T3 | Replay of Iterate response | T,R | TB2 | AP3 | 6.8 | MITIGATED |
| T4 | SIGHUP injection by unauthorised operator | S,E | TB4 | AP6 | 5.4 | DEFERRED-TO-PR-12 |
| **T5** | Manifest tampering via ConfigMap | T,E | TB5 | AP2 | **8.1** | MITIGATED |
| **T6** | mTLS cert rotation failure → outage | D | TB2,TB3 | AP6 | **7.5** | DEFERRED-TO-PR-16 |
| T7 | Resource exhaustion via Iterate flood | D | TB2 | AP3 | 6.5 | MITIGATED |
| T8 | Crafted dim mismatch | T,D | TB2 | AP3 | 6.1 | MITIGATED |
| T9 | Timing side-channel on convergence | I | TB3 | AP4 | 4.3 | **ACCEPTED** |
| T10 | /metrics endpoint info disclosure | I | TB6 | AP1, AP5 | 5.3 | MITIGATED |
| T11 | Trace context spoofing | T,R | TB2,TB3 | AP3, AP4 | 4.8 | MITIGATED |
| T12 | Determinism violation as covert channel | I,R | TB3 | AP4 | 5.5 | MITIGATED |
| **T13** | Stack overflow via crafted oneof | T,D | TB2,TB3 | AP3, AP4 | **7.2** | MITIGATED |
| **T14** | K8s NetworkPolicy bypass | T,E | TB3 | AP2 | **8.7** | DEFERRED-TO-PR-14 |
| **T15** | Cert SAN spoofing | S,E | TB2,TB3 | AP4 | **8.2** | DEFERRED-TO-PR-16 |
| **T16** | Admin priv-esc via ReloadConfig | E | TB4 | AP6, AP2 | **7.8** | DEFERRED-TO-PR-12 |
| T17 | Env-var injection redirects controller to malicious participant | S,T | TB5 | AP2, AP6 | 6.4 | MITIGATED |
| T18 | Log injection via crafted traceparent / RPC payload content | T,R | TB2,TB3 | AP3, AP4 | 4.1 | MITIGATED |
| T19 | gRPC `Info` RPC recon — build_sha + hostname + participant list | I | TB2 | AP1, AP3 | 3.7 | MITIGATED |
| T20 | `deferred_ticket` baseline YAML tampering bypasses anti-Φ-D.A1 | T,E | (workspace YAML) | AP2 | 5.2 | MITIGATED |

**Counts** : 20 total ; **1 SEV-1** + **7 SEV-2** + **10 SEV-3** + **2 SEV-4**.
Status : **12 MITIGATED** + **7 DEFERRED-TO-PR** + 1 ACCEPTED + **0 OPEN**.

### 5.2 Threat detail

#### T1 — Malicious participant biases consensus via crafted state

**STRIDE** : Tampering (T), Information disclosure (I).
**Surface** : TB3 (controller ↔ participant).
**Personas** : AP2 (malicious insider), AP4 (compromised participant pod).
**Assets at risk** : A1 (meta-consensus state), A5 (determinism guarantee).

**Description.**
A participant whose process has been compromised — by an RCE,
sidecar attack, or insider — can return arbitrary `new_state`
values in its `Step` responses.  An attacker who controls the
`pulled_from_meta → new_state` mapping can :

1. Bias the meta-consensus toward an attacker-chosen direction.
2. Cause divergence (residual oscillates) — DoS.
3. Exfiltrate via a covert channel encoded in the LSBs of returned
   states (information disclosure ; partially overlaps T12).

**Attack chain.**
1. Attacker gains code execution inside a participant pod
   (T13 or AP4 entry vector).
2. Attacker modifies the `MetaParticipantService::Step` handler
   to return `pulled_from_meta + ATTACKER_BIAS` instead of the
   contractive map.
3. Controller's `iterate()` runs the meta-operator with this
   biased input.
4. Meta-consensus converges (or diverges) under attacker control.

**CVSS v3.1 vector.**
```
AV:A/AC:H/PR:H/UI:N/S:U/C:H/I:H/A:L/E:F/RL:O/RC:C
```
Base = 6.9, Temporal = 6.3, **Environmental = 8.6** (raised because
the consensus impact propagates to all downstream decisions in
multi-tenant Phase 3).

**Severity** : **SEV-2**.

**Mitigations (layered defence).**

1. **Phase-1 contractive bound** (algorithmic, Phase-1 PR #1) : the
   algorithm itself bounds attacker bias to `(1 − κ_max) × max_step_size`.
   Attacker cannot cause **unbounded** consensus drift — the meta-
   operator's contractivity is a mathematical invariant.
2. **Safety projection** (Phase-1 PR #1, `BoxSafety` in Phase 2) :
   the meta-consensus is projected onto a safety set at every
   meta-step.  An attacker who pushes outside the box is clipped.
3. **Determinism check** (T12 + SLO-A1-META-DETERMINISM) : a biased
   participant would break the 3-rerun bit-identical guarantee on
   every Iterate.  Alert fires at SLO breach.
4. **mTLS + SAN check** (PR #16) : an external attacker cannot
   spoof a participant.  Only AP4 (in-cluster RCE) remains
   feasible.
5. **Phase-3 future control** : cryptographic attestation of step
   outputs (deferred ; tracked as `DT-A2-ATTESTATION`).

**Status** : **DEFERRED-TO-PR-16** (mTLS).  At L2 entry, mTLS reduces
the attacker pool to AP4 (in-cluster RCE) and the contractive bound
limits the impact.  Residual : moderate ; accepted for Phase 2 ;
full cryptographic attestation in Phase 3.

**Owner** : meta-swarm-team (algorithmic bound) + sec (mTLS + future
attestation).

**Detection.**
- `SLO-A1-META-DETERMINISM` breach → alert
- `meta_swarm_consensus_norm` outside expected band → alert (PR #17, PR #19)
- Optional Phase-3 : per-participant signature on returned state

**Test.**
Phase-2 multi-process integration test (PR #10) can be extended
with a "rogue participant" fixture that returns biased states.
Expected outcome : iteration completes within bounds dictated by
contractivity (no unbounded drift).  Tracked as
`DT-A2-ROGUE-PARTICIPANT-TEST`.

---

#### T2 — MITM on controller ↔ participant injects fake Step response

**STRIDE** : Spoofing (S), Tampering (T).
**Surface** : TB3.
**Personas** : AP4 (compromised participant pod can also MITM a sibling).
**Assets at risk** : A1, A2, A6.

**Description.**
An attacker positioned on the cluster pod network between the
controller and a participant intercepts gRPC frames and modifies /
replays them.  Without mTLS, this is trivial for any container that
shares the network namespace.

**Attack chain.**
1. Attacker compromises ANY container in the cluster (e.g. a
   misconfigured sidecar on an unrelated service).
2. Attacker positions to intercept controller → participant
   traffic (e.g. via ARP poisoning in the pod CIDR or by sniffing
   on the same node before NetworkPolicy is enforced).
3. Controller issues `Step` request.
4. Attacker swaps the `pulled_from_meta` field, returning a
   modified response that pushes consensus in attacker's direction.

**CVSS v3.1 vector.**
```
AV:A/AC:H/PR:N/UI:N/S:C/C:H/I:H/A:H/E:F/RL:O/RC:C
```
Base = 9.1, Temporal = 8.7.

**Severity** : **SEV-1**.

**Mitigations.**

1. **mTLS handshake** (PR #16, mandatory at Phase 2 GA) : the
   participant's leaf cert is verified by the controller against
   the cert-manager-issued CA bundle.
2. **SAN check** (PR #16) : the controller accepts only callers
   whose SAN matches `participant-<id>.efe.svc.cluster.local`.
3. **K8s NetworkPolicy** (PR #14) : restricts ingress to the
   participant pod to the controller pod IPs only.  Defence in
   depth — even if mTLS is misconfigured, ingress is blocked.
4. **HTTP/2 frame integrity** : tonic 0.12 validates HTTP/2
   framing ; malformed frames are rejected.
5. **Channel keepalive** (PR #6 `gRPC server timeout = 60 s`) :
   long-running MITM would trigger keepalive failure.

**Status** : **DEFERRED-TO-PR-16** (mTLS).  At PR #14 merge,
NetworkPolicy reduces the attack surface ; PR #16 completes the
mitigation with mTLS.

**Owner** : sec.

**Detection.**
- TLS handshake failure alert (PR #19 + custom)
- gRPC trailer mismatch alert
- Determinism breach alert

---

#### T3 — Replay of Iterate response

**STRIDE** : Tampering (T), Repudiation (R).
**Surface** : TB2.
**Personas** : AP3.
**Assets at risk** : A6, A1.

**Description.**
An attacker intercepts a previous `IterateResponse` and replays it
to `efe-server` in lieu of a fresh response.  Without freshness
controls, downstream consumers may use stale data.

**Attack chain.**
1. Attacker captures a valid `IterateResponse` from a previous
   call.
2. Attacker positions to MITM TB2.
3. New Iterate request from efe-server → attacker drops it and
   returns the cached response.

**CVSS v3.1 vector.**
```
AV:A/AC:H/PR:H/UI:N/S:U/C:L/I:H/A:L/E:F/RL:O/RC:C
```
Base = 5.4, Temporal = 5.0, **Environmental = 6.8**.

**Severity** : SEV-3.

**Mitigations.**

1. **gRPC deadline** (Phase 2 hard cap 30 s) : the response has an
   expiry implicit in the call duration.  A replayed response
   would arrive after the deadline → `DEADLINE_EXCEEDED` from
   client side.
2. **Idempotence + determinism** : two Iterate calls with the same
   input return bit-identical responses (SLO-A1-META-DETERMINISM).
   Replaying = computing twice — no integrity issue beyond duplicate
   computation cost.
3. **W3C TraceParent** (PR #18) : each call has a unique trace_id ;
   a replayed response with stale trace_id is detectable in OTel
   spans.

**Status** : **MITIGATED**.

**Owner** : meta-swarm + sec.

**Detection.**
- OTel span trace_id mismatch alert (PR #18 + #19)
- Deadline-exceeded rate on TB2

---

#### T4 — SIGHUP injection by unauthorised operator

**STRIDE** : Spoofing (S), Elevation of privilege (E).
**Surface** : TB4.
**Personas** : AP6 (operator typo), AP1 (curious).
**Assets at risk** : A3, A9.

**Description.**
An operator with K8s `pods/exec` access to the controller pod runs
`kill -HUP <pid>` (or `kubectl exec ... kill`), triggering an
unintended manifest reload.

**Attack chain.**
1. Operator with `pods/exec` permission on the controller pod
   accidentally targets the meta-swarm controller instead of
   another workload.
2. SIGHUP is delivered.
3. Controller reads the manifest from disk and applies it.
4. If the manifest changed since boot, behaviour shifts unexpectedly.

**CVSS v3.1 vector.**
```
AV:L/AC:L/PR:H/UI:N/S:U/C:N/I:H/A:L/E:H/RL:O/RC:C
```
Base = 4.3, Environmental = 5.4.

**Severity** : SEV-3.

**Mitigations.**

1. **K8s RBAC** : `pods/exec` access is highly restricted (operator
   sub-team only).
2. **`MetaSwarmService::ReloadConfig` admin scope** (PR #12) : the
   alternative path (via gRPC admin RPC) requires the `admin` scope
   on the auth token.  Belt + suspenders.
3. **Audit log of every SIGHUP** (PR #12 emits a structured log
   line at every signal) : forensic trail.
4. **Manifest-tampering alert** (PR #19) : if the manifest file
   changes outside a known deploy window, alert fires.

**Status** : **DEFERRED-TO-PR-12** (the admin scope + audit log are
PR #12's job).

**Owner** : platform + sec.

**Detection.**
- `meta_swarm_reload_total` counter increase outside deploy window
- audit log of SIGHUP received

---

#### T5 — Manifest tampering via ConfigMap edit

**STRIDE** : Tampering (T), Elevation of privilege (E).
**Surface** : TB5.
**Personas** : AP2 (malicious insider).
**Assets at risk** : A3.

**Description.**
An insider with `configmaps/update` permission edits the
`efe-meta-swarm-manifest` ConfigMap to inject participants with
different kappas, seeds, or safety geometries.  On the next reload,
the controller adopts the malicious manifest.

**Attack chain.**
1. Insider has `kubectl edit configmap` on the relevant ConfigMap.
2. Insider modifies a kappa from 0.6 → 0.99 (almost non-contractive),
   slowing convergence and causing soft DoS.
3. Or : insider swaps the safety_geometry kind from `box` to a
   `poly` with adversarial constraints.
4. Next SIGHUP (or periodic reload poll) picks up the new manifest.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:H/UI:N/S:C/C:N/I:H/A:H/E:H/RL:O/RC:C
```
Base = 7.5, Environmental = 8.1.

**Severity** : **SEV-2**.

**Mitigations.**

1. **K8s RBAC** (PR #13) : `configmaps/update` on the
   `efe-meta-swarm-manifest` ConfigMap is restricted to a 2-person
   group (`@efe-platform-leads`).  Other operators cannot edit.
2. **Sealed-secrets for participant seeds** (PR #13) : sensitive
   fields (seeds in particular, to prevent determinism attacks) are
   stored as SealedSecrets, not ConfigMap.
3. **Manifest validation on reload** (PR #12) : the controller
   refuses to apply a manifest whose dimensions changed or whose
   kappas are outside (0, 1).
4. **Audit log + alert** : ConfigMap edits emit K8s audit events ;
   PR #19 alert fires on any change.
5. **Signed manifests (Phase 3+)** : the manifest carries a sigstore
   signature ; controller verifies before applying.  `DT-A2-MANIFEST-SIG`.

**Status** : **MITIGATED** (RBAC + sealed-secrets + reload validation
cover Phase 2 ; signed manifests are Phase 3+).

**Owner** : platform + sec.

**Detection.**
- K8s audit event for ConfigMap update
- `meta_swarm_reload_total{outcome="rejected"}` counter
- Manifest-tampering alert (PR #19)

---

#### T6 — mTLS cert rotation failure → service outage

**STRIDE** : Denial of service (D).
**Surface** : TB2, TB3.
**Personas** : AP6 (operator misconfigures cert-manager).
**Assets at risk** : A4, A6.

**Description.**
Phase-2 mTLS certs are issued by cert-manager with 90-day validity
and auto-renewed at 60 days.  If renewal fails (cert-manager
crashes, CA exhausted, K8s API down), certs expire and TLS
handshakes start failing → service outage.

**Attack chain.**
1. cert-manager is unhealthy for > 30 days (silent failure).
2. Day 90 : leaf cert expires.
3. Next mTLS handshake fails → controller and participant cannot
   communicate.
4. All Iterate calls return `UNAVAILABLE`.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H/E:H/RL:U/RC:C
```
Base = 7.5, Temporal = 6.9.

**Severity** : **SEV-2**.

**Mitigations.**

1. **cert-manager 60-day auto-renewal** (PR #16) : the standard
   cert-manager Certificate resource renews at 2/3 lifetime.
2. **`MetaSwarmCertExpiringSoon` alert** (PR #16 + #19) : fires at
   7 days remaining.
3. **Manual rotation runbook** (`META_SWARM_CERT_EXPIRY.md`,
   PR #16) : if auto-renewal fails, operator can manually rotate.
4. **Per-pod readiness gating** : a pod whose cert is < 1 day from
   expiry flips its Health to `NOT_SERVING` (PR #16 enhancement).

**Status** : **DEFERRED-TO-PR-16** (cert-manager wiring).

**Owner** : sec + platform.

**Detection.**
- `MetaSwarmCertExpiringSoon` alert (7 d threshold)
- `meta_swarm_tls_handshake_failures_total` counter (PR #17)

---

#### T7 — Resource exhaustion : Iterate flood

**STRIDE** : Denial of service (D).
**Surface** : TB2.
**Personas** : AP3.
**Assets at risk** : A1 (availability).

**Description.**
A compromised `efe-server` (or a buggy retry loop) issues 10 000
Iterate calls/sec.  Each call holds a connection slot + worker
thread cycle.  The controller becomes unresponsive to legitimate
traffic.

**Attack chain.**
1. AP3 controls efe-server's outbound RPC scheduler.
2. AP3 issues Iterate calls in a tight loop, ignoring backpressure.
3. Controller hits `MAX_INFLIGHT` limit ; returns RESOURCE_EXHAUSTED.
4. Legitimate efe-server calls fail.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:H/UI:N/S:U/C:N/I:N/A:H/E:F/RL:O/RC:C
```
Base = 4.9, Environmental = 6.5.

**Severity** : SEV-3.

**Mitigations.**

1. **`EFE_META_SWARM_MAX_INFLIGHT` enforcement** (PR #6 config +
   PR #8 runtime) : default 10 concurrent Iterate calls ; beyond
   that, server returns RESOURCE_EXHAUSTED.
2. **gRPC server timeout = 60 s** (PR #6) : stuck calls are reaped.
3. **Per-caller rate limiting** (Phase 3+) : `DT-A2-PER-CALLER-RATE`.
4. **K8s LimitRange / ResourceQuota** : caps CPU + memory at the
   pod level ; cannot consume more than allocated.

**Status** : **MITIGATED** (MAX_INFLIGHT + timeout).

**Owner** : meta-swarm.

**Detection.**
- `meta_swarm_iterate_total{outcome="resource_exhausted"}` rate
- `meta_swarm_inflight` gauge spike

---

#### T8 — Crafted dim mismatch in IterateRequest

**STRIDE** : Tampering (T), Denial of service (D).
**Surface** : TB2.
**Personas** : AP3.
**Assets at risk** : A1.

**Description.**
A compromised efe-server sends an `IterateRequest` with
`expected_participants` whose `dim` field disagrees with the
controller's manifest.  Without validation, this could cause :

- Server-side panic on tensor reshape.
- Slow path that allocates a huge buffer based on the attacker-
  chosen dim.

**Attack chain.**
1. AP3 modifies the request body : sets `dim = u32::MAX`.
2. Controller (without validation) attempts to allocate a tensor
   of `u32::MAX × 4` bytes ≈ 17 GB.
3. OOM kill ; pod restart.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:H/UI:N/S:U/C:N/I:L/A:H/E:F/RL:O/RC:C
```
Base = 5.3, Environmental = 6.1.

**Severity** : SEV-3.

**Mitigations.**

1. **Server-side dim validation** (PR #8) : `IterateRequest.dim`
   must match the controller's manifest exactly ; otherwise
   `META_ERROR_DIM_MISMATCH` returned.
2. **Prost max-message-size = 4 MB** (PR #5) : a request with
   `dim = u32::MAX` would exceed the message-size cap and be
   rejected at the protobuf layer.
3. **`meta_swarm_iterate_total{outcome="config_error"}`** alert if
   spike.

**Status** : **MITIGATED**.

**Owner** : meta-swarm.

**Detection.**
- gRPC `FailedPrecondition` rate
- Custom counter `meta_swarm_dim_mismatch_total`

---

#### T9 — Timing side-channel reveals convergence trajectory

**STRIDE** : Information disclosure (I).
**Surface** : TB3.
**Personas** : AP4.
**Assets at risk** : A1.

**Description.**
An attacker with timing-precise access to a participant pod can
measure the wall time of each `Step` call.  By correlating timing
with the controller's outer-iteration counter, the attacker
reconstructs the convergence trajectory — potentially revealing
features of the underlying tenant workload.

**Attack chain.**
1. AP4 has a stopwatch in the participant container.
2. AP4 records the wall time of each Step (already self-reported
   in `StepResponse.step_us`).
3. AP4 correlates timing patterns with public information about the
   tenant.

**CVSS v3.1 vector.**
```
AV:A/AC:H/PR:H/UI:N/S:U/C:L/I:N/A:N/E:P/RL:U/RC:C
```
Base = 2.5, Environmental = 4.3.

**Severity** : SEV-3.

**Mitigations.**

1. **Phase-2 is internal-only.**  Tenant isolation arrives in
   Phase 3 ; until then, the side-channel only leaks within a
   single internal tenant.
2. **Constant-time iteration** : prohibitively expensive at the
   planned 100 rps target ; adds ~ 30 % latency.

**Status** : **ACCEPTED**.

**Acceptance rationale.**
- Severity is SEV-3 (medium).
- Phase 2 deployment is single-tenant + internal-only — the
  side-channel does not cross any tenant boundary that matters.
- Implementing constant-time iteration would add ~ 30 % latency,
  blowing the `SLO-A1-META-LATENCY-P99 ≤ 100 ms` budget.
- Phase 3 multi-tenant entry triggers re-assessment ; if any
  paying tenant requires constant-time, we implement it then.

**Expiry** : 2027-06-30 (Phase 5 entry).
**Revisit trigger** : a paying tenant signs requiring constant-time
guarantees ; or Phase 3 tenant isolation work flags this as a
blocker.
**Owner** : meta-swarm-team (algorithmic) + product (customer
requirements).

**Detection.**
- N/A : no detection possible for timing side-channels at this layer.

---

#### T10 — /metrics endpoint information disclosure

**STRIDE** : Information disclosure (I).
**Surface** : TB6.
**Personas** : AP1 (curious), AP5 (external — out of scope).
**Assets at risk** : A8.

**Description.**
The Prometheus `/metrics` endpoint exposes operational telemetry
including `meta_swarm_build_info`, `meta_swarm_participant_up`,
and (in Phase 3) potentially tenant ids.  An attacker who can
reach the endpoint can map the deployment topology.

**Attack chain.**
1. AP1 finds the participant pod IPs via `kubectl describe`.
2. AP1 `curl http://<ip>:9092/metrics` from within the cluster.
3. AP1 reads `meta_swarm_build_info{git_sha=…}` → learns version.
4. AP1 reads `meta_swarm_participant_up` → counts replicas.

**CVSS v3.1 vector.**
```
AV:A/AC:L/PR:L/UI:N/S:U/C:L/I:N/A:N/E:H/RL:O/RC:C
```
Base = 3.5, Environmental = 5.3.

**Severity** : SEV-3.

**Mitigations.**

1. **K8s NetworkPolicy** (PR #14) : `/metrics` ingress is restricted
   to the Prometheus scraper pod's IP range.  Other in-cluster
   callers are blocked.
2. **Bounded cardinality** (PR #17) : metric labels are bounded
   (no free-form tenant_id) ; the set of participant ids is fixed.
3. **No PII / customer data in metrics** (style guide, PR #17) :
   metric labels never carry tenant data ; only operational metadata.
4. **Future** : bearer-token auth on `/metrics` (Phase 3 ; `DT-A2-METRICS-AUTH`).

**Status** : **MITIGATED** (NetworkPolicy + bounded cardinality).

**Owner** : platform + sre.

**Detection.**
- `meta_swarm_metrics_requests_total` rate from unexpected sources
- K8s NetworkPolicy drop counter

---

#### T11 — Trace context spoofing via crafted W3C traceparent

**STRIDE** : Tampering (T), Repudiation (R).
**Surface** : TB2, TB3.
**Personas** : AP3, AP4.
**Assets at risk** : A8.

**Description.**
An attacker injects a crafted `traceparent` header to either :

- Cause trace collision in the OTel collector (two unrelated traces
  share the same trace_id) — disrupting forensics.
- Repudiate a real action by injecting a trace_id that points to a
  fabricated "innocent" earlier span.

**Attack chain.**
1. AP3 sets `traceparent: 00-deadbeef…-00f067aa0ba902b7-01` on
   every Iterate call.
2. OTel collector merges traces by trace_id.
3. Forensic investigation pulls all spans for `deadbeef…` and finds
   a tangled mess.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:L/UI:N/S:U/C:N/I:L/A:N/E:F/RL:O/RC:C
```
Base = 4.3, Environmental = 4.8.

**Severity** : SEV-3.

**Mitigations.**

1. **W3C traceparent validation** (PR #18) : the controller and
   participants validate the syntax (32 hex + 16 hex + flags) ;
   malformed traceparents are dropped + replaced with a fresh one.
2. **Sampling cap** (PR #18) : sample ratio capped at 5 % ; spurious
   trace_ids do not flood the collector.
3. **Per-call structured log line** (PR #6 tracing setup) : even if
   OTel spans are tangled, the structured logs preserve causality
   via the per-line `request_id`.

**Status** : **MITIGATED**.

**Owner** : meta-swarm + sre.

**Detection.**
- Spike in `meta_swarm_iterate_total` with malformed traceparents
- OTel collector ingestion-rejected metric

---

#### T12 — Determinism violation as covert channel

**STRIDE** : Information disclosure (I), Repudiation (R).
**Surface** : TB3.
**Personas** : AP4.
**Assets at risk** : A5.

**Description.**
A compromised participant deliberately violates determinism — returns
slightly different states on otherwise-identical Step requests —
encoding a covert channel.  Each bit of leaked information is a
deliberate LSB perturbation.

**Attack chain.**
1. AP4 modifies the Step handler to return
   `step_with_pull(pulled) + secret_bit_to_leak × 1e-7`.
2. Controller computes the joint residual, which now varies by
   `secret_bit` ;  attacker on the controller side (or via metrics)
   reads `meta_swarm_iterate_final_residual` LSB.

**CVSS v3.1 vector.**
```
AV:A/AC:H/PR:H/UI:N/S:U/C:H/I:L/A:N/E:F/RL:O/RC:C
```
Base = 4.6, Environmental = 5.5.

**Severity** : SEV-3.

**Mitigations.**

1. **`SLO-A1-META-DETERMINISM`** (Phase 1 PR #4 + Phase 2 PR #10) :
   3-rerun bit-identical check.  An attacker who perturbs LSBs
   breaks the SLO on every Iterate.
2. **`MetaSwarmDeterminismBreach` alert** (PR #17 + #19) : fires
   immediately on SLO breach.
3. **Audit log of every Iterate's `(iterations, residual.to_bits())`
   tuple** (in `meta_swarm_extras.json` sidecar, Phase-1) : forensic
   trail.

**Status** : **MITIGATED**.

**Owner** : meta-swarm + sec.

**Detection.**
- `SLO-A1-META-DETERMINISM` breach → alert
- Forensic walk of the audit log

---

#### T13 — Stack overflow in proto parsing via deeply-nested oneof

**STRIDE** : Tampering (T), Denial of service (D).
**Surface** : TB2, TB3.
**Personas** : AP3, AP4.
**Assets at risk** : A6, A1 (availability).

**Description.**
An attacker crafts a protobuf message with deeply-nested oneof
variants (e.g. exploiting the `MetaSpaceConfig.safety_geometry`
oneof) to cause stack overflow during parsing.

**Attack chain.**
1. AP3 constructs a message with a tree of recursive oneof
   wrappers exceeding the stack's max depth.
2. Controller's prost decoder recursively descends → stack overflow.
3. Process crash.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H/E:F/RL:O/RC:C
```
Base = 7.5, Environmental = 7.2.

**Severity** : **SEV-2**.

**Mitigations.**

1. **Prost max-message-size = 4 MB** (PR #5 build.rs) : protobuf
   messages > 4 MB are rejected at the gRPC framing layer.
2. **Prost decode-depth limit** (default in prost 0.13 = 100
   levels) : sufficient depth budget for our oneofs (max 2 levels).
3. **gRPC server timeout = 60 s** (PR #6) : even if parsing is slow,
   the call is reaped.
4. **`MetaParticipantService` is mTLS-authenticated** (PR #16) :
   only the controller can send messages to participants — AP3 must
   compromise the controller first.

**Status** : **MITIGATED**.

**Owner** : meta-swarm.

**Detection.**
- `meta_swarm_iterate_total{outcome="parse_error"}` rate
- Sudden pod restart from controller (OOMKilled or panic)

---

#### T14 — K8s NetworkPolicy bypass via misconfigured CNI

**STRIDE** : Tampering (T), Elevation of privilege (E).
**Surface** : TB3.
**Personas** : AP2.
**Assets at risk** : A6.

**Description.**
Phase-2 relies on K8s NetworkPolicy to restrict ingress between
controller and participant pods.  If the cluster's CNI plugin
doesn't enforce NetworkPolicy (e.g. flannel-without-calico), an
attacker can bypass it and reach participants directly.

**Attack chain.**
1. Cluster operator deploys with a CNI that ignores NetworkPolicy.
2. AP2 (insider) starts a pod in the same namespace.
3. The new pod can reach `participant-supplier-scoring:50090`
   directly.
4. Without mTLS (Phase 2 plaintext window), AP2 sends crafted Step
   responses ; covert channel established.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:L/UI:N/S:U/C:N/I:H/A:N/E:H/RL:O/RC:C
```
Base = 6.5, Environmental = 8.7 (raised because consensus impact
propagates).

**Severity** : **SEV-2**.

**Mitigations.**

1. **NetworkPolicy template in Helm chart** (PR #14) : ship the
   policy by default in the chart.
2. **`helm template` golden-diff CI check** (PR #15) : verifies the
   NetworkPolicy resource is emitted with correct selectors.
3. **CNI compatibility doc** (PR #14 README) : platform team
   documents the supported CNI list (calico, cilium, aws-vpc-cni
   with NP enabled).
4. **mTLS as defence in depth** (PR #16) : even with bypassed
   NetworkPolicy, mTLS SAN check prevents unauthorized callers.

**Status** : **DEFERRED-TO-PR-14** (NetworkPolicy ship) ; mTLS
(PR #16) provides defence in depth.

**Owner** : platform + sec.

**Detection.**
- `meta_swarm_participant_unexpected_caller_total` counter (PR #17)
- K8s audit event for unusual pod-to-pod traffic

---

#### T15 — Cert SAN spoofing via wildcard mismatch

**STRIDE** : Spoofing (S), Elevation of privilege (E).
**Surface** : TB2, TB3.
**Personas** : AP4.
**Assets at risk** : A4, A6.

**Description.**
If the SAN allowlist on the controller side accepts a wildcard
(`*.efe.svc.cluster.local`), an attacker who can obtain a cert for
ANY pod in the cluster can impersonate a participant.

**Attack chain.**
1. AP4 compromises an unrelated pod that has a cert-manager-issued
   cert for `some-other-service.efe.svc.cluster.local`.
2. AP4 connects to the controller posing as a participant.
3. Controller's wildcard SAN check accepts the cert.
4. Attacker now receives Step calls from the controller and can
   bias consensus (→ T1).

**CVSS v3.1 vector.**
```
AV:A/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:L/E:H/RL:O/RC:C
```
Base = 8.6, Environmental = 8.2.

**Severity** : **SEV-2**.

**Mitigations.**

1. **Strict SAN allowlist** (PR #16) : the controller accepts ONLY
   the exact SANs `participant-<id>.efe.svc.cluster.local` for
   each configured `<id>` ; no wildcards.
2. **`MetaSwarmAuthFailures` alert** (PR #19) : fires on SAN-check
   rejection rate.
3. **cert-manager Certificate resource ships exact CN + SAN**
   (PR #16) : a wildcard cert is never issued by the chart.

**Status** : **DEFERRED-TO-PR-16** (mTLS shipping).

**Owner** : sec.

**Detection.**
- `meta_swarm_auth_failures_total{reason="san_mismatch"}` rate
- K8s audit events for cert-manager Certificate creates outside
  the chart's namespace

---

#### T16 — Admin privilege escalation via ReloadConfig RPC

**STRIDE** : Elevation of privilege (E).
**Surface** : TB4 (and TB2 for the RPC variant).
**Personas** : AP6 (typo), AP2 (intentional).
**Assets at risk** : A3, A9.

**Description.**
The `MetaSwarmService::ReloadConfig` RPC accepts an inline manifest
(`new_meta`, `new_participants`).  Without strict scope enforcement,
any caller could replace the running manifest with arbitrary content.

**Attack chain.**
1. AP2 obtains a bearer token (e.g. from a misconfigured pod's
   service-account-token volume).
2. AP2 calls `ReloadConfig(new_participants=...)` on the controller.
3. Without scope check, controller applies the inline manifest.
4. Controller now uses attacker-chosen participant set.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:H/UI:N/S:C/C:N/I:H/A:H/E:F/RL:O/RC:C
```
Base = 7.2, Environmental = 7.8.

**Severity** : **SEV-2**.

**Mitigations.**

1. **`admin` scope on ReloadConfig** (PR #12) : the controller's
   middleware checks the caller's auth token has the `admin` scope
   ; otherwise returns `UNAUTHENTICATED`.
2. **mTLS + SAN check** (PR #16) : only certs with a specific
   `admin-...` SAN can invoke admin RPCs.
3. **K8s RBAC + service-account tokens with limited audiences**
   (Phase 3) : `DT-A2-SA-AUDIENCE`.
4. **Audit log of every ReloadConfig** (PR #12) : forensic trail.
5. **`MetaSwarmAdminRpcSpike` alert** (PR #19) : fires on > 1
   ReloadConfig / minute.

**Status** : **DEFERRED-TO-PR-12** (admin scope + audit log) +
PR #16 (cert-based admin SAN).

**Owner** : meta-swarm + sec.

**Detection.**
- `meta_swarm_admin_rpc_total{outcome="ok"}` rate
- `MetaSwarmAdminRpcSpike` alert
- Audit log of every ReloadConfig request

---

#### T17 — Env-var injection redirects controller to malicious participant

**STRIDE** : Spoofing (S), Tampering (T).
**Surface** : TB5 (analogous : operator → process env).
**Personas** : AP2 (malicious insider), AP6 (operator typo).
**Assets at risk** : A6, A1.

**Description.**
The controller reads `EFE_META_SWARM_PARTICIPANTS` from env vars
at startup.  An insider who can edit the Deployment / StatefulSet's
PodSpec (`spec.template.spec.containers[*].env`) can substitute the
participant addresses with attacker-controlled endpoints.  The
controller would happily connect to malicious participants on
restart.

**Attack chain.**
1. AP2 has `deployments/update` RBAC on the meta-swarm namespace.
2. AP2 edits the controller Deployment :
   `EFE_META_SWARM_PARTICIPANTS=supplier-scoring=evil-host:50091,…`
3. K8s rolling-restart of controller pod.
4. New controller connects to `evil-host:50091` instead of the
   legitimate participant Service.
5. Attacker now sends arbitrary Step responses (→ T1).

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:H/UI:N/S:C/C:L/I:H/A:L/E:H/RL:O/RC:C
```
Base = 6.4, Temporal = 6.0.

**Severity** : SEV-3.

**Mitigations.**

1. **K8s RBAC** (PR #13) : `deployments/update` on the meta-swarm
   namespace is restricted to a 2-person group
   (`@efe-platform-leads`).  Identical control as for T5
   (ConfigMap tampering).
2. **mTLS + SAN check** (PR #16) : the controller validates the
   participant's SAN against the **expected** static set ; an
   attacker-controlled endpoint won't present a valid cert with
   `participant-<id>.efe.svc.cluster.local` SAN.
3. **Helm chart pins endpoints** (PR #13) : env vars are rendered
   from chart values ; out-of-band edits trigger `helm diff` drift
   alert (PR #14 + sre runbook).
4. **K8s audit log** : every Deployment update emits an audit event ;
   `MetaSwarmAdminRpcSpike`-style alert can be added for unexpected
   Deployment edits.

**Status** : **MITIGATED** (RBAC + mTLS SAN check + chart pinning).

**Owner** : platform + sec.

**Detection.**
- K8s audit event for `Deployment` update outside deploy window
- `MetaSwarmAuthFailures{reason="san_mismatch"}` rate spike on
  startup (attacker endpoint can't present valid cert)
- Helm diff drift alert (chart-rendered values vs running pod env)

---

#### T18 — Log injection via crafted traceparent or RPC payload content

**STRIDE** : Tampering (T), Repudiation (R).
**Surface** : TB2, TB3.
**Personas** : AP3, AP4.
**Assets at risk** : A8 (operational telemetry).

**Description.**
An attacker injects newline characters (`\n`), ANSI escape
sequences, or fake log markers (`[ERROR]`, `timestamp=...`) into
RPC payload fields that get logged (e.g. `traceparent`,
`participant_id` in error messages).  The injected content can :

- Forge log lines that appear to originate from another component.
- Inject log markers that confuse log parsers / SIEM systems.
- Pollute forensic timelines by emitting bogus "evidence".

**Attack chain.**
1. AP3 sets `traceparent: "valid-id\n[INFO] fake forensic event"`.
2. Controller's tracing-subscriber logs the traceparent verbatim
   in some debug paths.
3. Log line in stdout now appears as TWO entries — the second
   crafted by the attacker.
4. SIEM ingestion sees two events ; forensic timeline is poisoned.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:L/UI:N/S:U/C:N/I:L/A:N/E:P/RL:O/RC:C
```
Base = 3.5, Temporal = 3.2 → adjusted upward (Env) to **4.1**.

**Severity** : SEV-4.

**Mitigations.**

1. **JSON-formatted structured logs** (PR #6 `tracing.rs::init` :
   JSON output when stderr is not a TTY) : newlines in user input
   are escaped as `\n` inside JSON strings.  Log parsers cannot be
   confused.
2. **W3C traceparent validation** (PR #18) : the controller
   validates the traceparent syntax (32 hex + 16 hex + flags) and
   rejects any input containing non-hex characters.  This catches
   `\n` injection at the protocol layer.
3. **Bounded log labels** (PR #17) : metric / log labels have a
   fixed vocabulary ; free-form user input never enters labels.
4. **Tracing-subscriber max-field-len** (default 8 KB) : oversized
   fields are truncated.

**Status** : **MITIGATED** (JSON logs + traceparent validation).

**Owner** : meta-swarm + sre.

**Detection.**
- Log parser warning rate (SIEM-side)
- Traceparent-malformed counter (PR #17)
- Manual review of any forensic anomaly

---

#### T19 — gRPC `Info` RPC recon (build_sha + hostname + participant list)

**STRIDE** : Information disclosure (I).
**Surface** : TB2 (efe-server → controller `Info`).
**Personas** : AP1 (curious insider), AP3 (compromised efe-server).
**Assets at risk** : A7 (Health + Info responses).

**Description.**
The controller's `MetaSwarmService::Info` RPC returns operational
metadata : `build_sha` (8-char git hash), `proto_version` ("v1"),
`leader_pod_name` (hostname / pod name), `configured_participants`
(list of participant ids).  An attacker with TB2 access can mine
this info for :

- **CVE matching** : `build_sha` reveals exact code version → look
  up known-CVE list of the dep tree at that SHA.
- **Topology recon** : participant list maps the consensus group ;
  useful pre-stage to T1 (target which participant to compromise).
- **Pod-name guessing** : in K8s, `leader_pod_name` reveals the
  StatefulSet ordinal index ; helpful for chaos / DoS targeting.

**Attack chain.**
1. AP3 has reached the controller via TB2 (mTLS handshake passed —
   AP3 controls efe-server which has a valid SAN).
2. AP3 calls `Info` ; gets full operational metadata.
3. AP3 cross-references `build_sha` with public CVE list ; finds
   known vuln in transitive dep.
4. AP3 plans targeted exploit.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:H/UI:N/S:U/C:L/I:N/A:N/E:H/RL:O/RC:C
```
Base = 2.7, Environmental = 3.7.

**Severity** : SEV-4.

**Mitigations.**

1. **mTLS + admin scope on Info** (PR #16) : Info is callable only
   by callers presenting a valid efe-server SAN cert.  External
   attackers cannot reach it.
2. **Build-sha rotation discipline** : every Phase-2 release tag
   bumps the build_sha ; CVE-prone SHAs are evicted from prod
   within 1 sprint of disclosure (workspace standing).
3. **Bounded `configured_participants` info** : Info returns only
   participant ids (already public via NetworkPolicy selector
   labels) ; never returns kappa/seed or other sensitive config.
4. **Hostname privacy** : Phase-3 may scrub `leader_pod_name` if
   tenant-isolation requires it (`DT-A3-INFO-HOSTNAME-SCRUB`).

**Status** : **MITIGATED** (mTLS + bounded payload).

**Owner** : meta-swarm + sec.

**Detection.**
- `meta_swarm_info_rpc_total` rate from unexpected callers
  (PR #17 + custom)
- gRPC trailer for unauthorized Info calls

---

#### T20 — `deferred_ticket` baseline YAML tampering bypasses anti-Φ-D.A1

**STRIDE** : Tampering (T), Elevation of privilege (E).
**Surface** : workspace YAML (not a runtime trust boundary ; covered
under repo audit).
**Personas** : AP2 (malicious insider with PR write access).
**Assets at risk** : A10 (`deferred_ticket` field).

**Description.**
The Phase-1 anti-Φ-D.A1 doctrine forbids silent skips : every SLO
is `ACTIVE` or `SKIP-DT` (with a `deferred_ticket` field).  An
insider with merge rights could add a fake `deferred_ticket`
(`DT-FAKE-...`) to a newly-introduced SLO that would otherwise
break the master gate, **bypassing** the doctrine without raising
alarms.

**Attack chain.**
1. AP2 opens a PR that adds a new SLO to `bench_baselines/v0.10.0/meta_swarm_slos.yaml` (or similar baseline).
2. AP2 sets `deferred_ticket: DT-FAKE-NEVER-RESOLVED-IGNORE`.
3. PR review : sec-team-lead doesn't notice the made-up DT id.
4. PR merges ; master gate now tolerates SKIP-SILENT for the new
   SLO without a real ticket tracking.
5. Production drift goes unnoticed for weeks.

**CVSS v3.1 vector.**
```
AV:N/AC:L/PR:H/UI:R/S:U/C:N/I:H/A:L/E:P/RL:O/RC:C
```
Base = 4.5, Environmental = 5.2.

**Severity** : SEV-3.

**Mitigations.**

1. **2-eyes PR review** (workspace standing) : any change to
   `bench_baselines/**` requires a CODEOWNERS-driven sec-team
   review.  AP2 alone cannot merge.
2. **DT registry** : a workspace-wide `docs/deferred_tickets.md`
   lists every valid DT id ; CI lint rejects `deferred_ticket`
   values not in the registry (proposed `DT-A2-DT-REGISTRY-CI`).
3. **Quarterly DT audit** (sec-team standing) : every quarter,
   sec-team walks every active `deferred_ticket` and verifies it
   maps to a real Jira / Linear ticket with an expiry date.
4. **Phase 3+** : sigstore signing of baseline YAML (`DT-A2-BASELINE-SIG`).

**Status** : **MITIGATED** (2-eyes review + DT registry CI lint).

**Owner** : sec + sre (DT registry custodian).

**Detection.**
- CI lint failure on unregistered `deferred_ticket` id
- Quarterly DT audit report
- `master_gate_check.py` flags SKIP-DT with unregistered DT id

---

## 6. Mitigations matrix

Cross-reference : each mitigation → which threats it covers → which
PR implements it.

| Mitigation | Implementing PR | Threats covered |
|---|---|---|
| mTLS handshake controller ↔ participant + efe-server ↔ controller | **PR #16** | T1, T2, T15 |
| Strict SAN allowlist (no wildcards) | PR #16 | T2, T15 |
| K8s NetworkPolicy (Helm chart) | **PR #14** | T2, T10, T14 |
| ConfigMap RBAC + sealed-secrets for seeds | **PR #13** | T5 |
| cert-manager 60-day auto-renewal + 7d alert | PR #16 | T6 |
| `EFE_META_SWARM_MAX_INFLIGHT` enforcement | PR #6 (config) + PR #8 (runtime) | T7 |
| Server-side dim validation | **PR #8** | T8 |
| Prost max-message-size = 4 MB | PR #5 + PR #8 | T13 |
| Prost decode-depth limit (default) | PR #5 | T13 |
| `admin` scope on ReloadConfig | **PR #12** | T4, T16 |
| Audit log of SIGHUP + ReloadConfig | PR #12 | T4, T16 |
| W3C traceparent validation | PR #18 | T11 |
| OTel sampling cap 5 % | PR #18 | T11 |
| Determinism SLO + 3-rerun check | Phase 1 (PR #4) + PR #10 | T1, T12 |
| gRPC deadline + idempotence | PR #6 + PR #8 | T3 |
| gRPC server timeout 60 s | PR #6 | T2 (long-running MITM), T7, T13 |
| Phase-1 contractivity (algorithmic) | Phase 1 (PR #1) | T1 |
| BoxSafety projection (algorithmic) | Phase 1 (PR #1) | T1 |
| K8s RBAC `deployments/update` restriction | PR #13 | T5, T17 |
| Helm chart pins endpoints + drift alert | PR #13 + PR #14 | T17 |
| JSON-formatted structured logs (escapes `\n`) | PR #6 | T18 |
| Bounded log labels (no free-form input) | PR #17 | T18 |
| mTLS + admin scope on `Info` RPC | PR #16 | T19 |
| Build-sha rotation discipline (per release tag) | workspace standing | T19 |
| Bounded `Info` payload (no kappa/seed leak) | PR #8 (Info impl) | T19 |
| Phase-1 baseline YAML 2-eyes review (anti-Φ-D.A1 doctrine) | workspace standing | T20 |
| `deferred_ticket` registry + CI lint | DT-A2-DT-REGISTRY-CI | T20 |
| Quarterly DT audit (sec-team standing) | workspace standing | T20 |
| Sigstore signing of baseline YAML (Phase 3) | DT-A2-BASELINE-SIG | T20 |

### 6.1 PRs that ship a security control

| PR | Sec-controls delivered | sec-team review required ? |
|---|---|---|
| PR #5 (proto) | Max-message-size in build.rs ; decode-depth defaults | ✅ done (PR #5 review) |
| PR #6 (server scaffold) | MAX_INFLIGHT config field ; gRPC server timeout 60 s | ✅ already merged with sec sign-off |
| PR #8 (Iterate body) | Dim validation, MAX_INFLIGHT runtime enforcement, idempotence | ⏳ yes |
| PR #12 (SIGHUP + ReloadConfig) | Admin scope check, audit log | ⏳ yes |
| PR #13 (Helm sub-chart) | ConfigMap RBAC, sealed-secrets template | ⏳ yes |
| PR #14 (Helm full) | NetworkPolicy templates | ⏳ yes |
| PR #16 (mTLS) | mTLS handshake, SAN check, cert-manager Certificate resource | ⏳ yes — full sec-team review |
| PR #17 (Prometheus) | Bounded cardinality on labels | ⏳ yes |
| PR #18 (OTel) | Traceparent validation, sampling cap | ⏳ yes |

---

## 7. Residual risk register

The threats explicitly **ACCEPTED** in Phase 2, with rationale +
expiry + revisit trigger.

| ID | Threat | CVSS | Rationale | Expiry | Revisit trigger | Owner |
|---|---|:---:|---|---|---|---|
| T9 | Timing side-channel reveals convergence trajectory | 4.3 | Internal-only Phase 2 ; tenant isolation is Phase 3 ; cost of constant-time iteration is prohibitive for ~ 100 rps target (~ 30 % latency overhead → blows SLO-A1-META-LATENCY-P99). | 2027-06-30 (Phase 5 entry) | Customer signs requiring constant-time guarantees ; or Phase 3 tenant isolation work flags this as a blocker. | meta-swarm + product |

**Only 1 ACCEPTED risk** in Phase 2.  All other CVSS-low risks
(SEV-4) are MITIGATED by default workspace controls (TLS, RBAC,
audit log).

---

## 8. Monitoring + alerting

Phase 2 ships **8 alert rules** (4 from PR #19 architecture + 4
proposed by this threat model).

### 8.1 Alert table

| Alert | Threshold | Threats covered | PR | Severity | Routes to |
|---|---|---|---|---|---|
| `MetaSwarmHighFailureRate` | > 2 % failure / 5 min | T1, T7, T8 | PR #19 | critical | PagerDuty |
| `MetaSwarmParticipantDown` | `participant_up == 0` / 1 min | T1 (suspicious), T6 | PR #19 | critical | PagerDuty |
| `MetaSwarmReloadFail` | `reload_fail > 0` / 10 min | T4, T16 | PR #19 | major | Slack #sre |
| `MetaSwarmLatencyP99` | p99 > 100 ms | T7, T13 | PR #19 | major | Slack #sre |
| `MetaSwarmAuthFailures` | `auth_failures > 10` / min | T2, T15, T16 | PR #19 (new) | critical | PagerDuty |
| `MetaSwarmCertExpiringSoon` | `cert_days_remaining < 7` | T6 | PR #16 | major | Slack #platform |
| `MetaSwarmDeterminismBreach` | `determinism != 1` | T1, T12 | PR #17 + #19 | major | Slack #meta-swarm |
| `MetaSwarmAdminRpcSpike` | admin_rpc_total rate > 1/min | T4, T16 | PR #12 + #19 | major | Slack #sre |

All 8 alerts route via the existing Alertmanager configuration ;
escalation policies are inherited from `ops_master_gate_fail.md`.

### 8.2 Dashboard coverage

The Phase-2 Grafana dashboards (PR #19) include 3 panels dedicated
to security signals :

- "Auth failures by reason" (sourced from `MetaSwarmAuthFailures` ;
  reasons : `san_mismatch`, `cert_invalid`, `scope_denied`, `token_expired`)
- "TLS cert days remaining" (per pod)
- "Determinism breach events" (timeline)

---

## 9. Incident response hooks

Each SEV-1 / SEV-2 threat has a runbook in `docs/runbooks/`.

### 9.1 Runbook table

| Runbook | Sev1/2 threats covered | PR creating | Status |
|---|---|---|---|
| `META_SWARM_DETERMINISM_BREACH.md` | T1, T12 | PR #24 | DEFERRED |
| `META_SWARM_TLS_FAILURE.md` | T2 | PR #16 | DEFERRED |
| `META_SWARM_MANIFEST_TAMPERED.md` | T5 | PR #24 | DEFERRED |
| `META_SWARM_CERT_EXPIRY.md` | T6 | PR #16 | DEFERRED |
| `META_SWARM_NETPOL_BYPASS.md` | T14 | PR #24 | DEFERRED |

Each runbook follows the workspace template `_template.md` :
Symptoms → Triage → Mitigation → Post-mortem.

### 9.2 Post-mortem template addition

The workspace postmortem template (`docs/postmortems/_template.md`)
is extended (in PR #25) with a "Meta-swarm threat reference" field
linking the incident to its catalogued threat ID.

---

## 10. Re-assessment cadence

The threat model is a **living document** with explicit triggers
for re-walk.

| Trigger | Action | Owner | Cadence |
|---|---|---|---|
| New PR adds a network surface (RPC, port) | Update § 3 (trust boundaries) + § 5 (threats) | PR author + sec-team | per PR |
| New PR removes a mitigation | Revoke `MITIGATED` status ; reassess CVSS | PR author + sec-team | per PR |
| Phase gate (L2, L3, L4, L5) | Full re-walk of all threats | sec-team lead | per gate |
| Sev1 incident on Phase-2 surface | Same-week patch + post-mortem amends model | sre + sec | per incident |
| New attacker persona observed | Add to § 4 + matrix | sec | as-needed |
| Periodic (every 6 months) | Re-walk in a quarterly sec-team review | sec-team lead | quarterly |

The frontmatter `last_review` field is the authoritative stamp.
CI lint warns if the stamp drifts > 7 months (added in PR #24).

---

## 11. Glossary

- **STRIDE** : Microsoft threat modelling taxonomy (Spoofing,
  Tampering, Repudiation, Information disclosure, DoS, Elevation
  of privilege).
- **CVSS v3.1** : FIRST Common Vulnerability Scoring System,
  version 3.1.  See `https://www.first.org/cvss/specification-document`.
- **DREAD** : Microsoft secondary scoring (Damage, Reproducibility,
  Exploitability, Affected users, Discoverability).
- **mTLS** : mutual TLS — both client and server present + validate
  certificates.
- **SAN** : Subject Alternative Name field in an X.509 certificate.
- **CNI** : Container Network Interface (K8s network plugin layer).
- **K8s RBAC** : Role-Based Access Control in Kubernetes.
- **sealed-secrets** : `bitnami/sealed-secrets` operator that
  encrypts K8s Secrets so they can be checked into git.
- **TB** : Trust Boundary (cf. § 3).
- **AP** : Attacker Persona (cf. § 4).
- **T-N** : Threat scenario number `N` (cf. § 5).
- **Anti-Φ-D.A1** : workspace doctrine forbidding silent skips (every
  SLO is `ACTIVE` or `SKIP-DT` — no `SKIP-SILENT`).
- **L2 gate** : Phase 2 graduation criteria (cf.
  [`EFE_A1_PHASE2_ARCHITECTURE.md`](../EFE_A1_PHASE2_ARCHITECTURE.md) § 12) ; criterion G8 = "threat model signed off".
- **DT-A2-...** : Deferred Ticket id for a Phase-2 follow-up.

---

## Appendix A — CVSS scoring details

The 7 SEV-1 / SEV-2 threats with full CVSS metric breakdown for
reviewer reproducibility.

### A.1 T2 — MITM on controller ↔ participant

| Metric | Value | Rationale |
|---|---|---|
| AV (Attack Vector) | Adjacent (A) | Requires same K8s pod network namespace |
| AC (Attack Complexity) | High (H) | Requires positioning between pods (sidecar / ARP) |
| PR (Privileges Required) | None (N) | Once on the network, no auth needed |
| UI (User Interaction) | None (N) | Fully automated |
| S (Scope) | Changed (C) | Compromises both controller + participant |
| C (Confidentiality) | High (H) | Reads all Step messages |
| I (Integrity) | High (H) | Modifies Step messages |
| A (Availability) | High (H) | Can drop/delay → DoS |

Base vector :
```
AV:A/AC:H/PR:N/UI:N/S:C/C:H/I:H/A:H
```
**Base = 9.1 → SEV-1.**

Temporal :
| Metric | Value |
|---|---|
| E (Exploit Code Maturity) | Functional (F) — MITM tools exist generically |
| RL (Remediation Level) | Official Fix (O) — mTLS is standard |
| RC (Report Confidence) | Confirmed (C) |

Temporal = **8.7**.

**Status** : DEFERRED-TO-PR-16.

### A.2 T1 — Malicious participant biases consensus

| Metric | Value | Rationale |
|---|---|---|
| AV | Adjacent (A) | In-cluster pod |
| AC | High (H) | Requires RCE inside participant |
| PR | High (H) | Operator-level access to deploy malicious image |
| UI | None (N) | Auto |
| S | Unchanged (U) | Stays in meta-swarm scope |
| C | High (H) | Reads all pulled states |
| I | High (H) | Modifies returned states |
| A | Low (L) | Phase-1 contractivity bounds drift |

Base : `AV:A/AC:H/PR:H/UI:N/S:U/C:H/I:H/A:L` → 6.9
Environmental (raised by multi-tenant impact projection) : **8.6 → SEV-2**.

### A.3 T5 — Manifest tampering via ConfigMap

| Metric | Value | Rationale |
|---|---|---|
| AV | Network (N) | K8s API call |
| AC | Low (L) | Standard `kubectl edit` |
| PR | High (H) | Requires `configmaps/update` RBAC |
| UI | None (N) | Auto |
| S | Changed (C) | Affects entire meta-swarm |
| C | None (N) | Manifest is not secret |
| I | High (H) | Wholly replaces algorithm config |
| A | High (H) | Bad kappa → divergence → service degraded |

Base : `AV:N/AC:L/PR:H/UI:N/S:C/C:N/I:H/A:H` → 7.5
Environmental : **8.1 → SEV-2**.

### A.4 T6 — Cert rotation failure

```
AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H
```
Base = 7.5 → **SEV-2**.

### A.5 T13 — Stack overflow via crafted oneof

```
AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H
```
Base = 7.5.  Environmental reduces to **7.2** (mitigated by 4 MB
cap) → **SEV-2**.

### A.6 T14 — NetworkPolicy bypass

```
AV:N/AC:L/PR:L/UI:N/S:U/C:N/I:H/A:N
```
Base = 6.5.  Environmental **8.7** (raised by consensus-impact
projection) → **SEV-2**.

### A.7 T15 — SAN spoofing

```
AV:A/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:L
```
Base = 8.6 → **SEV-2**.  Environmental **8.2**.

### A.8 T16 — Admin priv-esc via ReloadConfig

```
AV:N/AC:L/PR:H/UI:N/S:C/C:N/I:H/A:H
```
Base = 7.2 → **SEV-2**.  Environmental **7.8**.

---

## Appendix B — Cross-references

- Phase 2 architecture security model : [`EFE_A1_PHASE2_ARCHITECTURE.md`](../EFE_A1_PHASE2_ARCHITECTURE.md) § 15
- Operational doctrine : [`EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](../EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md)
- Phase 2 roadmap (PR #7 entry) : [`EFE_A1_PHASE2_ROADMAP.md`](../EFE_A1_PHASE2_ROADMAP.md) § 2.1
- PR #7 detailed architecture : [`EFE_A1_PHASE2_PR07_ARCHITECTURE.md`](../EFE_A1_PHASE2_PR07_ARCHITECTURE.md)
- Sister architectures : [PR #5 (proto)](../EFE_A1_PHASE2_PR05_ARCHITECTURE.md), [PR #6 (server scaffold)](../EFE_A1_PHASE2_PR06_ARCHITECTURE.md)
- Workspace runbook (master gate fail) : [`runbooks/ops_master_gate_fail.md`](../runbooks/ops_master_gate_fail.md)
- FIRST CVSS v3.1 calculator : `https://www.first.org/cvss/calculator/3.1`
- FIRST CVSS v3.1 specification : `https://www.first.org/cvss/specification-document`
- STRIDE reference : Microsoft Threat Modelling guidance
- L2 graduation criteria : [`EFE_A1_PHASE2_ARCHITECTURE.md`](../EFE_A1_PHASE2_ARCHITECTURE.md) § 12 (G8)

---

## Sign-off

The L2 graduation gate criterion G8 requires the following 4
signatures :

| Role | Name | Date | Signature |
|---|---|---|---|
| sec-team lead | R. Kapoor | 2026-MM-DD | ☐ |
| sec-team second pair (red-team) | E. Vargas | 2026-MM-DD | ☐ |
| meta-swarm-team representative | J. Tan | 2026-MM-DD | ☐ |
| sre-team representative | L. Ortiz | 2026-MM-DD | ☐ |

Each signer attests that :
- They have walked all 16 threats and concur with the STRIDE,
  CVSS, status, and mitigation analysis.
- They confirm that **zero** SEV-1 threat is in `ACCEPTED` or
  `OPEN` state.
- They confirm that every `DEFERRED-TO-PR-N` references a PR
  scheduled in Phase 2.
- They confirm that the `ACCEPTED` risk (T9) carries explicit
  rationale + expiry + owner.

At G8 satisfaction, the L2 verification ceremony (Mon 2026-08-24)
marks G8 as ✅ in the L2 verification report.

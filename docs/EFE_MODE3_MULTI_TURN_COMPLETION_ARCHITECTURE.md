# EFE — Mode-3 (Multi-Turn Conversation Anchors) Completion Architecture (Φ-D-MODE3, D70 → D74)

> Document architecture **complétion** pour Mode-3 (= multi-turn
> conversation-aware dynamic anchors). Lecture obligatoire **avant**
> `EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md` (= archi originale M22
> D65-D69) car les artefacts livrés ici **ferment** le contrat opposable
> Mode-3 :
>
> 1. Convergence : ≥ 90 % des sessions convergent en ≤ 5 turns.
> 2. Cost per session : p99 ≤ $0.10 (= ~ 5× Mode-2 single anchor).
> 3. Latency per turn : p99 ≤ 35 s (= ~ 1.2× Mode-2 anchor latency).
> 4. GDPR Art. 22 : 100 % des sessions ont opt-in tracé + opt-out fonctionnel.
> 5. Human review workflow : escalation p99 ≤ 5 min.
> 6. Audit chain per-session : HMAC chain integrity 100 %.
> 7. Fallback to Mode-2 : 3 trigger reasons fonctionnels.
> 8. Session isolation : 0 cross-session anchor pollution.
>
> **Sprint** : Φ-D-MODE3 (D70 → D74, 5 jours).
> **Statut** : architecture **prête à exécution** — code drop joint.
> **Doctrine** : aucune session n'évolue sans consent ; aucun turn n'échappe à l'audit ; aucune divergence ne reste silencieuse ; toute escalade humaine est tracée + temps-bornée ; toute évolution d'anchor est replayable bit-identique.

---

## Table des matières

- **§ 0** — Méta, conventions, glossaire chirurgical
- **§ 1** — Mission Φ-D-MODE3 & contrat opposable
- **§ 2** — Position vs archi M22 D65-D69 (= gap analysis)
- **§ 3** — État AS-IS chirurgical
- **§ 4** — Décomposition du pipeline Mode-3 multi-turn (= 9 étages mesurables)
- **§ 5** — Surface 1 : Concept stack production-grade
- **§ 6** — Surface 2 : Multi-turn state machine + convergence detection
- **§ 7** — Surface 3 : Context window management + summarization
- **§ 8** — Surface 4 : GDPR Art. 22 compliance (= consent + opt-out + audit)
- **§ 9** — Surface 5 : Human review workflow (= 5 escalation triggers)
- **§ 10** — Surface 6 : Audit chain per-session (= HMAC chain Mode-3)
- **§ 11** — Surface 7 : Fallback vers Mode-2 graceful
- **§ 12** — Surface 8 : Session state encryption at rest
- **§ 13** — Surface 9 : Session isolation tenant-scoped
- **§ 14** — Surface 10 : Cost amortization per-turn
- **§ 15** — SLO opposables Φ-D-MODE3 (= 18 SLOs YAML-parseable)
- **§ 16** — Matrice complète des artefacts Φ-D-MODE3 (= 42 entrées)
- **§ 17** — Plan d'implémentation D70 → D74 chirurgical
- **§ 18** — Risques identifiés & mitigations (24 risques)
- **§ 19** — Décisions de conception clés (22)
- **§ 20** — Anti-régression Mode-3 (= 16 pin tests)
- **§ 21** — Critères d'acceptation (DoD à 100 %)
- **§ 22** — Hand-off & dépendances downstream
- **§ 23** — Annexes (A → J)

---

## § 0. Méta, conventions, glossaire chirurgical

### 0.1 Doctrine Φ-D-MODE3

| Doctrine | Énoncé chirurgical |
|---|---|
| **D-MODE3-1 — Convergence opposable** | Sur 10 catalogs × 100 sessions multi-turn, ≥ 90 % convergent en ≤ 5 turns (= cosine_sim entre anchor_N et anchor_N+1 ≥ 0.95). Sinon : fallback Mode-2 + alerte. |
| **D-MODE3-2 — GDPR Art. 22 strict** | Avant la première anchor refinement, le session manager exige un consentement opt-in explicite tracé dans `session_consent_log` table. Toute requête sans consent → 451 Unavailable For Legal Reasons. Opt-out invalide la session + purge state ≤ 30 s. |
| **D-MODE3-3 — Cost per session hard cap** | Chaque session a un cap configurable (default $0.10). Hard stop à 100 % → fallback Mode-2 + alert. Cap inclut tous les turns + summarizations + human review LLM calls. |
| **D-MODE3-4 — Audit HMAC chain per-session** | Chaque turn produit une `Mode3Attestation` enrichie de : `session_id, turn_index, consent_hash, parent_anchor_hash, refined_anchor_hash, convergence_score, cost_usd_micros, human_review_status`. Chained per-session, parent-chained to D14-bis. |
| **D-MODE3-5 — Fallback graceful** | 3 trigger reasons : `convergence_failed` (= 5+ turns sans converger), `consent_revoked` (= user opt-out mid-session), `cost_exhausted` (= session cap atteint). Fallback dans < 3 s, sans interruption user-facing. |
| **D-MODE3-6 — Session isolation tenant-scoped** | Session state never crosses tenant boundary. Cache key prefix `mode3:v1:tenant:<tid>:session:<sid>`. Adversarial test enforces. |
| **D-MODE3-7 — Encryption at rest** | Session state (= anchor history + conversation transcript) chiffré AES-256-GCM avec per-tenant key. Decryption only on-demand pour le worker tenant. |
| **D-MODE3-8 — Convergence cosine** | Convergence détectée quand `cosine_sim(anchor_N, anchor_N+1) ≥ 0.95` pendant 2 turns consécutifs. Documente l'invariant numérique. |
| **D-MODE3-9 — Human review workflow** | 5 escalation triggers : `low_confidence`, `convergence_oscillating`, `high_cost_session`, `gdpr_sensitive_class`, `tenant_policy`. Each escalation produces a reviewable artifact + p99 review latency ≤ 5 min. |
| **D-MODE3-10 — Max turns enforced** | Hard cap K=10 turns per session. Beyond K → fallback Mode-2 + alert. Doctrine prevents runaway cost. |

### 0.2 Glossaire chirurgical

| Terme | Définition |
|---|---|
| **Session** | Conversation multi-turn entre un user et le system. Identifiée par `session_id` (UUID v4). Live max 24h ; persisted encrypted. |
| **Turn** | Une paire (user_input, system_anchor_refinement). Indexé `turn_index` ∈ [0, K-1] avec K=10 max. |
| **Anchor refinement** | Update de l'anchor cone basé sur les turns accumulés. Génère un nouveau anchor vector via LLM call enrichi du context. |
| **Convergence** | État où `cosine_sim(anchor_N, anchor_N+1) ≥ 0.95` pendant 2 turns. La session arrête de raffiner. |
| **Concept stack** | Pile LIFO de concepts extraits des turns. Chaque concept = `(noun_phrase, confidence, source_turn)`. Max 50 concepts. |
| **Context window** | Liste des K derniers turns + résumé des plus anciens. Bornée par token budget (= 8k tokens default). |
| **Summarization** | Compression LLM des turns 0..N-3 en une représentation textuelle compacte. Déclenchée quand token budget dépassé. |
| **Consent log** | Table `session_consent_log` qui enregistre opt-in/opt-out events avec timestamp + IP + user_agent. |
| **Human review** | Escalation à un opérateur humain via web UI. SLA réponse 5 min p99. |
| **Mode3Attestation** | Record HMAC-chained par turn. Extension de Mode2Attestation avec session-specific fields. |
| **Convergence oscillation** | Pattern dangereux : anchor flip-flopping entre 2 valeurs sans converger. Trigger human review. |
| **Session expiry** | TTL de 24h. Au-delà, le session state est purgé (= GDPR-compliant). |
| **Replay** | Re-exécution d'une session enregistrée pour reproduire les verdicts (= audit + debug). |

### 0.3 Conventions de référence

* SLO : `SLO-MODE3-<KEY>` (= 18 entries dans `bench_baselines/v0.6.0/mode3_slos.yaml`).
* Instrument Prometheus : `efe_mode3_<noun>_<unit>`.
* Artefact tracking : `Φ-D-MODE3.A<N>`.
* Workflow CI : `mode3-contract.yml`, `mode3-bench.yml`, `mode3-audit-replay.yml`, `mode3-human-review-drill.yml`.

---

## § 1. Mission Φ-D-MODE3 & contrat opposable

### 1.1 Mission

> Élever Mode-3 (= multi-turn conversation-aware anchor evolution) du
> statut **scaffold M22** au statut **production-grade contractuellement
> opposable et GDPR-compliant**.
>
> Concrètement :
>
> 1. Sur 10 catalogs × 100 sessions, **≥ 90 %** convergent en ≤ 5 turns.
> 2. Cost per session p99 ≤ $0.10 (= 5× Mode-2 single anchor).
> 3. Latency per turn p99 ≤ 35 s (= 1.2× Mode-2 anchor latency).
> 4. GDPR Art. 22 : 100 % opt-in tracé + opt-out fonctionnel ≤ 30 s.
> 5. Human review escalation : p99 latency ≤ 5 min.
> 6. Audit HMAC chain per-session : integrity = 100 %.
> 7. Fallback Mode-2 : 3 trigger reasons opérationnels + p99 ≤ 3 s.
> 8. Session isolation : 0 cross-tenant + 0 cross-session leak.
> 9. Encryption at rest : AES-256-GCM per-tenant.
> 10. Replay deterministic : 100 % sessions replayable bit-identique.

### 1.2 Contrat opposable

| Promesse | SLO | Mesure | Gating |
|---|---|---|---|
| Convergence | **SLO-MODE3-CONVERGENCE-RATE** ≥ 90 % | bench multi-turn | nightly + tag |
| Recall vs Mode-2 | **SLO-MODE3-RECALL-DELTA-VS-M2** ≥ +5 % | recall bench | nightly + tag |
| Cost per session | **SLO-MODE3-COST-PER-SESSION-P99** ≤ $0.10 | cost_governor extension | nightly |
| Latency per turn | **SLO-MODE3-LATENCY-PER-TURN-P99** ≤ 35 s | turn latency histogram | nightly |
| Human review latency | **SLO-MODE3-HUMAN-REVIEW-LATENCY-P99** ≤ 5 min | escalation tracker | nightly |
| GDPR opt-in coverage | **SLO-MODE3-GDPR-OPT-IN-COVERAGE** = 100 % | consent log replay | PR-gate + tag |
| Audit chain integrity | **SLO-MODE3-AUDIT-CHAIN-INTEGRITY** = 100 % | per-session replay | nightly |
| Session isolation | **SLO-MODE3-SESSION-ISOLATION** PASS | adversarial test | PR-gate |
| Fallback functional | **SLO-MODE3-FALLBACK-WORKS** = 3/3 | injection test | PR-gate |
| Context window | **SLO-MODE3-CONTEXT-WINDOW-RESPECT** = 100 % | token counter | PR-gate |
| Session expiry | **SLO-MODE3-SESSION-EXPIRY** ≤ 24h | TTL audit | nightly |
| Encryption at rest | **SLO-MODE3-ENCRYPTION-AT-REST** PASS | adversarial decrypt | PR-gate |
| Prompt versioning | **SLO-MODE3-PROMPT-VERSIONING** = 100 % | prompt audit | PR-gate |
| Max turns | **SLO-MODE3-MAX-TURNS-ENFORCED** PASS | enforcement test | PR-gate |
| Cost amortization | **SLO-MODE3-COST-PER-TURN-DECREASING** PASS | bench trend | nightly |
| Convergence cosine | **SLO-MODE3-CONVERGENCE-COSINE** PASS | cosine threshold ≥ 0.95 | PR-gate |
| Cache hit rate | **SLO-MODE3-CACHE-HIT-RATE** ≥ 60 % | Redis stats | nightly |
| Replay | **SLO-MODE3-REPLAY** PASS | deterministic re-run | nightly |

### 1.3 Anti-mission (ce que Φ-D-MODE3 **ne fait pas**)

* Ne livre pas un **modèle conversationnel custom** (= utilise les LLM Mode-2).
* Ne livre pas un **chat UI** (= scope EFE web séparé).
* Ne change pas le format `Mode2Attestation` (= extension only, backward-compatible).
* Ne livre pas un **active learning loop** (= scope M24).
* Ne livre pas un **multi-modal multi-turn** (= scope M30+).
* Ne livre pas un **session sharing cross-tenant** (= bypasse GDPR Art. 22).
* Ne livre pas une **machine d'état >10 turns** (= cap dur K=10).

---

## § 2. Position vs archi M22 D65-D69 (= gap analysis)

### 2.1 Ce que l'archi M22 couvre déjà (2 412 lignes)

* Phase A (D65) — Concept stack + multi-turn loop scaffolding.
* Phase B (D66) — GDPR Art. 22 compliance baseline.
* Phase C (D67) — Mode-3 backend RPC + persistence.
* Phase D (D68) — Human review workflow baseline.
* Phase E (D69) — mcp-server tools + audit pass M22.
* 80 pin tests baseline.

### 2.2 Ce que Φ-D-MODE3 ajoute (= gaps fermés)

| # | Gap | Étage | Sévérité | Φ-D-MODE3 artefact |
|---|---|---|---|---|
| G1 | Pas de SLO YAML opposable | SLO | **CRIT** | `mode3_slos.yaml` (18 entries) |
| G2 | Pas de convergence bench formel | Tests | **CRIT** | `eval/scripts/mode3_convergence_bench.py` |
| G3 | Pas de recall bench Mode-3 vs Mode-2 | Tests | **CRIT** | `eval/scripts/mode3_recall_bench.py` |
| G4 | Pas de GDPR consent replay tool | Compliance | **CRIT** | `scripts/mode3/replay_consent_log.py` |
| G5 | Pas de session isolation test | Sécurité | **CRIT** | `tests/mode3_session_isolation.py` |
| G6 | Pas de audit replay per-session | Audit | HIGH | `scripts/mode3/replay_session_chain.py` |
| G7 | Pas de fallback injection test | Tests | HIGH | `tests/mode3_fallback_injection.py` |
| G8 | Pas de encryption test adversarial | Sécurité | HIGH | `tests/mode3_encryption_adversarial.py` |
| G9 | Pas de context window enforcement | Tests | HIGH | `tests/mode3_context_window.py` |
| G10 | Pas de cost amortization tracking | Mesure | HIGH | `scripts/mode3/track_cost_amortization.py` |
| G11 | Pas de human review drill | Process | HIGH | `.github/workflows/mode3-human-review-drill.yml` |
| G12 | Pas de session expiry verifier | Tests | MED | `scripts/mode3/verify_session_expiry.py` |
| G13 | Pas de convergence oscillation detector | Tests | MED | `eval/scripts/mode3_oscillation_detector.py` |
| G14 | Pas de max-turns enforcement test | Tests | HIGH | `tests/mode3_max_turns_enforced.py` |
| G15 | Pas de Grafana dashboard Mode-3 | Obs | MED | `deploy/observability/grafana/efe-mode3.json` |
| G16 | Pas de 5 runbooks Mode-3 | Ops | MED | `docs/runbook/mode3_*.md` (× 5) |
| G17 | Pas de pin tests Mode-3 | Process | **CRIT** | `scripts/tests/test_mode3_completion.py` |
| G18 | Pas de plan day-by-day D70-D74 | Process | MED | § 17 |
| G19 | Pas de Mode3Attestation extension | Code | HIGH | `crates/efe-server/src/audit/mode3.rs` |
| G20 | Pas de OTel cross-Mode-3 instruments | Obs | HIGH | `crates/efe-telemetry/src/metrics/mode3_instruments.rs` |
| G21 | Pas de "tests le test" meta validation | Process | HIGH | dans pin tests |
| G22 | Pas de prompt registry extension session-aware | Code | MED | `crates/efe-prompts/src/session.rs` |
| G23 | Pas de session state schema strict | Schema | HIGH | `schemas/mode3_session_state.json` |
| G24 | Pas de adversarial multi-turn corpus | Tests | HIGH | `eval/mode3/adversarial_sessions/` (40) |

### 2.3 Doctrine de gap-closing

Le pattern Φ-X Completion (CUDA, FFI, SDK, Mode-2) éradique le défaut "silently broken SLO gating" via :

* SLO YAML field → parser branche → pin test.
* Production caller documenté pour chaque instrument.
* Script référencé = script existant (D-FFI-3).
* Workflow honest skip_reason (= jamais hardcoded PASS).

Φ-D-MODE3 ajoute une **dimension GDPR** : aucune session ne peut être créée sans consentement opt-in tracé, et toute opt-out doit purger le state ≤ 30 s.

---

## § 3. État AS-IS chirurgical

### 3.1 Inventaire `crates/` Mode-3 existant

```
# Mode-3 components are scattered across :
crates/
├── efe-llm-router/         # Reused from Mode-2
├── efe-prompts/            # Reused ; needs session-aware extension
├── efe-server/
│   └── src/
│       ├── mode3/
│       │   ├── mod.rs              # Multi-turn state machine
│       │   ├── concept_stack.rs    # Concept extraction
│       │   ├── convergence.rs      # Convergence detection
│       │   ├── session.rs          # Session lifecycle
│       │   └── human_review.rs     # Escalation workflow
│       └── audit/
│           └── mode2.rs            # Base ; Mode-3 extension TBD
```

### 3.2 Statut par dimension

| Dimension | M22 D65-D69 | Post-Φ-D-MODE3 |
|---|---|---|
| Concept stack scaffolding | ✅ | ✅ + production hardening |
| Multi-turn loop | ✅ basic | ✅ + state machine pinned |
| Convergence detection | ⚠️ heuristic | ✅ + cosine ≥ 0.95 strict |
| Context window | ⚠️ basic | ✅ + token-budget enforced |
| GDPR Art. 22 | ✅ baseline | ✅ + consent replay tool |
| Human review workflow | ✅ basic | ✅ + 5 escalation triggers + SLA |
| Audit chain per-session | ❌ | ✅ + Mode3Attestation + replay |
| Fallback Mode-2 | ❌ | ✅ + 3 trigger reasons |
| Encryption at rest | ❌ | ✅ AES-256-GCM per-tenant |
| Session isolation | ⚠️ implicit | ✅ + adversarial test |
| Cost amortization | ❌ | ✅ tracking + SLO |
| Convergence oscillation | ❌ | ✅ detector |
| Max turns enforced | ⚠️ soft | ✅ hard K=10 |
| Grafana dashboard | ❌ | ✅ |
| Pin tests Φ-D-MODE3 | ❌ | ✅ 16 tests |
| Runbooks | ❌ | ✅ 5 docs |

### 3.3 Mesure quantitative AS-IS

| Métrique | M22 baseline | Cible Φ-D-MODE3 |
|---|---|---|
| Tests Rust unit | ~ 80 | ~ 130 |
| Tests Python eval | 0 | ~ 60 |
| Tests multi-turn corpus | 0 | 10 × 100 = 1 000 sessions |
| Workflows CI | 0 | 4 (`contract`, `bench`, `audit-replay`, `human-review-drill`) |
| Runbooks | 0 | 5 |
| SLO YAML entries | 0 | 18 |
| Instruments Prom | ~ 6 | ~ 20 |
| Adversarial sessions | 0 | 40 |

---

## § 4. Décomposition du pipeline Mode-3 multi-turn

### 4.1 Étages canoniques (9 étages)

```text
┌─────────────────────────────────────────────────────────────────────┐
│  session_classify(session_id, turn_text) → Verdict                  │
│                                                                     │
│  Mode-3 path (= multi-turn anchor evolution)                        │
│  ────────────────────────────────────────────                       │
│  Stage 1 : session lookup + tenant resolution           ≈ 2 ms      │
│  Stage 2 : consent check (= GDPR Art. 22)               ≈ 1 ms      │
│  Stage 3 : context window assembly                      ≈ 5 ms      │
│  Stage 4 : [if budget exceeded] summarization LLM call ≈ 10 s p99  │
│  Stage 5 : concept stack update                         ≈ 2 ms      │
│  Stage 6 : anchor refinement LLM call (= Mode-2 enriched) ≈ 25 s p99│
│  Stage 7 : convergence detection (= cosine_sim)         ≈ 1 ms      │
│  Stage 8 : audit attestation (= HMAC chain extension)   ≈ 3 ms      │
│  Stage 9 : verdict materialization                      ≈ 50 ms     │
│                                                                     │
│  Total p99 per turn (cold) : ~ 35 s                                 │
│  Total p99 per turn (warm) : ~ 100 ms (= cache hit + no summarize)  │
│  Total p99 per session (5 turns) : ~ 60 s (= w/ amortization)      │
└─────────────────────────────────────────────────────────────────────┘
```

### 4.2 Étages observables (instruments Prometheus § 23.B)

| Stage | Instrument |
|---|---|
| Session lookup | `efe_mode3_session_lookup_seconds` |
| Consent check | `efe_mode3_consent_check_seconds` |
| Context assembly | `efe_mode3_context_assembly_seconds` |
| Summarization | `efe_mode3_summarization_seconds` |
| Concept stack update | `efe_mode3_concept_stack_seconds` |
| Anchor refinement | `efe_mode3_anchor_refinement_seconds` |
| Convergence detection | `efe_mode3_convergence_check_seconds` |
| Audit attestation | `efe_mode3_audit_attest_seconds` |
| Verdict materialize | `efe_mode3_verdict_materialize_seconds` |
| Total turn | `efe_mode3_turn_seconds` |
| Total session | `efe_mode3_session_seconds` |

---

## § 5. Surface 1 : Concept stack production-grade

### 5.1 Schema

```rust
pub struct ConceptStack {
    pub session_id: SessionId,
    pub concepts: BoundedVec<Concept, 50>,  // LIFO, max 50
    pub last_update_turn: u32,
}

pub struct Concept {
    pub noun_phrase: String,       // e.g. "refund policy"
    pub confidence: f32,            // 0.0..=1.0
    pub source_turn: u32,
    pub embedding: Vec<f32>,        // dim=384, for similarity matching
}
```

### 5.2 Update algorithm

```text
on_turn(text):
  1. Extract noun phrases via Mode-2 LLM call.
  2. For each new concept :
     a. Embed via bundle.encode(noun_phrase) → vec
     b. Compare cosine_sim vs existing concepts
     c. If sim ≥ 0.85 with existing → boost confidence, merge
     d. Else → push to stack (= LIFO)
  3. If stack > 50 → pop oldest concepts
  4. Persist to encrypted session state
```

### 5.3 Anchor refinement using concept stack

L'anchor refinement utilise les top-K concepts (K=5) du stack pour
enrichir le prompt :

```
You are an expert classifier. Given the user's conversation so far,
the active concepts are :
1. {{concept_1.noun_phrase}} (confidence {{concept_1.confidence}})
2. {{concept_2.noun_phrase}} ...

Generate anchors for the catalog labels {{labels}}, emphasizing the
active concepts.
```

---

## § 6. Surface 2 : Multi-turn state machine + convergence detection

### 6.1 State machine

```text
                    [opt-in granted]
       ┌────────────────────────────────────┐
       ▼                                    │
   ┌─────────┐                       ┌──────┴───────┐
   │  INIT   │ ──── first turn ───►  │ EVOLVING     │
   │         │                       │ (turns 1..N) │
   └─────────┘                       └──────┬───────┘
                                            │
                          ┌─────────────────┴────────────────┐
                          │                                  │
                  convergence ≥ 0.95                  turn_count = 10
                  for 2 turns                         OR cost cap
                          │                                  │
                          ▼                                  ▼
                  ┌──────────────┐                  ┌──────────────┐
                  │  CONVERGED   │                  │   FALLBACK   │
                  │  (stable)    │                  │   TO MODE-2  │
                  └──────────────┘                  └──────────────┘
                          │
                          │ user opt-out OR session expiry
                          ▼
                  ┌──────────────┐
                  │  TERMINATED  │
                  │  (purged)    │
                  └──────────────┘
```

### 6.2 Convergence detection algorithm

```rust
fn check_convergence(
    anchor_history: &[Vec<f32>],
    threshold: f32,
    window: usize,
) -> ConvergenceStatus {
    if anchor_history.len() < 2 {
        return ConvergenceStatus::TooEarly;
    }
    // Check last `window` consecutive turn pairs.
    let mut converged_count = 0;
    for w in anchor_history.windows(2).rev().take(window) {
        let sim = cosine_similarity(&w[0], &w[1]);
        if sim >= threshold {
            converged_count += 1;
        }
    }
    if converged_count >= window {
        ConvergenceStatus::Converged
    } else if anchor_history.len() >= 10 {
        ConvergenceStatus::FailedToConverge
    } else {
        ConvergenceStatus::StillEvolving
    }
}
```

Default : `threshold=0.95`, `window=2`.

### 6.3 Oscillation detection

Oscillation = pattern où anchor flip-flop entre 2 valeurs sans converger :

```text
turn N:   anchor_N    (cosine vs N-1 = 0.91)
turn N+1: anchor_N+1  (cosine vs N   = 0.82)
turn N+2: anchor_N+2  (cosine vs N+1 = 0.91, vs N = 0.93)
turn N+3: anchor_N+3  (cosine vs N+2 = 0.82, vs N+1 = 0.93)
```

Détecté quand `cosine_sim(anchor_N, anchor_N-2) > cosine_sim(anchor_N, anchor_N-1)` sur ≥ 3 turns consécutifs.

Trigger : human review escalation.

---

## § 7. Surface 3 : Context window management

### 7.1 Token budget

Default budget : 8 000 tokens (= compatible with all 8 providers' context).

```text
context = system_prompt (~ 200 tokens)
        + active_concepts (~ 500 tokens for K=5)
        + summary_of_old_turns (~ 1500 tokens max)
        + recent_turns (~ 6000 tokens for 3-5 turns)
        Total ≤ 8000 tokens
```

### 7.2 Summarization trigger

Quand `total_tokens > 80 %` du budget :
1. Identify oldest non-summarized turns (= turns 0..N-3).
2. LLM call : "summarize these turns preserving key facts".
3. Replace turns 0..N-3 in context with summary.
4. Cost : ~ $0.002 per summarization on premium provider.

### 7.3 Hard-truncation fallback

Si summarization échoue après 2 retries :
1. Truncate oldest turns brutally.
2. Increment `efe_mode3_context_truncation_total`.
3. Log WARN.

---

## § 8. Surface 4 : GDPR Art. 22 compliance

### 8.1 Consent log schema

```sql
CREATE TABLE session_consent_log (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    session_id UUID NOT NULL,
    event_type TEXT NOT NULL,  -- 'opt_in', 'opt_out', 'expiry', 'admin_revoke'
    consent_version TEXT NOT NULL,  -- e.g. 'v2.1'
    timestamp_unix_micros BIGINT NOT NULL,
    ip_address INET,
    user_agent TEXT,
    consent_hash BYTEA NOT NULL,  -- SHA-256 of consent text + version
    notes TEXT
);

CREATE INDEX idx_consent_session ON session_consent_log(session_id);
CREATE INDEX idx_consent_tenant_time ON session_consent_log(tenant_id, timestamp_unix_micros);
```

### 8.2 Opt-in flow

```text
POST /mode3/session/create
  ↓
1. Check tenant policy : does this tenant require Art. 22 consent ?
   If yes :
     ↓
2. Verify request has consent_token (= HMAC-signed by frontend after user click).
   ↓
3. Decode consent_token : { user_id, consent_version, timestamp }
   ↓
4. Verify consent_version is current (= rolled out in config).
   ↓
5. Insert row in session_consent_log (event_type='opt_in').
   ↓
6. Create session with consent_hash linked.
   ↓
7. Return session_id.
```

### 8.3 Opt-out flow

```text
DELETE /mode3/session/{session_id}
  ↓
1. Authenticate request (= user OR admin).
   ↓
2. Insert row in session_consent_log (event_type='opt_out').
   ↓
3. Mark session terminated in state store.
   ↓
4. Schedule async purge : delete session state + audit trail.
   ↓
5. Purge must complete ≤ 30 s (= SLO-MODE3-OPT-OUT-LATENCY).
   ↓
6. Return 204 No Content.
```

### 8.4 Replay tool

`scripts/mode3/replay_consent_log.py` :
* Verify every session has corresponding `opt_in` event.
* Verify `consent_hash` integrity (= rehash consent_version + verify).
* Detect orphan sessions (= sessions without consent).
* Emit SLO compliance for `SLO-MODE3-GDPR-OPT-IN-COVERAGE`.

---

## § 9. Surface 5 : Human review workflow

### 9.1 5 escalation triggers

| Trigger | Detection | Action |
|---|---|---|
| `low_confidence` | Turn-N anchor confidence < 0.5 | Auto-escalate to review queue |
| `convergence_oscillating` | Oscillation detected (cf. § 6.3) | Auto-escalate |
| `high_cost_session` | Cumulative cost > 80% of cap | Escalate + notify owner |
| `gdpr_sensitive_class` | LLM detects PII / sensitive content | Auto-escalate + delay verdict |
| `tenant_policy` | Tenant config flags this catalog for review | Per-policy escalation |

### 9.2 Review queue schema

```sql
CREATE TABLE mode3_review_queue (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    session_id UUID NOT NULL,
    turn_index INT NOT NULL,
    escalation_trigger TEXT NOT NULL,
    escalation_timestamp BIGINT NOT NULL,
    review_status TEXT DEFAULT 'pending',  -- 'pending', 'in_review', 'approved', 'rejected'
    reviewer_id TEXT,
    review_timestamp BIGINT,
    review_notes TEXT,
    proposed_verdict JSONB
);
```

### 9.3 SLA enforcement

* `pending → in_review` : p99 ≤ 30 s (= auto-assignment).
* `in_review → decided` : p99 ≤ 5 min (= human-touch).
* Total escalation → decided : p99 ≤ 5 min 30 s.

Mesuré via `efe_mode3_review_latency_seconds{stage}`.

### 9.4 Drill workflow

`mode3-human-review-drill.yml` simule un escalation chaque jour à 09:00 UTC :
1. Inject synthetic session with low_confidence trigger.
2. Verify auto-assignment ≤ 30 s.
3. Verify review UI receives the item.
4. Auto-approve via test reviewer account.
5. Verify total latency ≤ 5 min 30 s.

Failure → PagerDuty SEV2.

---

## § 10. Surface 6 : Audit chain per-session

### 10.1 Mode3Attestation record

```rust
pub struct Mode3Attestation {
    // Base fields from Mode2Attestation (= inherited).
    pub schema_version: u32,
    pub tenant_id: TenantId,
    pub request_id: RequestId,
    pub timestamp_unix_micros: u64,
    pub prev_hash: [u8; 32],
    pub hmac_sha256: [u8; 32],

    // Mode-3 specific fields.
    pub session_id: SessionId,
    pub turn_index: u32,
    pub consent_hash: [u8; 32],
    pub parent_anchor_hash: [u8; 32],   // anchor at turn N-1
    pub refined_anchor_hash: [u8; 32],   // anchor at turn N
    pub convergence_score: f32,          // cosine_sim(N-1, N)
    pub convergence_status: ConvergenceStatus,
    pub cost_usd_micros: u64,
    pub turn_latency_ms: u32,
    pub human_review_status: HumanReviewStatus,  // none / pending / in_review / decided
    pub fallback_reason: Option<FallbackReason>,
    pub context_tokens: u32,
    pub summarization_applied: bool,
}

#[repr(u8)]
pub enum ConvergenceStatus {
    TooEarly = 0,
    StillEvolving = 1,
    Converged = 2,
    Oscillating = 3,
    FailedToConverge = 4,
}

#[repr(u8)]
pub enum HumanReviewStatus {
    None = 0,
    Pending = 1,
    InReview = 2,
    Approved = 3,
    Rejected = 4,
}
```

### 10.2 HMAC chain protocol

Identique à Mode-2 mais avec **per-session sub-chain** :

```text
session_chain = [att_turn_0, att_turn_1, ..., att_turn_N]
att_turn_K.prev_hash = att_turn_K-1.hmac_sha256  (within session)
att_turn_0.prev_hash = parent_session_chain.head  (cross-session linkage)
```

### 10.3 Replay tool

`scripts/mode3/replay_session_chain.py` :
* Pour chaque session, replay le HMAC chain et verify integrity.
* Verify session-level convergence_score progression makes sense.
* Detect tampering (= mismatched hashes).
* Emit SLO compliance for `SLO-MODE3-AUDIT-CHAIN-INTEGRITY`.

---

## § 11. Surface 7 : Fallback vers Mode-2 graceful

### 11.1 3 trigger reasons

| Trigger | Detection | Action |
|---|---|---|
| `convergence_failed` | 10 turns sans converger | Mode-2 single-anchor on last turn input |
| `consent_revoked` | User opt-out mid-session | Terminate session + Mode-2 next request |
| `cost_exhausted` | Session cost > cap | Mode-2 single-anchor + notify owner |

### 11.2 Fallback latency budget

P99 ≤ 3 s entre détection trigger et retour verdict Mode-2.

### 11.3 Continuity guarantee

Le verdict Mode-2 retourné suite à fallback est **immédiatement utilisable** (= pas de re-prompt user). Le session_id est marqué terminated.

---

## § 12. Surface 8 : Encryption at rest

### 12.1 Per-tenant key management

```rust
pub struct TenantSessionKey {
    pub tenant_id: TenantId,
    pub key_version: u32,
    pub aes_256_gcm_key: [u8; 32],  // sealed via KMS
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,  // 90-day rotation
}
```

### 12.2 Encryption format

```text
session_state_record = {
    session_id: UUID,
    tenant_id: TenantId,
    encrypted_payload: AES-256-GCM(plaintext = MessagePack(SessionState), key = tenant_key),
    nonce: [u8; 12],
    auth_tag: [u8; 16],
    key_version: u32,  // for rotation
}
```

### 12.3 Adversarial test

`tests/mode3_encryption_adversarial.py` :
1. Create session for tenant A.
2. Attempt to read session_state record using tenant B's key → must fail with AEAD auth tag mismatch.
3. Attempt to read with WRONG tenant_A key version → must fail.
4. Verify plaintext never appears in DB / WAL / backup snapshots.

### 12.4 Key rotation

* Every 90 days.
* Old key kept for 180 days for replay.
* Re-encryption job runs in background ; new sessions use new key immediately.

---

## § 13. Surface 9 : Session isolation tenant-scoped

### 13.1 Namespace separation

* Session state cache key : `mode3:v1:tenant:<tid>:session:<sid>`
* Audit chain : per-session sub-chain (linked to per-tenant master chain)
* Review queue : tenant_id NOT NULL constraint
* Consent log : tenant_id NOT NULL
* Metrics : `efe_mode3_*{tenant}` labels

### 13.2 Adversarial test

```python
def test_tenant_a_cannot_access_tenant_b_session():
    # Tenant A creates session.
    session_a = create_session(tenant="tenant-a", consent=opt_in_a)
    # Tenant B attempts to query session A.
    response = api_call(tenant="tenant-b", session_id=session_a.id)
    assert response.status_code == 404  # MUST not leak existence
```

---

## § 14. Surface 10 : Cost amortization per-turn

### 14.1 Theoretical model

```text
turn_1_cost ≈ Mode-2 single anchor cost ($0.015)
turn_N_cost (N > 1) :
  - context lookup cost (~ $0.002)
  - anchor refinement (= warm cache benefit, ~ 30 % cache hit) ≈ $0.010
  - convergence check + audit ≈ $0.001
Total turn_N ≈ $0.013

Session 5 turns cost ≈ $0.015 + 4 × $0.013 = $0.067
Session 10 turns cost ≈ $0.015 + 9 × $0.013 = $0.132
```

Hard cap default : $0.10 per session → fallback after turn ~ 7-8.

### 14.2 SLO `SLO-MODE3-COST-PER-TURN-DECREASING`

```text
Pour 100 sessions sampled :
  mean(turn_N cost) ≤ mean(turn_1 cost) - 10 % pour N ≥ 3
```

Si non respecté → soit summarization inefficient, soit cache cold, soit catalog problem.

---

## § 15. SLO opposables Φ-D-MODE3 (= 18 SLOs)

### 15.1 Fichier `bench_baselines/v0.6.0/mode3_slos.yaml`

```yaml
version: v0.6.0
domain: mode3
host_fingerprint_baseline: "PLACEHOLDER_compute_at_freeze"
slos:
  - id: SLO-MODE3-CONVERGENCE-RATE
    description: "≥ 90 % of sessions converge in ≤ 5 turns"
    comparator: gte
    threshold_ratio: 0.90
    source_bench: convergence_bench

  - id: SLO-MODE3-RECALL-DELTA-VS-M2
    description: "Mode-3 recall@1 beats Mode-2 by ≥ +5 % aggregate"
    comparator: gte
    threshold_ratio: 0.05
    source_bench: recall_bench

  - id: SLO-MODE3-COST-PER-SESSION-P99
    description: "p99 cost per session ≤ $0.10"
    comparator: lte
    threshold_ratio: 0.10
    source_bench: cost_tracking

  - id: SLO-MODE3-LATENCY-PER-TURN-P99
    description: "p99 latency per turn ≤ 35 seconds"
    comparator: lte
    threshold_seconds: 35.0
    source_bench: latency_bench

  - id: SLO-MODE3-HUMAN-REVIEW-LATENCY-P99
    description: "p99 escalation → decided latency ≤ 5 min 30 s"
    comparator: lte
    threshold_seconds: 330.0
    source_bench: review_tracker

  - id: SLO-MODE3-GDPR-OPT-IN-COVERAGE
    description: "100 % of sessions have opt-in tracked"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: consent_replay

  - id: SLO-MODE3-AUDIT-CHAIN-INTEGRITY
    description: "100 % of session HMAC chains validate"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: chain_replay

  - id: SLO-MODE3-SESSION-ISOLATION
    description: "Adversarial cross-session / cross-tenant test : 0 leak"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: isolation_test

  - id: SLO-MODE3-FALLBACK-WORKS
    description: "All 3 fallback trigger reasons functional"
    comparator: gte
    threshold_pass_count: 3
    expected_count: 3
    source_bench: fallback_injection

  - id: SLO-MODE3-CONTEXT-WINDOW-RESPECT
    description: "100 % of LLM calls respect token budget"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: context_window_test

  - id: SLO-MODE3-SESSION-EXPIRY
    description: "Session TTL ≤ 24h enforced"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: expiry_verifier

  - id: SLO-MODE3-ENCRYPTION-AT-REST
    description: "Adversarial decrypt + cross-tenant test : all fail"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: encryption_adversarial

  - id: SLO-MODE3-PROMPT-VERSIONING
    description: "100 % of prompts used in Mode-3 are semver-pinned"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: prompt_audit

  - id: SLO-MODE3-MAX-TURNS-ENFORCED
    description: "Hard cap K=10 turns enforced on injection test"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: max_turns_test

  - id: SLO-MODE3-COST-PER-TURN-DECREASING
    description: "Cost amortization : mean(turn_N) ≤ mean(turn_1) - 10 %"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: cost_amortization

  - id: SLO-MODE3-CONVERGENCE-COSINE
    description: "Convergence detected at cosine_sim ≥ 0.95 for 2 turns"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: convergence_cosine_test

  - id: SLO-MODE3-CACHE-HIT-RATE
    description: "Cache hit rate on warm workload ≥ 60 %"
    comparator: gte
    threshold_ratio: 0.60
    source_bench: cache_stats

  - id: SLO-MODE3-REPLAY
    description: "100 % of sessions replayable bit-identique"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: deterministic_replay
```

### 15.2 Matrice de gating

| SLO | PR-gate | Nightly | Tag-gate |
|---|---|---|---|
| CONVERGENCE-RATE | — | ✅ | ✅ |
| RECALL-DELTA-VS-M2 | — | ✅ | ✅ |
| COST-PER-SESSION-P99 | — | ✅ | ✅ |
| LATENCY-PER-TURN-P99 | — | ✅ | ✅ |
| HUMAN-REVIEW-LATENCY | — | ✅ | — |
| GDPR-OPT-IN-COVERAGE | ✅ | ✅ | ✅ |
| AUDIT-CHAIN-INTEGRITY | — | ✅ | ✅ |
| SESSION-ISOLATION | ✅ | ✅ | ✅ |
| FALLBACK-WORKS | ✅ | — | ✅ |
| CONTEXT-WINDOW-RESPECT | ✅ | — | — |
| SESSION-EXPIRY | — | ✅ | — |
| ENCRYPTION-AT-REST | ✅ | — | ✅ |
| PROMPT-VERSIONING | ✅ | — | ✅ |
| MAX-TURNS-ENFORCED | ✅ | — | ✅ |
| COST-PER-TURN-DECREASING | — | ✅ | — |
| CONVERGENCE-COSINE | ✅ | — | — |
| CACHE-HIT-RATE | — | ✅ | — |
| REPLAY | — | ✅ | ✅ |

---

## § 16. Matrice complète des artefacts Φ-D-MODE3 (= 42)

| # | Artefact | Chemin | Type | Sprint | Gap |
|---|---|---|---|---|---|
| 01 | SLO YAML | `bench_baselines/v0.6.0/mode3_slos.yaml` | config | D70 | G1 |
| 02 | Doc archi | `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md` | doc | D70 | — |
| 03 | Mode3Attestation Rust type | `crates/efe-server/src/audit/mode3.rs` | code | D70 | G19 |
| 04 | Session state JSON schema | `schemas/mode3_session_state.json` | schema | D70 | G23 |
| 05 | Prompt registry session-aware | `crates/efe-prompts/src/session.rs` | code | D70 | G22 |
| 06 | Encryption at rest module | `crates/efe-server/src/mode3/encryption.rs` | code | D70 | — |
| 07 | Mode-3 OTel instruments | `crates/efe-telemetry/src/metrics/mode3_instruments.rs` | code | D70 | G20 |
| 08 | Convergence detection algo | `crates/efe-server/src/mode3/convergence.rs` | code | D70 | — |
| 09 | 10 multi-turn bench catalogs | `eval/mode3/catalogs/` | data | D71 | — |
| 10 | 40 adversarial sessions | `eval/mode3/adversarial_sessions/` | data | D71 | G24 |
| 11 | Convergence bench | `eval/scripts/mode3_convergence_bench.py` | tool | D71 | G2 |
| 12 | Recall bench Mode-3 vs Mode-2 | `eval/scripts/mode3_recall_bench.py` | tool | D71 | G3 |
| 13 | Cost amortization tracker | `scripts/mode3/track_cost_amortization.py` | tool | D71 | G10 |
| 14 | Consent log replay | `scripts/mode3/replay_consent_log.py` | tool | D71 | G4 |
| 15 | Session chain replay | `scripts/mode3/replay_session_chain.py` | tool | D71 | G6 |
| 16 | Session expiry verifier | `scripts/mode3/verify_session_expiry.py` | tool | D72 | G12 |
| 17 | Oscillation detector | `eval/scripts/mode3_oscillation_detector.py` | tool | D72 | G13 |
| 18 | Convergence cosine test | `eval/scripts/mode3_convergence_cosine_test.py` | tool | D72 | — |
| 19 | Session isolation test | `tests/mode3_session_isolation.py` | test | D72 | G5 |
| 20 | Fallback injection test | `tests/mode3_fallback_injection.py` | test | D72 | G7 |
| 21 | Encryption adversarial test | `tests/mode3_encryption_adversarial.py` | test | D72 | G8 |
| 22 | Context window test | `tests/mode3_context_window.py` | test | D72 | G9 |
| 23 | Max turns enforced test | `tests/mode3_max_turns_enforced.py` | test | D72 | G14 |
| 24 | Context summarization test | `tests/mode3_context_summarization.py` | test | D72 | — |
| 25 | Concept stack test | `tests/mode3_concept_stack.py` | test | D72 | — |
| 26 | Human review escalation triggers | `eval/scripts/mode3_review_triggers_test.py` | tool | D73 | — |
| 27 | Human review SLA tracker | `scripts/mode3/track_review_sla.py` | tool | D73 | — |
| 28 | Human review drill script | `scripts/mode3/drill_review.sh` | tool | D73 | G11 |
| 29 | Deterministic session replay | `scripts/mode3/replay_session_state.py` | tool | D73 | — |
| 30 | Cache stats collector | `scripts/mode3/collect_cache_stats.py` | tool | D73 | — |
| 31 | Runbook : convergence failure | `docs/runbook/mode3_convergence_failure.md` | doc | D73 | G16 |
| 32 | Runbook : human review queue full | `docs/runbook/mode3_review_queue_full.md` | doc | D73 | G16 |
| 33 | Runbook : GDPR opt-out cascade | `docs/runbook/mode3_gdpr_opt_out.md` | doc | D73 | G16 |
| 34 | Runbook : encryption key rotation | `docs/runbook/mode3_key_rotation.md` | doc | D73 | G16 |
| 35 | Runbook : session expiry purge | `docs/runbook/mode3_session_purge.md` | doc | D73 | G16 |
| 36 | Convention doc : multi-turn prompts | `docs/mode3-multi-turn-prompt-conventions.md` | doc | D73 | — |
| 37 | Grafana dashboard | `deploy/observability/grafana/efe-mode3.json` | obs | D74 | G15 |
| 38 | Pin tests Φ-D-MODE3 | `scripts/tests/test_mode3_completion.py` | test | D74 | G17, G21 |
| 39 | Workflow contract | `.github/workflows/mode3-contract.yml` | ci | D74 | — |
| 40 | Workflow bench | `.github/workflows/mode3-bench.yml` | ci | D74 | — |
| 41 | Workflow audit replay | `.github/workflows/mode3-audit-replay.yml` | ci | D74 | — |
| 42 | Workflow human review drill | `.github/workflows/mode3-human-review-drill.yml` | ci | D74 | G11 |

---

## § 17. Plan d'implémentation D70 → D74 chirurgical

### 17.1 D70 — Foundations (jour 1)

| Heure | Tâche | Artefact |
|---|---|---|
| 09:00–10:30 | SLO YAML + doc archi | 01, 02 |
| 10:30–12:00 | Mode3Attestation + JSON schema | 03, 04 |
| 14:00–15:30 | Session prompt registry + encryption module | 05, 06 |
| 15:30–17:30 | OTel instruments + convergence algo | 07, 08 |

### 17.2 D71 — Bench harness + audit tools (jour 2)

| Heure | Tâche | Artefact |
|---|---|---|
| 09:00–11:00 | 10 multi-turn catalogs + 40 adversarial sessions | 09, 10 |
| 11:00–13:00 | Convergence bench + recall bench | 11, 12 |
| 14:00–15:30 | Cost amortization + consent replay | 13, 14 |
| 15:30–17:30 | Session chain replay | 15 |

### 17.3 D72 — Tests + adversarials (jour 3)

| Heure | Tâche | Artefact |
|---|---|---|
| 09:00–10:30 | Session expiry + oscillation + cosine test | 16, 17, 18 |
| 10:30–13:00 | Isolation + fallback + encryption tests | 19, 20, 21 |
| 14:00–15:30 | Context window + max turns + summarization tests | 22, 23, 24 |
| 15:30–17:30 | Concept stack test | 25 |

### 17.4 D73 — Human review + runbooks (jour 4)

| Heure | Tâche | Artefact |
|---|---|---|
| 09:00–10:30 | Review triggers + SLA tracker | 26, 27 |
| 10:30–12:00 | Review drill + deterministic replay + cache stats | 28, 29, 30 |
| 14:00–17:00 | 5 runbooks | 31, 32, 33, 34, 35 |
| 17:00–17:30 | Convention doc multi-turn prompts | 36 |

### 17.5 D74 — Workflows + pin tests + go-live (jour 5)

| Heure | Tâche | Artefact |
|---|---|---|
| 09:00–10:30 | Grafana dashboard | 37 |
| 10:30–13:00 | Pin tests Φ-D-MODE3 (= 16 tests) | 38 |
| 14:00–15:30 | Workflows contract + bench | 39, 40 |
| 15:30–17:00 | Workflows audit-replay + human-review-drill | 41, 42 |
| 17:00–17:30 | Final audit + commit + go-live | — |

**Go/No-Go D74 EOD = Φ-D-MODE3 go-live**.

---

## § 18. Risques identifiés & mitigations (24 risques)

| # | Risque | Sévérité | Probabilité | Mitigation |
|---|---|---|---|---|
| R1 | LLM context window overflow | HIGH | MED | Token budget + summarization + fallback to truncation |
| R2 | Convergence oscillation infinie | HIGH | LOW | Oscillation detector + human review escalation |
| R3 | Cost spike (= verbose summarizations) | HIGH | MED | Per-session hard cap + soft alert at 80 % |
| R4 | GDPR consent token forgery | CRIT | LOW | HMAC-signed consent_token + frontend secret rotation |
| R5 | Session state encrypted leakage | CRIT | LOW | AES-256-GCM + per-tenant key + KMS sealed |
| R6 | Cross-session cache poisoning | HIGH | LOW | Per-session cache namespace + adversarial test |
| R7 | Human review queue overflow | HIGH | MED | Backpressure + dead-letter + scale review staff |
| R8 | Encryption key rotation breaks replay | MED | LOW | Keep old key 180 days + re-encryption job |
| R9 | Audit chain HMAC key leak | CRIT | LOW | Rotate every 90 days ; Vault-stored |
| R10 | Session expiry purge fails | HIGH | LOW | Retry + dead-letter + manual purge runbook |
| R11 | Convergence threshold too strict | MED | MED | A/B test threshold variations |
| R12 | Convergence threshold too loose | MED | MED | Calibrate via recall vs convergence trade-off |
| R13 | Prompt injection in turn text | HIGH | MED | Safety filter (= Mode-2 inherited) + adversarial sessions |
| R14 | Long-running session > 24h | LOW | LOW | Session expiry hard cap |
| R15 | Cost cap surprised user | MED | MED | Soft cap email alert at 80 % |
| R16 | Concept stack drift | MED | MED | Concept embedding similarity merge |
| R17 | Multi-tenant prompt drift | MED | LOW | Per-tenant prompt overrides documented |
| R18 | Replay tool produces different verdicts | HIGH | LOW | Pin test deterministic + fixed seed |
| R19 | LLM provider down mid-session | HIGH | MED | Mode-3 fallback chain = (current provider → fallback → Mode-2) |
| R20 | Database WAL contains plaintext session state | CRIT | LOW | Encrypt at write ; verify via grep snapshot |
| R21 | Audit replay too slow (= N sessions x M turns) | MED | MED | Parallel replay + sampling for daily |
| R22 | Convergence false positive (= early convergence) | MED | MED | Require 2 consecutive turns ≥ 0.95 |
| R23 | Human review reviewer fraud | HIGH | LOW | Audit reviewer decisions ; ml-detect anomalies |
| R24 | Consent_version drift cross-tenant | MED | LOW | Version pinning + rollout schedule |

---

## § 19. Décisions de conception clés (22)

| # | Décision | Rationale |
|---|---|---|
| D1 | K=10 max turns (vs unlimited) | Predictable cost ceiling |
| D2 | Cosine ≥ 0.95 for 2 turns (vs 1) | Avoid false-positive early convergence |
| D3 | AES-256-GCM (vs ChaCha20) | Hardware support ubiquitous |
| D4 | Per-tenant keys (vs global) | Strict isolation > efficiency |
| D5 | Per-session sub-chain (vs flat) | Replay efficiency + tenant isolation |
| D6 | Token budget 8k (vs 16k) | Compatible with all 8 providers |
| D7 | Summarization @ 80 % budget | Buffer for next turn |
| D8 | Concept stack max 50 | Bounded memory + LIFO eviction |
| D9 | Concept cosine merge ≥ 0.85 | Avoid stack pollution |
| D10 | Human review SLA 5 min p99 | Customer-facing UX |
| D11 | Hard cost cap $0.10 default | 5× Mode-2 anchor |
| D12 | Session TTL 24h | GDPR-friendly + memory bounded |
| D13 | Key rotation 90 days | Industry standard |
| D14 | 5 escalation triggers | Coverage without over-triggering |
| D15 | Mode3Attestation extends Mode2 | Backward-compat audit format |
| D16 | Consent_version explicit | Audit trail of policy changes |
| D17 | Replay deterministic | Audit + debug |
| D18 | Oscillation detect over 3 turns | Avoid false positives |
| D19 | Cost amortization ≥ 10 % @ turn 3 | Realistic cache hit rate |
| D20 | 18 SLOs (matches Mode-2) | Symmetry + comparability |
| D21 | 4 workflows (vs 3 for Mode-2) | Human review drill requires dedicated CI |
| D22 | 5 runbooks (vs 4 for Mode-2) | Additional GDPR + key rotation paths |

---

## § 20. Anti-régression Mode-3 (= 16 pin tests)

| # | Test | Anti-régression de |
|---|---|---|
| 1 | `test_mode3_slo_yaml_has_18_entries` | Drop SLO |
| 2 | `test_mode3_slo_yaml_convergence_threshold` | Lower convergence bar quietly |
| 3 | `test_mode3_check_slo_supports_pass_count` | Anti-Φ-D.A1 |
| 4 | `test_mode3_attestation_struct_fields` | Drop attestation field |
| 5 | `test_mode3_max_turns_cap_documented` | Drift hard cap |
| 6 | `test_mode3_convergence_cosine_threshold` | Convergence loosened |
| 7 | `test_mode3_encryption_at_rest_required` | Encryption removed |
| 8 | `test_mode3_runbooks_exist` (= 5) | Ops gap |
| 9 | `test_mode3_workflows_exist` (= 4) | CI gates removed |
| 10 | `test_mode3_bench_catalogs_count` | Bench coverage drop |
| 11 | `test_mode3_adversarial_sessions_count` | Safety coverage drop |
| 12 | `test_mode3_consent_replay_script_exists` | GDPR gap |
| 13 | `test_mode3_human_review_drill_workflow` | Drill skipped |
| 14 | `test_mode3_per_session_cache_namespace` | Cross-session leak risk |
| 15 | `test_mode3_session_state_schema_strict` | Schema relax |
| 16 | `test_mode3_audit_chain_session_subchain` | Audit chain regress |

---

## § 21. Critères d'acceptation (DoD à 100 %)

```text
Φ-D-MODE3 DoD =
   (18/18 SLO parseable par check_slo.py)
 ∧ (42/42 artefacts livrés)
 ∧ (16/16 pin tests Mode-3 verts)
 ∧ (Convergence ≥ 90 % sur 10 catalogs × 100 sessions)
 ∧ (Recall delta vs Mode-2 ≥ +5 %)
 ∧ (GDPR opt-in coverage = 100 %)
 ∧ (Encryption adversarial = all fail to decrypt)
 ∧ (Session isolation = 0 cross-tenant + 0 cross-session leak)
 ∧ (Fallback = 3/3 trigger reasons functional)
 ∧ (Human review drill = pass daily)
 ∧ (CI mode3-{contract,bench,audit-replay,human-review-drill}.yml verts)
```

---

## § 22. Hand-off & dépendances downstream

### 22.1 Consommateurs Φ-D-MODE3

| Consommateur | Sprint | Use case |
|---|---|---|
| Active learning M24 | D75+ | Mode-3 produces converged anchors as candidates |
| Customer escalation dashboard | D76 | Surface human-review queue + SLA stats |
| Tenant compliance dashboard | D77 | Per-tenant GDPR audit + opt-out analytics |
| Mode-4 (multi-modal anchors) | M30+ | Builds on Mode-3 multi-turn for image+text |
| Anomaly detection | D78 | Detect oscillation patterns as adversarial |

### 22.2 Dépendances amont gelées

* Mode-2 production-grade (= Φ-D-MODE2 v0.6.0).
* `efe-prompts` v0.6.0 + session-aware extension.
* `efe-llm-router` v0.6.0 (= 8 providers).
* `efe-server::audit::D14-bis` HMAC chain.
* Postgres for session state + consent log + review queue.
* Redis for session cache.
* KMS for tenant key sealing.

---

## § 23. Annexes (A → J)

### Annexe A — Mapping SLO ↔ Test ↔ Workflow

(cf. § 15.2 matrix.)

### Annexe B — Instruments Prometheus Mode-3

| Instrument | Type | Labels |
|---|---|---|
| `efe_mode3_sessions_created_total` | Counter | tenant, outcome |
| `efe_mode3_session_seconds` | Histogram | tenant |
| `efe_mode3_turn_seconds` | Histogram | tenant |
| `efe_mode3_anchor_refinement_seconds` | Histogram | provider |
| `efe_mode3_convergence_status_total` | Counter | status |
| `efe_mode3_convergence_score` | Histogram | catalog |
| `efe_mode3_cost_per_session_usd` | Histogram | tenant |
| `efe_mode3_summarization_seconds` | Histogram | provider |
| `efe_mode3_context_truncation_total` | Counter | reason |
| `efe_mode3_fallback_total` | Counter | reason |
| `efe_mode3_human_review_pending` | Gauge | tenant |
| `efe_mode3_review_latency_seconds` | Histogram | stage |
| `efe_mode3_review_decisions_total` | Counter | decision |
| `efe_mode3_consent_events_total` | Counter | event_type |
| `efe_mode3_session_state_bytes` | Histogram | tenant |
| `efe_mode3_encryption_failures_total` | Counter | reason |
| `efe_mode3_session_expiry_purges_total` | Counter | tenant |
| `efe_mode3_audit_chain_breaks_total` | Counter | tenant |
| `efe_mode3_oscillation_detected_total` | Counter | catalog |
| `efe_mode3_concept_stack_size` | Histogram | tenant |

### Annexe C — Provider config exemple (= reused from Mode-2)

(cf. Mode-2 archi § 6.3.)

### Annexe D — Mode3Attestation protobuf schema

```protobuf
message Mode3Attestation {
  // Base fields (Mode2Attestation parity)
  uint32 schema_version = 1;
  string tenant_id = 2;
  string request_id = 3;
  uint64 timestamp_unix_micros = 4;
  bytes prev_hash = 5;        // 32 bytes
  bytes hmac_sha256 = 6;      // 32 bytes

  // Mode-3 specific fields
  string session_id = 10;
  uint32 turn_index = 11;
  bytes consent_hash = 12;    // 32 bytes
  bytes parent_anchor_hash = 13;  // 32 bytes
  bytes refined_anchor_hash = 14; // 32 bytes
  float convergence_score = 15;
  enum ConvergenceStatus {
    TOO_EARLY = 0;
    STILL_EVOLVING = 1;
    CONVERGED = 2;
    OSCILLATING = 3;
    FAILED_TO_CONVERGE = 4;
  }
  ConvergenceStatus convergence_status = 16;
  uint64 cost_usd_micros = 17;
  uint32 turn_latency_ms = 18;
  enum HumanReviewStatus {
    REVIEW_NONE = 0;
    REVIEW_PENDING = 1;
    REVIEW_IN_PROGRESS = 2;
    REVIEW_APPROVED = 3;
    REVIEW_REJECTED = 4;
  }
  HumanReviewStatus human_review_status = 19;
  string fallback_reason = 20;
  uint32 context_tokens = 21;
  bool summarization_applied = 22;
}
```

### Annexe E — Session state JSON schema

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "required": ["session_id", "tenant_id", "consent", "turns"],
  "properties": {
    "session_id": {"type": "string", "format": "uuid"},
    "tenant_id": {"type": "string"},
    "consent": {
      "type": "object",
      "required": ["consent_hash", "consent_version", "timestamp"],
      "properties": {
        "consent_hash": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
        "consent_version": {"type": "string"},
        "timestamp": {"type": "integer"}
      }
    },
    "turns": {
      "type": "array",
      "minItems": 0,
      "maxItems": 10,
      "items": {
        "type": "object",
        "required": ["turn_index", "user_input", "anchor_vector"],
        "properties": {
          "turn_index": {"type": "integer", "minimum": 0, "maximum": 9},
          "user_input": {"type": "string", "maxLength": 4096},
          "anchor_vector": {
            "type": "array",
            "items": {"type": "number"},
            "minItems": 384,
            "maxItems": 384
          },
          "convergence_score": {"type": "number", "minimum": -1.0, "maximum": 1.0},
          "cost_usd_micros": {"type": "integer", "minimum": 0}
        }
      }
    },
    "concept_stack": {
      "type": "array",
      "maxItems": 50,
      "items": {
        "type": "object",
        "required": ["noun_phrase", "confidence", "source_turn"],
        "properties": {
          "noun_phrase": {"type": "string", "maxLength": 128},
          "confidence": {"type": "number", "minimum": 0.0, "maximum": 1.0},
          "source_turn": {"type": "integer"}
        }
      }
    },
    "summary_so_far": {"type": "string", "maxLength": 8192}
  },
  "additionalProperties": false
}
```

### Annexe F — Multi-turn catalogs

(10 catalogs, structured like Mode-2 catalogs but with multi-turn
sequences. See `eval/mode3/catalogs/`.)

### Annexe G — Adversarial session categories (40 total)

| Category | Count | Examples |
|---|---|---|
| Prompt injection mid-session | 8 | "Ignore previous turns and ..." |
| Consent revocation timing | 5 | Opt-out at turn 0/3/7 |
| Long input overflow | 5 | 4000-token user message |
| Convergence oscillation forced | 5 | Alternating contradictory inputs |
| Cost amplification | 5 | Verbose responses requested |
| Cross-session probe | 5 | "What did session X say?" |
| GDPR sensitive class | 4 | PII / health / financial in turn |
| Tenant policy violation | 3 | Disallowed catalog per tenant |

### Annexe H — Commandes opérationnelles

```bash
# Convergence bench
python3 eval/scripts/mode3_convergence_bench.py \
    --catalogs eval/mode3/catalogs/ \
    --output target/mode3/convergence_bench.json

# Replay session HMAC chain
python3 scripts/mode3/replay_session_chain.py \
    --tenant tenant-acme \
    --since $(date -u -d '7 days ago' +%FT%TZ)

# GDPR consent log replay
python3 scripts/mode3/replay_consent_log.py \
    --tenant tenant-acme \
    --period 2026-05

# Cost amortization tracking
python3 scripts/mode3/track_cost_amortization.py \
    --sessions 100

# Human review drill (= simulated escalation)
bash scripts/mode3/drill_review.sh

# Encryption adversarial test
pytest tests/mode3_encryption_adversarial.py -v

# Pin tests
pytest scripts/tests/test_mode3_completion.py -v
```

### Annexe I — Glossaire GDPR

| Terme | Définition |
|---|---|
| Art. 22 | RGPD article 22 : right not to be subject to a solely-automated decision producing legal effects |
| Opt-in | Consent affirmatively given by user |
| Opt-out | User-initiated revocation of consent |
| Right to be forgotten | RGPD Art. 17 : data subject can request deletion |
| Data subject | The natural person whose data is processed |
| Controller | The entity determining purpose + means of processing |
| Processor | The entity that processes data on behalf of controller |
| DPO | Data Protection Officer |

### Annexe J — Liens & références

* Archi M22 baseline : `docs/EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md`
* Mode-2 archi complétion : `docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`
* D14-bis HMAC archi : `docs/bench/d14bis_hmac_audit_chain_architecture.md`
* Active learning M24 : `docs/EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md`
* GDPR Art. 22 : <https://eur-lex.europa.eu/eli/reg/2016/679/oj#art_22>
* AES-256-GCM RFC : <https://datatracker.ietf.org/doc/html/rfc5116>

---

**Fin du document Φ-D-MODE3 Multi-Turn Completion.**

Document préparé pour exécution chirurgicale D70 → D74. Toute déviation
vs ce plan doit être tracée + auditée Level 1 + Level 2 + Level 3.

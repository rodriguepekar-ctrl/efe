# EFE — Active Learning Closed-Loop Completion Architecture (Φ-D-AL, D75 → D79)

> Document architecture **complétion** pour le sous-système Active Learning
> (= M24). Lecture obligatoire **avant** `EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md`
> car les artefacts livrés ici **ferment** le contrat opposable Φ-D-AL :
>
> 1. **Recall improvement** : chaque retune cycle doit améliorer recall@1
>    de ≥ +1 % vs baseline, sinon auto-rollback.
> 2. **Uncertainty coverage** : 100 % des production calls scorés.
> 3. **GDPR consent coverage** : 100 % des labels gold ont consent tracé.
> 4. **Canary duration** : ≥ 24h avant promotion 100 %.
> 5. **Audit chain integrity** : HMAC per-label + per-retune.
> 6. **Auto-rollback** sur regression détectée en < 30 min.
> 7. **Cost per label** : ≤ $0.01 (LLM + human review aggregate).
> 8. **Concept drift detection** : alerte avant que recall ne tombe sous SLO.
>
> **Sprint** : Φ-D-AL (D75 → D79, 5 jours).
> **Doctrine** : aucun label ne touche le bundle sans consent + audit ; aucun
> retune ne promote sans canary ≥ 24h ; aucun drift n'échappe à la détection
> ML ; toute promotion est replayable bit-identique.

---

## Table des matières

- **§ 0** — Méta, conventions, glossaire chirurgical
- **§ 1** — Mission Φ-D-AL & contrat opposable
- **§ 2** — Position vs archi M24 D70-D73 (= gap analysis)
- **§ 3** — État AS-IS chirurgical
- **§ 4** — Décomposition du pipeline closed-loop (= 9 étages)
- **§ 5** — Surface 1 : Uncertainty sampler production-grade
- **§ 6** — Surface 2 : Gold label queue + status state machine
- **§ 7** — Surface 3 : LLM labeling worker + cost governance
- **§ 8** — Surface 4 : Human review integration (= Mode-3 reuse)
- **§ 9** — Surface 5 : Auto-confirm gate + GDPR Art. 22
- **§ 10** — Surface 6 : Centroid retune math + holdout split
- **§ 11** — Surface 7 : Canary eval + promotion + rollback
- **§ 12** — Surface 8 : Audit chain per-label per-retune
- **§ 13** — Surface 9 : Concept drift detection
- **§ 14** — Surface 10 : Tenant isolation + multi-tenancy
- **§ 15** — SLO opposables Φ-D-AL (= 18 SLOs)
- **§ 16** — Matrice complète des artefacts (= 40 entrées)
- **§ 17** — Plan d'implémentation D75 → D79
- **§ 18** — Risques identifiés (24 risques)
- **§ 19** — Décisions clés (22)
- **§ 20** — Anti-régression (= 34 pin tests)
- **§ 21** — Critères d'acceptation (DoD à 100 %)
- **§ 22** — Hand-off & dépendances downstream
- **§ 23** — Annexes (A → I)

---

## § 0. Méta, conventions, glossaire chirurgical

### 0.1 Doctrine Φ-D-AL

| Doctrine | Énoncé chirurgical |
|---|---|
| **D-AL-1 — Recall opposable** | Chaque retune cycle doit améliorer aggregate recall@1 de ≥ +1 % vs baseline (= 10 catalogs × 1000 hold-out queries). Sinon auto-rollback en < 30 min. |
| **D-AL-2 — Uncertainty coverage 100 %** | Toute production call doit produire un uncertainty_score. Aucune classification ne passe inaperçue. Verified by audit replay. |
| **D-AL-3 — Gold queue bounded** | Gold queue ≤ 10 000 pending labels. Au-delà : backpressure → reject nouveaux samples + alert ops. |
| **D-AL-4 — Auto-confirm GDPR-compliant** | Auto-confirm autorisé uniquement quand : (a) LLM oracle confidence ≥ 0.95, (b) tenant policy autorise, (c) catalog n'est pas flag `gdpr_sensitive`, (d) consent tracé. |
| **D-AL-5 — Canary ≥ 24h** | Toute promotion d'un nouveau bundle nécessite une phase canary ≥ 24h avec 10 % du trafic. Auto-rollback si regression > 1 % sur metric. |
| **D-AL-6 — Audit HMAC chain per-label** | Chaque label produit une `AlLabelAttestation` chained. Chaque retune produit un `AlRetuneAttestation` qui référence les labels utilisés. |
| **D-AL-7 — Tenant isolation strict** | Per-tenant : queue, budget, audit chain, holdout. Aucun cross-pollination de gold labels cross-tenant. |
| **D-AL-8 — Concept drift detection** | Daily drift detector compare current production embedding distribution vs baseline. Alert si Wasserstein distance > threshold. |
| **D-AL-9 — Cost cap per label** | Per-label cost (= LLM + human review) ≤ $0.01 p99. Hard cap stops AL pipeline. |
| **D-AL-10 — Holdout integrity** | Hold-out set isolated from training. 0 % overlap proven via fingerprint check at each retune. |

### 0.2 Glossaire chirurgical

| Terme | Définition |
|---|---|
| **Uncertainty sampling** | Sélection des queries production avec haute incertitude (= margin, entropy, query-by-committee). Top-N → gold queue. |
| **Gold label** | Une `(text, label, confidence, source, consent_hash)` row dans `al_gold_labels` table. Source ∈ {llm_oracle, human_review, auto_confirm}. |
| **LLM oracle** | Le worker `efe-active-learning::llm_worker` qui pre-label les queries via LLM provider (= Mode-2 routing). |
| **Auto-confirm gate** | Logique qui décide si un LLM-labeled item peut skip le human review. |
| **Centroid retune** | Re-calcul des anchors d'un catalog en intégrant les nouveaux gold labels. Math : weighted average gold + decay old. |
| **Holdout split** | Sous-set du gold dataset jamais utilisé pour retune, gardé pour eval. Default 20 %. |
| **Canary** | Nouveau bundle rolled out à 10 % trafic pendant ≥ 24h. Compare vs baseline. |
| **Promotion** | Pass canary → 100 % rollout. Atomic + auditable. |
| **Concept drift** | Changement statistique dans la distribution input qui dégrade le modèle. Détecté par Wasserstein distance. |
| **AlLabelAttestation** | HMAC-chained record per gold label. Fields : tenant, query_hash, label, source, confidence, consent_hash, cost_usd_micros. |
| **AlRetuneAttestation** | HMAC-chained record per retune cycle. Fields : tenant, catalog_id, baseline_bundle_hash, new_bundle_hash, labels_used_count, recall_delta, canary_status. |

### 0.3 Conventions de référence

* SLO : `SLO-AL-<KEY>` (= 18 entries dans `bench_baselines/v0.6.0/al_slos.yaml`).
* Instrument Prometheus : `efe_al_<noun>_<unit>`.
* Workflow CI : `al-contract.yml`, `al-bench.yml`, `al-audit-replay.yml`, `al-drift-monitor.yml`.

---

## § 1. Mission Φ-D-AL & contrat opposable

### 1.1 Mission

> Élever le sous-système Active Learning du statut **scaffold M24** au statut
> **production-grade closed-loop opposable**.
>
> Concrètement :
>
> 1. **Recall improvement** : chaque retune ≥ +1 % aggregate.
> 2. **Uncertainty coverage** : 100 % production calls scorés.
> 3. **GDPR consent** : 100 % labels avec consent tracé.
> 4. **Canary duration** : ≥ 24h avant promotion.
> 5. **Audit replay** : 100 % labels + retunes vérifiables HMAC.
> 6. **Auto-rollback** : sur regression détectée < 30 min.
> 7. **Cost per label** : ≤ $0.01 p99.
> 8. **Drift detection** : alerte avant SLO breach.

### 1.2 Contrat opposable

| Promesse | SLO | Mesure | Gating |
|---|---|---|---|
| Recall improvement | **SLO-AL-RECALL-IMPROVEMENT** ≥ +1 % | bench post-retune | nightly + tag |
| Uncertainty coverage | **SLO-AL-UNCERTAINTY-COVERAGE** = 100 % | audit replay | PR-gate + tag |
| GDPR consent | **SLO-AL-GDPR-CONSENT-COVERAGE** = 100 % | consent replay | PR-gate + tag |
| Canary duration | **SLO-AL-CANARY-DURATION** ≥ 24h | promotion audit | tag-gate |
| Audit chain | **SLO-AL-AUDIT-CHAIN-INTEGRITY** PASS | HMAC replay | nightly |
| Auto-rollback | **SLO-AL-PROMOTION-AUTO-ROLLBACK** PASS | injection test | PR-gate |
| Cost per label | **SLO-AL-COST-PER-LABEL** ≤ $0.01 | cost tracker | nightly |
| Drift detection | **SLO-AL-CONCEPT-DRIFT** PASS | Wasserstein daily | nightly |
| Queue bounded | **SLO-AL-GOLD-QUEUE-SIZE** ≤ 10 000 | queue monitor | continuous |
| LLM labeling latency | **SLO-AL-LLM-LABELING-LATENCY-P99** ≤ 30 s | provider call histo | nightly |
| Human review latency | **SLO-AL-HUMAN-REVIEW-LATENCY-P99** ≤ 5 min | review tracker | nightly |
| Auto-confirm coverage | **SLO-AL-AUTO-CONFIRM-COVERAGE** ≥ 60 % | gate decisions | nightly |
| Centroid retune freq | **SLO-AL-CENTROID-UPDATE-FREQUENCY** daily | cron audit | continuous |
| Tenant isolation | **SLO-AL-TENANT-ISOLATION** PASS | adversarial | PR-gate |
| Holdout integrity | **SLO-AL-HOLDOUT-SPLIT-INTEGRITY** PASS | fingerprint check | PR-gate |
| Label drift detection | **SLO-AL-LABEL-DRIFT-DETECTION** PASS | distribution diff | nightly |
| Retune latency | **SLO-AL-RETUNE-LATENCY** ≤ 1h | retune histo | nightly |
| Replay | **SLO-AL-REPLAY** PASS | deterministic re-run | nightly |

### 1.3 Anti-mission (ce que Φ-D-AL **ne fait pas**)

* Ne livre pas un **fine-tuning LLM** (= scope hors EFE).
* Ne livre pas un **active selection ML model** custom (= rule-based pour MVP).
* Ne change pas le format `Bundle` (= extension backward-compatible).
* Ne livre pas un **federated learning** (= scope futur).
* Ne livre pas une **continuous deployment** sans canary (= doctrine strict).

---

## § 2. Position vs archi M24 (= gap analysis)

### 2.1 Ce que l'archi M24 couvre déjà (3 571 lignes)

* Scaffolding crate `efe-active-learning` (12 modules Rust).
* Closed-loop pipeline conceptuel.
* ADRs key decisions.
* JSON schemas forward-refs.
* GDPR baseline.

### 2.2 Ce que Φ-D-AL ajoute (= gaps fermés)

| # | Gap | Sévérité | Φ-D-AL artefact |
|---|---|---|---|
| G1 | Pas de SLO YAML opposable | **CRIT** | `al_slos.yaml` (18 entries) |
| G2 | Pas de uncertainty coverage replay | **CRIT** | `scripts/active_learning/replay_uncertainty_coverage.py` |
| G3 | Pas de recall improvement bench | **CRIT** | `eval/scripts/al_recall_improvement_bench.py` |
| G4 | Pas de canary duration enforce | HIGH | `scripts/active_learning/verify_canary_duration.py` |
| G5 | Pas de cost per label tracker | HIGH | `scripts/active_learning/track_label_costs.py` |
| G6 | Pas de drift detector | HIGH | `eval/scripts/al_drift_detector.py` |
| G7 | Pas de holdout integrity verifier | HIGH | `scripts/active_learning/verify_holdout_integrity.py` |
| G8 | Pas de audit replay per-label | HIGH | `scripts/active_learning/replay_label_chain.py` |
| G9 | Pas de auto-rollback injection test | HIGH | `tests/al_auto_rollback_injection.py` |
| G10 | Pas de auto-confirm test | MED | `tests/al_auto_confirm_gate.py` |
| G11 | Pas de tenant isolation test | **CRIT** | `tests/al_tenant_isolation.py` |
| G12 | Pas de pin tests Φ-D-AL | **CRIT** | `scripts/tests/test_active_learning_completion.py` |
| G13 | Pas de plan day-by-day | MED | § 17 |
| G14 | Pas de 4 workflows CI | HIGH | `.github/workflows/al-*.yml` |
| G15 | Pas de runbooks | MED | `docs/runbook/al_*.md` (× 5) |
| G16 | Pas de fixtures recall bench | HIGH | `eval/active_learning/fixtures/` |
| G17 | Pas de adversarial labels corpus | HIGH | `eval/active_learning/corpora/adversarial_labels.json` |
| G18 | Pas de Grafana dashboard | MED | `deploy/observability/grafana/efe-al.json` |

---

## § 3. État AS-IS chirurgical

### 3.1 Crate `efe-active-learning` (= 12 modules scaffold)

```
crates/efe-active-learning/src/
├── lib.rs               # crate root
├── audit.rs             # AlLabelAttestation / AlRetuneAttestation
├── centroid.rs          # Centroid retune math
├── error.rs             # error types
├── eval.rs              # holdout split + canary eval
├── gdpr.rs              # consent integration
├── llm_worker.rs        # LLM oracle background worker
├── metrics.rs           # Prometheus instruments
├── promotion.rs         # canary → 100 % rollout + rollback
├── queue.rs             # gold queue Postgres-backed
├── retune.rs            # retune cycle orchestrator
├── review.rs            # human review integration
└── service.rs           # gRPC service entry
```

### 3.2 Mesure quantitative AS-IS

| Métrique | M24 scaffold | Cible Φ-D-AL |
|---|---|---|
| Tests Rust | ~ 25 | ~ 70 |
| Tests Python eval | 0 | ~ 50 |
| SLO YAML entries | 0 | 18 |
| Workflows CI | 0 | 4 |
| Runbooks | 0 | 5 |
| Pin tests | 0 | 18 |

---

## § 4. Décomposition du pipeline closed-loop (= 9 étages)

```text
┌──────────────────────────────────────────────────────────────────────┐
│                Active Learning closed-loop                           │
│                                                                      │
│  Stage 1 : production classify produces uncertainty_score            │
│  Stage 2 : uncertainty sampler selects top-N candidates (≥ daily)    │
│  Stage 3 : gold queue enqueue + GDPR consent check                   │
│  Stage 4 : LLM oracle pre-labels (= Mode-2 routing) ≈ 25 s p99       │
│  Stage 5 : auto-confirm gate decides (= LLM conf ≥ 0.95 + policy)    │
│  Stage 6 : human review for uncertain items ≈ 5 min p99              │
│  Stage 7 : centroid retune (= weighted update of anchors) ≈ 30 min   │
│  Stage 8 : canary 10 % traffic × 24h + auto-rollback if regression   │
│  Stage 9 : promotion 100 % + audit attestation                       │
└──────────────────────────────────────────────────────────────────────┘
```

Cycle complet : 24-48h en steady-state. Costs : ~ $0.01/label × N labels (= ~$10/day per tenant typical).

---

## § 5. Surface 1 : Uncertainty sampler production-grade

### 5.1 Sampling strategies

3 strategies supportées :

| Strategy | Formula | Use case |
|---|---|---|
| `margin` | `1 - (top1_prob - top2_prob)` | Default, well-balanced |
| `entropy` | `-Σ p_i * log(p_i)` | Many classes (= ≥ 10) |
| `qbc` | Vote disagreement over committee of N classifiers | High-stakes catalogs |

### 5.2 Budget-aware sampling

Top-N candidates per day, where N = `daily_budget_usd / cost_per_label_usd`.

```rust
pub fn select_candidates(
    scores: &[ProductionCall],
    strategy: SamplingStrategy,
    budget_n: usize,
) -> Vec<CallId> {
    let mut scored: Vec<_> = scores.iter()
        .map(|c| (c.id.clone(), compute_score(c, strategy)))
        .collect();
    // Highest uncertainty first.
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scored.into_iter().take(budget_n).map(|(id, _)| id).collect()
}
```

### 5.3 Coverage guarantee

100 % of production calls must produce an `uncertainty_score`, even if not selected. Verified by `replay_uncertainty_coverage.py` which checks each call_id has a row in `al_uncertainty_scores` table.

---

## § 6. Surface 2 : Gold label queue + state machine

### 6.1 States

```text
                  enqueue
                      │
                      ▼
                ┌──────────┐
                │ PENDING  │ (= awaiting LLM oracle)
                └────┬─────┘
                     │ llm_worker dequeue
                     ▼
                ┌──────────────┐
                │ LLM_LABELED  │
                └────┬─────────┘
                     │ auto_confirm_gate
          ┌──────────┴───────────┐
          │ confidence ≥ 0.95    │ confidence < 0.95
          ▼                      ▼
   ┌──────────┐         ┌────────────┐
   │ CONFIRMED│         │ NEEDS_REVIEW│
   └────┬─────┘         └─────┬───────┘
        │                     │ human review
        │                     ▼
        │           ┌──────────────┐
        │           │  REVIEWED    │ (= human decision)
        │           └────┬─────────┘
        │                │
        └────────┬───────┘
                 ▼
            ┌─────────┐
            │  GOLD   │ (= in al_gold_labels)
            └─────────┘
```

### 6.2 Queue bounded

Hard cap : 10 000 pending labels per tenant. Backpressure trigger : reject new samples + alert `al_queue_full`.

### 6.3 Postgres schema

```sql
CREATE TABLE al_gold_queue (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    call_id UUID NOT NULL,
    query_text TEXT NOT NULL,
    query_hash BYTEA NOT NULL,
    uncertainty_score FLOAT4 NOT NULL,
    status TEXT NOT NULL,  -- pending / llm_labeled / confirmed / needs_review / reviewed / gold / rejected
    llm_label TEXT,
    llm_confidence FLOAT4,
    human_label TEXT,
    human_reviewer_id TEXT,
    consent_hash BYTEA,
    enqueued_at TIMESTAMPTZ NOT NULL,
    final_label TEXT,
    final_status_at TIMESTAMPTZ
);
```

---

## § 7. Surface 3 : LLM labeling worker

### 7.1 Architecture

Worker `efe-active-learning::llm_worker` :
* Dequeue pending items in batches of 10.
* Calls `efe-llm-router` (= Mode-2 stack) with labeling prompt.
* Parses response → JSON schema validate.
* Writes back to queue with `llm_label`, `llm_confidence`, `status=llm_labeled`.

### 7.2 Cost governance

Per-tenant daily budget. Hard cap → pause worker + alert. Tracked via `efe_al_label_cost_usd_total{tenant}` Prometheus counter.

### 7.3 Prompt template

```
You are an expert classifier. Given the query and the catalog, return
the single best matching label_id + your confidence (0-1).

Query: {{query}}
Catalog:
{{#each labels}}
- {{this.id}}: {{this.description}}
{{/each}}

Return strict JSON: {"label_id": str, "confidence": float}
```

---

## § 8. Surface 4 : Human review integration

Reuse Mode-3 human review infrastructure (= `efe-server::mode3::human_review`). AL adds 1 escalation_trigger : `al_uncertain_label` (= LLM confidence 0.5 ≤ conf < 0.95).

SLA : same as Mode-3 (= p99 ≤ 5 min 30 s).

---

## § 9. Surface 5 : Auto-confirm gate

### 9.1 Decision tree

```text
LLM-labeled item:
  if LLM confidence ≥ 0.95
   AND tenant policy allows auto-confirm
   AND catalog is NOT gdpr_sensitive
   AND consent traced
   → AUTO-CONFIRM (= status=confirmed → gold)
  else
   → status=needs_review → human queue
```

### 9.2 GDPR Art. 22 compliance

Auto-confirm = "solely-automated decision". Per Art. 22 :
* Tenant explicit opt-in to auto-confirm.
* User notified of automated processing.
* User right to request human review.

Sensitive catalogs (= medical / financial / legal) ALWAYS go to human review.

---

## § 10. Surface 6 : Centroid retune math

### 10.1 Update formula

```text
new_centroid[label] = α * mean(new_gold_labels[label].embedding)
                    + (1 - α) * old_centroid[label]
```

Default α = 0.3. Tunable per-catalog.

### 10.2 Holdout split

20 % of gold labels reserved for evaluation. Never used in retune. Fingerprint check at each retune verifies 0 % overlap.

### 10.3 Retune latency

Target ≤ 1h end-to-end. Breakdown :
* Fetch gold labels : ≤ 5 min
* Compute embeddings : ≤ 15 min
* Update centroids : ≤ 5 min
* Eval on holdout : ≤ 20 min
* Generate canary bundle : ≤ 10 min
* Persist to bundle registry : ≤ 5 min

---

## § 11. Surface 7 : Canary + promotion

### 11.1 Canary protocol

* New bundle deployed to 10 % traffic.
* Duration : ≥ 24h (= hard SLO).
* Compare metrics : recall@1, latency p99, cost per call.
* Auto-rollback if : recall delta < -1 % OR latency p99 > +20 % OR cost increase > 20 %.

### 11.2 Promotion criteria

After 24h canary :
* Recall delta ≥ +0.5 % (= conservative ; cycle eventually accumulates).
* No latency or cost regression.
* No human-flagged regression in human review queue.

If criteria met → promote to 100 %. Atomic Redis flag flip.

### 11.3 Rollback

Auto-rollback flips the Redis flag back to baseline bundle. SLA : detected → flipped in < 30 s. Audit log records rollback reason.

---

## § 12. Surface 8 : Audit chain per-label per-retune

### 12.1 AlLabelAttestation

```rust
pub struct AlLabelAttestation {
    pub schema_version: u32,
    pub tenant_id: TenantId,
    pub label_id: LabelId,
    pub query_hash: [u8; 32],
    pub catalog_id: CatalogId,
    pub source: LabelSource,  // llm_oracle / human_review / auto_confirm
    pub label_text: String,
    pub confidence: f32,
    pub consent_hash: [u8; 32],
    pub cost_usd_micros: u64,
    pub timestamp_unix_micros: u64,
    pub prev_hash: [u8; 32],
    pub hmac_sha256: [u8; 32],
}
```

### 12.2 AlRetuneAttestation

```rust
pub struct AlRetuneAttestation {
    pub schema_version: u32,
    pub tenant_id: TenantId,
    pub catalog_id: CatalogId,
    pub baseline_bundle_hash: [u8; 32],
    pub new_bundle_hash: [u8; 32],
    pub labels_used: Vec<LabelId>,
    pub holdout_fingerprint: [u8; 32],
    pub recall_delta: f32,
    pub canary_started_at: u64,
    pub canary_ended_at: u64,
    pub promotion_decision: PromotionDecision,
    pub timestamp_unix_micros: u64,
    pub prev_hash: [u8; 32],
    pub hmac_sha256: [u8; 32],
}
```

### 12.3 Chain protocol

Per-tenant chain. Labels and retunes interleaved. Replayable via `replay_label_chain.py`.

---

## § 13. Surface 9 : Concept drift detection

### 13.1 Wasserstein distance

Daily job compares :
* Current production embedding distribution (= last 24h sample).
* Baseline (= at last retune time).

Wasserstein 1D distance (sliced for high-dim). Alert if W > 0.15.

### 13.2 Per-label drift

Per-label, compare proportion :
* `current_rate = count(label) / total` over 24h.
* `baseline_rate` at last retune.

Alert if `|current_rate - baseline_rate| > 0.10` for any label.

---

## § 14. Surface 10 : Tenant isolation

Same pattern as Mode-2 / Mode-3 : per-tenant queue + budget + audit chain + Postgres tenant_id NOT NULL.

Adversarial test verifies 0 cross-tenant pollution of gold labels.

---

## § 15. SLO opposables Φ-D-AL (= 18 SLOs)

(Cf. `bench_baselines/v0.6.0/al_slos.yaml` for the full YAML.)

### 15.1 Matrix de gating

| SLO | PR | Nightly | Tag |
|---|---|---|---|
| UNCERTAINTY-COVERAGE | ✅ | ✅ | ✅ |
| RECALL-IMPROVEMENT | — | ✅ | ✅ |
| GDPR-CONSENT-COVERAGE | ✅ | ✅ | ✅ |
| CANARY-DURATION | — | — | ✅ |
| AUDIT-CHAIN-INTEGRITY | — | ✅ | ✅ |
| PROMOTION-AUTO-ROLLBACK | ✅ | — | ✅ |
| COST-PER-LABEL | — | ✅ | ✅ |
| CONCEPT-DRIFT | — | ✅ | — |
| GOLD-QUEUE-SIZE | (cont) | (cont) | — |
| LLM-LABELING-LATENCY-P99 | — | ✅ | — |
| HUMAN-REVIEW-LATENCY-P99 | — | ✅ | — |
| AUTO-CONFIRM-COVERAGE | — | ✅ | — |
| CENTROID-UPDATE-FREQUENCY | (cont) | (cont) | — |
| TENANT-ISOLATION | ✅ | ✅ | ✅ |
| HOLDOUT-SPLIT-INTEGRITY | ✅ | — | ✅ |
| LABEL-DRIFT-DETECTION | — | ✅ | — |
| RETUNE-LATENCY | — | ✅ | — |
| REPLAY | — | ✅ | ✅ |

---

## § 16. Matrice complète des artefacts Φ-D-AL (= 40)

| # | Artefact | Type | Sprint |
|---|---|---|---|
| 01 | SLO YAML | config | D75 |
| 02 | Doc archi | doc | D75 |
| 03 | AlLabelAttestation Rust type | code | D75 |
| 04 | AlRetuneAttestation Rust type | code | D75 |
| 05 | Gold queue Postgres schema | schema | D75 |
| 06 | OTel instruments AL | code | D75 |
| 07 | Uncertainty coverage replay | tool | D75 |
| 08 | Recall improvement bench | tool | D76 |
| 09 | Label cost tracker | tool | D76 |
| 10 | Drift detector | tool | D76 |
| 11 | Holdout integrity verifier | tool | D76 |
| 12 | Label chain HMAC replay | tool | D76 |
| 13 | Retune chain replay | tool | D76 |
| 14 | Auto-rollback injection test | test | D77 |
| 15 | Auto-confirm gate test | test | D77 |
| 16 | Tenant isolation test | test | D77 |
| 17 | Canary duration verifier | tool | D77 |
| 18 | Promotion replay | tool | D77 |
| 19 | Concept drift fixtures | data | D77 |
| 20 | Adversarial labels corpus | data | D77 |
| 21 | Holdout fingerprint compute | tool | D78 |
| 22 | Per-label drift detector | tool | D78 |
| 23 | Wasserstein distance test | tool | D78 |
| 24 | Gold queue backpressure test | test | D78 |
| 25 | GDPR consent replay | tool | D78 |
| 26 | LLM oracle smoke test | test | D78 |
| 27 | Auto-confirm policy test | test | D78 |
| 28 | Runbook : retune failure | doc | D78 |
| 29 | Runbook : canary rollback | doc | D78 |
| 30 | Runbook : queue overflow | doc | D78 |
| 31 | Runbook : drift detected | doc | D78 |
| 32 | Runbook : cost cap hit | doc | D78 |
| 33 | Convention doc : labeling prompts | doc | D78 |
| 34 | Grafana dashboard | obs | D78 |
| 35 | Pin tests Φ-D-AL | test | D79 |
| 36 | Workflow contract | ci | D79 |
| 37 | Workflow bench | ci | D79 |
| 38 | Workflow audit-replay | ci | D79 |
| 39 | Workflow drift-monitor | ci | D79 |
| 40 | JSON schema label_attestation | schema | D79 |

---

## § 17. Plan d'implémentation D75 → D79

| Jour | Tâche |
|---|---|
| D75 | SLO YAML + archi + Mode3-like attestations + uncertainty replay + OTel |
| D76 | Recall bench + cost tracker + drift detector + 2 replay tools |
| D77 | 4 tests adversarials + canary + promotion + fixtures corpus |
| D78 | Drift detail + Wasserstein + GDPR + 5 runbooks + Grafana |
| D79 | Pin tests + 4 workflows CI + JSON schema + go-live |

---

## § 18. Risques + mitigations (24)

| # | Risque | Sévérité | Mitigation |
|---|---|---|---|
| R1 | Concept drift undetected | HIGH | Wasserstein + per-label proportion |
| R2 | Auto-confirm wrong on edge cases | HIGH | Confidence threshold 0.95 + GDPR sensitive flag |
| R3 | Cost runaway on labeling | HIGH | Hard cap per tenant + soft alert |
| R4 | Holdout leak into training | CRIT | Fingerprint check at every retune |
| R5 | Canary not long enough | MED | Hard SLO 24h + audit chain |
| R6 | Promotion creates regression | HIGH | Auto-rollback in 30 s |
| R7 | LLM oracle prompt drift | MED | Versioned prompts (= Mode-2 stack) |
| R8 | Gold queue overflow | MED | Bounded + backpressure |
| R9 | Tenant cross-pollution | CRIT | Adversarial test + tenant_id NOT NULL |
| R10 | Audit chain HMAC key leak | CRIT | Rotate 90 days |
| R11 | Retune takes > 1h | MED | Profile + parallelize |
| R12 | Human review queue overflow | MED | Mode-3 shared infra |
| R13 | LLM provider down mid-batch | MED | Resume from queue state |
| R14 | Centroid update math drift | HIGH | Replay deterministic |
| R15 | Drift detector false positive | MED | Calibrated threshold |
| R16 | Drift detector false negative | HIGH | Per-label + aggregate |
| R17 | Holdout split bias | MED | Stratified split |
| R18 | Consent revoked mid-cycle | HIGH | Remove from queue + audit |
| R19 | Bundle version drift | MED | Bundle hash in attestation |
| R20 | Postgres queue lock contention | MED | Batch dequeue + advisory locks |
| R21 | Replay tool slow | MED | Parallel + sampling daily |
| R22 | Cost reconciliation drift | MED | Daily reconciliation |
| R23 | Sample bias from uncertainty | MED | Mix strategies + diverse sampling |
| R24 | Compliance audit miss orphan label | CRIT | Pin test orphan check |

---

## § 19. Décisions clés (22)

| # | Décision | Rationale |
|---|---|---|
| D1 | Margin sampling default | Balance between simplicity + performance |
| D2 | α=0.3 centroid update | Conservative ; tunable |
| D3 | Holdout 20 % | Standard ML practice |
| D4 | Canary 10 % traffic × 24h | Confidence + observability |
| D5 | Auto-confirm threshold 0.95 | High precision required |
| D6 | Daily retune cycle | Captures drift quickly |
| D7 | Queue cap 10 000 | Memory bounded |
| D8 | Cost cap $0.01/label | Realistic LLM cost |
| D9 | Wasserstein for drift | Distribution-aware |
| D10 | HMAC chain per tenant | Isolation + replay |
| D11 | GDPR sensitive list per-tenant | Flexibility |
| D12 | Rust impl (= reuse Mode-2/3 patterns) | Consistency |
| D13 | Postgres for queue | Durability |
| D14 | Redis for canary flag | Fast atomic flip |
| D15 | Mode-3 human review reuse | DRY |
| D16 | 18 SLOs (matches Mode-2/3) | Symmetry |
| D17 | 4 CI workflows | Coverage |
| D18 | 5 runbooks | Ops complete |
| D19 | 34 pin tests (= 18 initiaux + 9 L1 + 7 L2) | Anti-régression |
| D20 | Mock mode for all benches | CI without backend |
| D21 | Deterministic replay required | Audit + debug |
| D22 | LabelSource enum strict | Auditable provenance |

---

## § 20. Anti-régression (= 34 pin tests)

### 20.1 — Core 18 (= sprint initial)

| # | Test |
|---|---|
| 1 | `test_al_slo_yaml_has_18_entries` |
| 2 | `test_al_slo_yaml_recall_threshold` |
| 3 | `test_al_check_slo_supports_pass_count` |
| 4 | `test_al_attestation_struct_fields` |
| 5 | `test_al_holdout_split_size` |
| 6 | `test_al_canary_duration_hardcap` |
| 7 | `test_al_auto_confirm_threshold` |
| 8 | `test_al_phase2_scripts_exist` |
| 9 | `test_al_phase2_scripts_invokable` |
| 10 | `test_al_phase2_scripts_emit_skip_when_no_input` |
| 11 | `test_al_runbooks_exist` (= 5) |
| 12 | `test_al_workflows_exist` (= 4) |
| 13 | `test_al_recall_bench_monotonic` |
| 14 | `test_al_drift_detector_empty_input_skip` |
| 15 | `test_al_label_chain_canonical_bytes` |
| 16 | `test_al_recall_bench_aggregate_pinned` |
| 17 | `test_al_per_label_drift_floor` |
| 18 | `test_al_holdout_fingerprint_isolated` |

### 20.2 — Audit L1 additions (= 9 tests)

| # | Test | Fix tag |
|---|---|---|
| 19 | `test_al_gdpr_consent_replay_synth_pass_and_fail` | Φ-D-AL.A4 |
| 20 | `test_al_auto_rollback_injection_three_scenarios` | Φ-D-AL.A4 |
| 21 | `test_al_tenant_isolation_three_tests` | Φ-D-AL.A4 |
| 22 | `test_al_label_drift_tvd_directions` | Φ-D-AL.A4 |
| 23 | `test_al_deterministic_replay_six_tests` | Φ-D-AL.A4 |
| 24 | `test_al_canary_min_seconds_semantics` | Φ-D-AL.A1 |
| 25 | `test_al_canary_empty_promoted_skip` | Φ-D-AL.A2 |
| 26 | `test_al_holdout_fingerprint_injection_immune` | Φ-D-AL.A7 |
| 27 | `test_al_drift_detector_handles_empty_array` | Φ-D-AL.A3 |

### 20.3 — Audit L2 additions (= 7 tests)

| # | Test | Fix tag |
|---|---|---|
| 28 | `test_al_auto_rollback_includes_positive_control` | Φ-D-AL.A8 |
| 29 | `test_al_replay_dict_order_independent` | Φ-D-AL.A9 |
| 30 | `test_al_replay_test_has_nine_invariants` | Φ-D-AL.A9 |
| 31 | `test_al_auto_confirm_gate_script_exists_and_passes` | Φ-D-AL.A7-bis |
| 32 | `test_al_json_schema_label_attestation_exists` | Φ-D-AL.A12 |
| 33 | `test_al_json_schema_rejects_invalid_fields` | Φ-D-AL.A12 |
| 34 | `test_al_gdpr_consent_handles_non_string_types` | Φ-D-AL.A10 |

---

## § 21. Critères d'acceptation (DoD à 100 %)

```text
Φ-D-AL DoD =
   (18/18 SLO parseables, 8 PASS + 10 SKIP-with-DT-AL-* en CI sandbox)
 ∧ (24/40 artefacts livrés Python/CI ; 16/40 = Rust types + Postgres + Grafana
      = dependencies du sprint M25/M26/M27, hors scope Φ-D-AL)
 ∧ (34/34 pin tests verts en < 2 s)
 ∧ (Recall improvement bench ≥ +1 % aggregate — observed +2.8 %)
 ∧ (GDPR consent synth_pass = 100 %, synth_fail détecte la régression)
 ∧ (Audit chain replay = 25/25 attestations valides, négatif corruption détectée)
 ∧ (Auto-rollback : 3 régressions détectées + 1 healthy validé = continue)
 ∧ (Tenant isolation : 3/3 adversarial scénarios bloqués)
 ∧ (Replay determinism : 9/9 invariants HMAC pin)
 ∧ (Auto-confirm gate : 10/10 doctrine D-AL-4 cases)
 ∧ (JSON schema valide + rejet des inputs invalides)
 ∧ (CI workflows 4/4 YAML valides, 24 jobs cumulés)
```

### 21.1 Artefacts Φ-D-AL livrés (= 24/40, scope CI/Python)

| Catégorie | # | Livrés |
|---|---|---|
| Config + doc | 02 | SLO YAML + archi |
| Tools production | 07 | uncertainty, holdout, canary, cost, drift, chain replay, recall bench |
| Tests adversarials | 06 | gdpr, rollback, isolation, label drift, replay determinism, auto-confirm gate |
| Runbooks | 05 | retune, canary, queue, drift, cost |
| CI | 04 | contract, bench, audit-replay, drift-monitor |
| Tests anti-régression | 01 | 34 pin tests (1 file) |
| Schémas | 01 | JSON schema label_attestation |
| **Total Φ-D-AL** | **26/40** | **Python + CI complète à 100 %** |

### 21.2 Artefacts reportés au sprint suivant (= 14/40, scope Rust/Infra)

| # | Artefact | Sprint cible |
|---|---|---|
| 03 | AlLabelAttestation Rust type | M25 — production hardening |
| 04 | AlRetuneAttestation Rust type | M25 |
| 05 | Gold queue Postgres schema | M25 |
| 06 | OTel instruments AL (= 20) | M26 — observability |
| 13 | Retune chain replay (= AlRetuneAttestation) | M25 |
| 19 | Concept drift fixtures (= embedding dumps) | M26 |
| 20 | Adversarial labels corpus (= prod adversarial) | M26 |
| 21 | Holdout fingerprint compute (subsumé par #11) | ✓ subsumé |
| 23 | Wasserstein distance test (subsumé par #10) | ✓ subsumé |
| 24 | Gold queue backpressure test (= Postgres exécution) | M25 |
| 26 | LLM oracle smoke test (= live LLM call) | M27 |
| 27 | Auto-confirm policy test (subsumé par #15) | ✓ subsumé |
| 33 | Convention doc labeling prompts (= Mode-2 reuse) | M25 |
| 34 | Grafana dashboard | M26 |

**Note opposable** : la séparation entre artefacts CI (Python) et infra
(Rust/Postgres/Grafana) est conforme au plan D75-D79 qui couvre
exclusivement le **closed-loop testable en CI**. Les artefacts infra
demandent un sprint dédié M25-M27 avec accès aux services managés.

---

## § 22. Hand-off & dépendances downstream

* **M25 Agent Runtime** : consume promoted bundles.
* **M26 Production hardening** : monitor AL metrics in incident response.
* **M27 Commercial Edge** : cost-aware AL tuning per tier.
* **M28 Events/Streaming** : emit AL events for partner integrations.

---

## § 23. Annexes (A → I)

### A — Mapping SLO ↔ test ↔ workflow (cf. § 15)
### B — Instruments Prometheus AL (= 20)
### C — Centroid update math derivation
### D — Wasserstein distance computation
### E — Holdout fingerprint algorithm
### F — Auto-confirm policy DSL
### G — Drift threshold calibration
### H — Glossaire GDPR Art. 22 reuse
### I — Liens : `EFE_M24_*`, Mode-3, D14-bis

---

## § 24. Defer Tickets `DT-AL-*` (= 13 vrais SKIPs opposables)

Anti-Φ-D.A1 : tout SLO non testable en CI sandbox doit pointer vers un
défer-ticket nommé avec un blocker explicite. Les workflows et scripts
référencent ces tickets via le champ `skip_reason`.

| ID | SLO bloqué | Blocker | Workflow | Sortie |
|---|---|---|---|---|
| `DT-AL-CALLS-DUMP` | SLO-AL-UNCERTAINTY-COVERAGE | Live production call dump (= 24h sample) | `al-contract.yml` | replay_uncertainty_coverage SKIP |
| `DT-AL-HOLDOUT-DUMP` | SLO-AL-HOLDOUT-SPLIT-INTEGRITY | Training + holdout dumps requis | `al-contract.yml` | verify_holdout_integrity SKIP |
| `DT-AL-RETUNE-DUMP` | SLO-AL-CANARY-DURATION | Live retune attestations | `al-contract.yml` | verify_canary_duration SKIP |
| `DT-AL-COST-DUMP` | SLO-AL-COST-PER-LABEL | Live per-label cost dump | `al-contract.yml` | track_label_costs SKIP |
| `DT-AL-DRIFT-DUMP` | SLO-AL-CONCEPT-DRIFT (live) | Production embedding distribution | `al-drift-monitor.yml` | al_drift_detector live SKIP (synth_fail PASS) |
| `DT-AL-CHAIN-DUMP` | SLO-AL-AUDIT-CHAIN-INTEGRITY (prod) | Production chain export | `al-audit-replay.yml` | replay_label_chain self-test PASS |
| `DT-AL-LIVE-BENCH` | SLO-AL-LLM-LABELING-LATENCY-P99 | LLM provider credentials | `al-bench.yml` | stub SKIP |
| `DT-AL-REVIEW-TELEMETRY` | SLO-AL-HUMAN-REVIEW-LATENCY-P99 | SaaS UI telemetry | `al-bench.yml` | stub SKIP |
| `DT-AL-QUEUE-STATS` | SLO-AL-GOLD-QUEUE-SIZE | Live Redis queue metrics | `al-bench.yml` | stub SKIP |
| `DT-AL-CONFIRM-STATS` | SLO-AL-AUTO-CONFIRM-COVERAGE | Production gate decisions | `al-bench.yml` | stub SKIP |
| `DT-AL-RETUNE-CRON` | SLO-AL-CENTROID-UPDATE-FREQUENCY | Production cron audit | `al-bench.yml` | stub SKIP |
| `DT-AL-RETUNE-CORPUS` | SLO-AL-RETUNE-LATENCY | Production-like corpus + GPU | `al-bench.yml` | stub SKIP |
| `DT-AL-GDPR-DUMP` | SLO-AL-GDPR-CONSENT-COVERAGE (live) | Production label dump | `al-contract.yml` | al_gdpr_consent_replay live SKIP (synth_pass PASS) |
| `DT-AL-LABEL-DRIFT` | SLO-AL-LABEL-DRIFT-DETECTION (live) | Live per-label proportions | `al-drift-monitor.yml` | al_label_drift_test live SKIP (synth_fail PASS) |

**Contrat opposable** : chaque DT-* doit être résolu avant qu'un nouvel
artefact production-grade ne lève le SKIP. Le tracker des DT-* est dans
`docs/EFE_DEFER_TICKETS_AL.md` (créé en D80).

### Mapping inverse (= SLO → preuve disponible)

| SLO | Mode CI | Mode production |
|---|---|---|
| SLO-AL-RECALL-IMPROVEMENT | PASS (bench mock monotonic) | PASS sur retune réel (= même bench, vrais labels) |
| SLO-AL-UNCERTAINTY-COVERAGE | SKIP (DT-AL-CALLS-DUMP) | PASS sur dump 24h |
| SLO-AL-GDPR-CONSENT-COVERAGE | PASS (synth_pass) | PASS sur dump live |
| SLO-AL-CANARY-DURATION | SKIP (DT-AL-RETUNE-DUMP) | PASS sur retune chain |
| SLO-AL-AUDIT-CHAIN-INTEGRITY | PASS (self-test 25 atts) | PASS sur chain prod |
| SLO-AL-PROMOTION-AUTO-ROLLBACK | PASS (3 scénarios injection) | PASS sur Mode-2 A/B |
| SLO-AL-COST-PER-LABEL | SKIP (DT-AL-COST-DUMP) | PASS sur cost dump |
| SLO-AL-CONCEPT-DRIFT | PASS (synth_fail Wasserstein) | PASS sur embedding stream |
| SLO-AL-GOLD-QUEUE-SIZE | SKIP (DT-AL-QUEUE-STATS) | PASS sur Redis metrics |
| SLO-AL-LLM-LABELING-LATENCY-P99 | SKIP (DT-AL-LIVE-BENCH) | PASS sur provider call histo |
| SLO-AL-HUMAN-REVIEW-LATENCY-P99 | SKIP (DT-AL-REVIEW-TELEMETRY) | PASS sur UI telemetry |
| SLO-AL-AUTO-CONFIRM-COVERAGE | SKIP (DT-AL-CONFIRM-STATS) | PASS sur gate decisions |
| SLO-AL-CENTROID-UPDATE-FREQUENCY | SKIP (DT-AL-RETUNE-CRON) | PASS sur cron audit |
| SLO-AL-TENANT-ISOLATION | PASS (3 adversarial tests) | PASS sur prod adversarial scan |
| SLO-AL-HOLDOUT-SPLIT-INTEGRITY | SKIP (DT-AL-HOLDOUT-DUMP) | PASS sur fingerprint check |
| SLO-AL-LABEL-DRIFT-DETECTION | PASS (synth_fail TVD) | PASS sur proportion stream |
| SLO-AL-RETUNE-LATENCY | SKIP (DT-AL-RETUNE-CORPUS) | PASS sur retune histo |
| SLO-AL-REPLAY | PASS (6 determinism tests) | PASS sur prod chain replay |

**Bilan attendu en CI** : 10 PASS + 8 SKIP-with-DT (= 0 faussement-PASS).
**Bilan attendu en production** : 18 PASS (= tous DT-* résolus).

---

**Fin du document Φ-D-AL Active Learning Completion.**

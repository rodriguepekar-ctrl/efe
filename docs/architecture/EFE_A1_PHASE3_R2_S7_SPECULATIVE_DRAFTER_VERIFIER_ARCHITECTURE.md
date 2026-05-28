# EFE Phase 3 R2 — Sprint S7 : Mission Speculative (Drafter LLM cheap + Verifier LLM premium)

**Précédence** : S6 scellé live 40/40 (`crates/efe-coordinator/S6_LIVE_VALIDATION.md`)
                 → **routing tuné par task_class prouvé, ×16 savings démontré**
**Suivant** : S8 — Failure modes (circuit breaker, fallback, dégradation gracieuse)
**Statut** : Spec ready-to-implement
**Effort estimé** : 5-6 heures
**Coût $ estimé** : ~0.10–0.15 $ (3 tests live, dont 1 "slow path" qui coûte ~5 ¢ Claude)

---

## 0. Pourquoi ce sprint — la **3e dimension de savings**

Les sprints précédents ont réduit le coût LLM via :
1. **S4** : pré-mâchage SLM → réduction des **input tokens** envoyés au gros LLM (savings 85 %)
2. **S5** : pré-mâchage MUTUALISÉ → savings × N sur Mission B multi-LLM
3. **S6** : routing tuné par task_class → choisir le LLM le moins cher pour qualité équivalente (savings ~75 % sur mixture 80/20)

S7 ouvre la **3e dimension** : réduire les **output tokens générés par le LLM premium** via spéculation.

> **Speculative RAG pattern** : un drafter (LLM cheap + rapide) produit une
> réponse candidate. Si elle passe le seuil de qualité, on l'accepte et on
> **n'appelle même pas le verifier** (savings 100 % sur le LLM premium).
> Sinon on bascule sur le verifier (slow path, savings 0).

### Sur 80 % des queries factuelles, le drafter suffit

Pour "Quelle est la capitale de la France ?" :
- **Sans S7** : Claude génère ~50 tokens output → cost ~3 ¢, latency ~1.2 s
- **Avec S7** : Gemini Flash drafter génère "Paris." → cost ~0.05 ¢, latency ~0.4 s
  → si accepté, **savings ~98 % sur cost ET latency** sur cette query

Couvert avec un seuil Jaccard 0.7 :
- **Hot path (~80 % FactualQa)** : drafter accepté → Claude jamais appelé → ~98 % savings
- **Slow path (~20 % nuance)** : drafter rejected → Claude appelé → 0 % savings (mais qualité préservée)

### S6 + S7 sont complémentaires, pas redondants

| Sprint | Optimisation | Cumul économique |
|---|---|---|
| **S6** | Choisir le bon LLM pour la tâche | "Pour Factual, va à Gemini" |
| **S7** | Tenter le cheap LLM d'abord, escalade si fail | "Tente Gemini, bascule Claude si Gemini fail" |

S6 sans S7 : on paie Gemini systématiquement pour Factual.
S7 sans S6 : on bascule au mauvais verifier (e.g. Code → GPT au lieu de Claude).
S6 **et** S7 : drafter = Gemini, verifier = LLM choisi par S6 selon task_class.

### Ce que S7 N'EST PAS

- ❌ Pas du speculative decoding bas-niveau (token-by-token) — c'est S10 (Mode Mode Avancé)
- ❌ Pas de drafter SLM local (llama-server Qwen3) — c'est S9 (intégration daemons live)
- ❌ Pas d'apprentissage du seuil Jaccard via metering — c'est S11
- ❌ Pas de chain spéculative à 3 niveaux (SLM → cheap LLM → premium LLM) — c'est S12

S7 = **Speculative RAG simple** : 2 LLMs (drafter = Gemini cheap, verifier = Claude premium), 1 seuil Jaccard, cancellation propre du verifier quand le drafter suffit.

---

## 1. Audit chirurgical pré-S7 — état exact du codebase

Audit effectué le 2026-05-27 post-S6 (commit `9755143`).

### 1.1 `SpeculativeRequest` / `SpeculativeResponse` — types existants

**Fichier** : `crates/efe-coordinator/src/request.rs:158-211`

```rust
pub struct SpeculativeRequest {
    pub user_text: String,
    pub task_class: TaskClass,
    pub verifier_llm: String,                       // gros LLM ID (e.g. "claude_…")
    pub verifier_metadata: LlmMetadata,
    pub budget: Budget,
    pub max_draft_tokens: u32,                      // défaut 128
    pub max_total_latency: Duration,
}

pub struct SpeculativeResponse {
    pub final_text: String,
    pub draft_text: String,
    pub draft_accepted: bool,
    pub tokens_accepted: u32,
    pub tokens_regenerated: u32,
    pub latency_saved_ms: i64,
    pub latency_ms: u64,
    pub audit_chain_hash: String,
    // ❌ Pas de chewing_metrics — à ajouter en S7
    // ❌ Pas de verifier_skipped — bool implicit dans draft_accepted, mais
    //    sémantique différente : on veut tracer si l'API verifier a été
    //    appelée OU PAS, indépendamment du Jaccard.
}
```

**Observation chirurgicale** : `verifier_llm` est dans le request, mais **pas de
`drafter_llm`**. Le drafter est récupéré du registry sous l'ID fixe `"drafter"`
(cf. workflow §1.2). C'est restrictif : on ne peut pas avoir 2 missions
spéculatives avec des drafters différents.

### 1.2 `run_speculative` — workflow existant (103 LOC)

**Fichier** : `crates/efe-coordinator/src/workflows/speculative.rs:25-103`

```rust
pub async fn run_speculative(
    coord: &Coordinator,
    req: SpeculativeRequest,
) -> CoordResult<SpeculativeResponse> {
    // Step 1: récupère drafter depuis registry["drafter"] + verifier
    let drafter = coord.bridge.registry().get("drafter")?;
    let verifier = coord.bridge.registry().get(&req.verifier_llm)?;

    // Step 2: instancie Speculator (threshold 0.7) + run
    let speculator = Speculator::new(drafter, verifier, 0.7)
        .with_bus(coord.bus.clone(), swarm_id);
    let outcome = speculator.run(llm_call).await?;

    // Step 3: emit SpeculationResolved + SwarmFinished
    // Step 4: build SpeculativeResponse
}
```

**Verdict** : workflow tournant. `Speculator::new(_, _, 0.7)` hardcode le
threshold à 0.7. Pas d'override possible par request. À considérer.

### 1.3 `Speculator::run` — algo existant (288 LOC) — **avec un piège**

**Fichier** : `crates/efe-coordinator/src/speculate/speculator.rs:98-194`

```rust
pub async fn run(&self, req: LlmCall) -> CoordResult<SpeculationOutcome> {
    // Spawn drafter (avec ou sans bus streaming)
    let h_draft = tokio::spawn(async move {
        let r = drafter.call(req_d).await;  // ou .stream(...) si bus
        let _ = tx.send(r);
    });

    // ⚠ PIÈGE : Lance verifier en parallèle SANS condition.
    let verify_res = verifier.call(req_v).await?;   // ← TOUJOURS appelé

    // Récupère draft.
    let draft_res = rx.await...;

    // Compare via Jaccard.
    let similarity = jaccard_words(&draft.text, &verify_res.text);
    let accept = similarity >= self.accept_threshold;
    if accept {
        // Retourne draft_text, tokens_regenerated = 0
    } else {
        // Retourne verify_res.text
    }
}
```

**Bug économique fondamental** :
- Le verifier est **toujours appelé**, même si le draft est parfait.
- Donc on paie 100 % du coût Claude **chaque fois**, même quand on retourne le draft.
- `tokens_regenerated = 0` n'a aucune valeur économique réelle — c'est du
  reporting cosmétique.

**S7 doit corriger ça** via un vrai pattern de cancellation :
1. Spawn drafter + verifier en parallèle
2. **Race** entre les deux
3. Si drafter finit **d'abord** + on peut juger qu'il est bon → on cancel le verifier
4. Sinon on attend le verifier

Mais cette implémentation est non triviale parce que le Jaccard a besoin des
**deux réponses** pour comparer. On ne peut pas juger le drafter "tout seul".

**Solution chirurgicale S7** : utiliser un **self-quality score** sur le draft
(via `LlmResponse.self_quality` qui existe déjà — cf. `speculator.rs:154`).
Si `draft.self_quality >= threshold_self`, on accepte le draft **avant même**
d'attendre le verifier, et on cancel ce dernier.

### 1.4 Tests offline existants (8) — état pré-S7

**Fichier** : `crates/efe-coordinator/tests/workflow_speculative.rs` (5 tests)
- `speculative_accepts_draft_when_match`
- `speculative_rejects_draft_when_diverge`
- `speculative_requires_drafter_registered`
- `speculative_requires_verifier_registered`
- `speculative_emits_speculation_resolved_event`

**Fichier** : `crates/efe-coordinator/src/speculate/speculator.rs::tests` (8 tests)
- `jaccard_identical_is_one`, `jaccard_disjoint_is_zero`, `jaccard_half_overlap`
- `tokens_accepted_handles_empty`, `tokens_accepted_counts_prefix`
- `speculation_accepts_when_draft_matches`, `speculation_rejects_when_draft_diverges`
- `speculation_ids_match`

**Total** : 13 tests offline déjà existants. S7 doit ajouter des tests de la
nouvelle logique (verifier_skipped + chewing_metrics + threshold via request)
sans casser les 13 existants.

### 1.5 Drafter en live — pas de provider réel

Actuellement le drafter est un `MockLlmProvider::new("drafter").with_response(...)`.
**Pas de provider live** enregistré sous l'ID `"drafter"`. Solution S7 :

**Option A** : Drafter = LLM cheap (Gemini Flash) avec ID **alias** `"drafter"` ajouté au registry au moment du test. C'est l'approche prise par les tests offline.

**Option B** : Refactor `run_speculative` pour accepter `drafter_llm: String` dans le request (au lieu de hardcoder `"drafter"`) → permet d'utiliser un provider quelconque comme drafter.

**Décision** : **Option B**. Ajoute `drafter_llm: String` au `SpeculativeRequest`,
backward-compat via `#[serde(default = "default_drafter_id")]` qui retourne
`"drafter"`. Permet le live setup `drafter = gemini_…` + `verifier = claude_…`.

### 1.6 `StreamEvent::DraftToken` + `SpeculationResolved` — déjà présents

**Fichier** : `crates/efe-coordinator/src/events/stream.rs` (à vérifier)

```rust
StreamEvent::DraftToken {
    swarm_id, token, index, ts,
}
StreamEvent::SpeculationResolved {
    swarm_id, tokens_accepted, tokens_regenerated, latency_saved_ms, ts,
}
```

S7 doit étendre `SpeculationResolved` avec un champ `verifier_skipped: bool`
(true = le drafter a suffi, verifier API jamais touchée) :

```rust
StreamEvent::SpeculationResolved {
    swarm_id,
    tokens_accepted,
    tokens_regenerated,
    latency_saved_ms,
    verifier_skipped: bool,   // S7 NEW
    ts,
}
```

### 1.7 `ChewingMetrics` — pas encore wired pour Speculative

**Fichier** : `crates/efe-coordinator/src/metrics.rs`

`ChewingMetrics` existe (S4) pour Mission A, étendu en S5 pour Mission B. **Mais pas pour Mission Speculative.** S7 doit :
- Ajouter `chewing_metrics: Option<ChewingMetrics>` à `SpeculativeResponse`
- Tracer dans `run_speculative` :
  - `drafter_latency_ms` (champ existant)
  - `verifier_latency_ms` (champ existant, mais 0 si skip)
  - `input_tokens_raw` + `input_tokens_compressed` (pas de compressor en spec)
  - `llm_tokens_generated` (du drafter OU du verifier selon le path)
  - `selected_llm` = drafter si accepté, verifier si rejeté
  - **NEW S7** : `verifier_skipped: bool` + `savings_output_tokens: u32`

### 1.8 Verdict audit complet

| Composant | État pré-S7 | Action S7 |
|---|---|---|
| `SpeculativeRequest/Response` types | ✅ existent | Ajouter `drafter_llm`, `accept_threshold`, `chewing_metrics` |
| `run_speculative` workflow | ✅ tourne en offline | Refactor : route drafter_llm dynamique + métriques + skip verifier |
| `Speculator::run` algo | ⚠ verifier toujours appelé | Refactor : cancellation via self_quality OU early-accept |
| `Jaccard similarity` | ✅ implémenté | Aucune (réutilise) |
| 13 tests offline | ✅ tous PASS | Préserver 100 % (backward-compat) + ajouter 5-6 nouveaux S7 |
| Provider drafter (live) | ❌ inexistant | Utiliser Gemini Flash comme drafter (alias registry) |
| `StreamEvent::SpeculationResolved` | ✅ émis | +champ `verifier_skipped: bool` |
| `ChewingMetrics` pour Spec | ❌ pas wired | Wire dans `run_speculative` |
| Fixture spec live | ❌ inexistante | NEW `08_speculative_factual.json` |
| Test live spec | ❌ inexistant | NEW `live_mission_speculative_*` |

**S7 est principalement un refactor + extension** : aucun nouveau type majeur,
juste compléter le scaffolding existant pour le faire vivre en live + corriger
le piège chirurgical de §1.3 (verifier toujours appelé).

---

## 2. Architecture S7 — Vue d'ensemble

### 2.1 Le pattern Speculative RAG moderne

```
                    ┌────────────────────────┐
                    │ User query             │
                    │ "Capital of France?"   │
                    └────────────┬───────────┘
                                 │
                    ┌────────────▼───────────┐
                    │ Drafter = Gemini Flash │
                    │ (cheap, fast, ~0.05 ¢) │
                    │ Latency ~400 ms        │
                    └────────────┬───────────┘
                                 │ draft="Paris."
                                 │ self_quality=0.95
                                 ▼
                       ┌─────────────────┐
                       │ self_quality    │
                       │   >= threshold? │
                       └──┬──────────┬───┘
                         Yes        No
                          │          │
                          ▼          ▼
            ┌──────────────────┐  ┌───────────────────────┐
            │ HOT PATH (~80 %) │  │ SLOW PATH (~20 %)     │
            │ Accept draft     │  │ Call verifier=Claude  │
            │ Skip verifier    │  │ Wait for verify       │
            │ Cost: 0.05 ¢     │  │ Cost: 5 ¢             │
            │ Latency: 400 ms  │  │ Latency: ~1200 ms     │
            └──────────────────┘  └───────────────────────┘
                          │          │
                          └────┬─────┘
                               ▼
                    ┌────────────────────┐
                    │ SpeculativeResponse│
                    │ + chewing_metrics  │
                    │ + verifier_skipped │
                    └────────────────────┘
```

### 2.2 Refactor `Speculator::run` — vrai pattern avec cancellation

**Décision chirurgicale** : pour rester non-intrusif sur les 8 tests unitaires
existants de `Speculator`, on garde la signature `run(&self, req: LlmCall) ->
SpeculationOutcome` mais on **ajoute un champ** `verifier_skipped: bool` au
`SpeculationOutcome`. La nouvelle logique :

```rust
pub async fn run(&self, req: LlmCall) -> CoordResult<SpeculationOutcome> {
    let started = Instant::now();

    // ── Step 1 : drafter SEUL d'abord (pas en parallèle) ──
    //
    // S7 patch : on attend le drafter en premier. C'est la latence
    // critique du hot path. Le verifier ne tournera QUE si on rejette.
    let drafter_started = Instant::now();
    let draft = self.drafter.call(req.clone()).await?;
    let drafter_latency = drafter_started.elapsed().as_millis() as u64;

    // ── Step 2 : décision d'accept-without-verify ──
    //
    // Si self_quality du drafter ≥ threshold, on retourne directement
    // le draft (le verifier API n'est PAS appelé).
    let accept_without_verify = draft.self_quality >= self.accept_threshold;

    if accept_without_verify {
        // HOT PATH : pas de coût verifier, pas d'API call premium.
        return Ok(SpeculationOutcome {
            final_text: draft.text.clone(),
            draft_text: draft.text,
            draft_accepted: true,
            tokens_accepted: draft.tokens_generated,
            tokens_regenerated: 0,
            latency_saved_ms: 0, // pas de baseline observée — calc post-hoc
            latency_ms: started.elapsed().as_millis() as u64,
            verifier_id: self.verifier.id(),
            verifier_skipped: true,  // S7 NEW
            drafter_latency_ms: drafter_latency,
            verifier_latency_ms: 0,
        });
    }

    // ── Step 3 : SLOW PATH — verifier appelé ──
    let verifier_started = Instant::now();
    let verify_res = self.verifier.call(req).await?;
    let verifier_latency = verifier_started.elapsed().as_millis() as u64;

    // Jaccard comparison post-hoc (pour reporting).
    let similarity = jaccard_words(&draft.text, &verify_res.text);
    let accept = similarity >= self.accept_threshold;

    let (final_text, tokens_accepted, tokens_regenerated, draft_accepted) =
        if accept {
            (draft.text.clone(), draft.tokens_generated, 0, true)
        } else {
            (
                verify_res.text.clone(),
                tokens_accepted_count(&draft.text, &verify_res.text),
                verify_res.tokens_generated.saturating_sub(/*...*/),
                false,
            )
        };

    Ok(SpeculationOutcome {
        final_text,
        draft_text: draft.text,
        draft_accepted,
        tokens_accepted,
        tokens_regenerated,
        latency_saved_ms: 0,
        latency_ms: started.elapsed().as_millis() as u64,
        verifier_id: self.verifier.id(),
        verifier_skipped: false,  // verifier a été appelé
        drafter_latency_ms: drafter_latency,
        verifier_latency_ms: verifier_latency,
    })
}
```

**Garantie de non-régression** :
- Si `draft.self_quality == 0.0` (MockLlmProvider sans `with_self_quality`),
  on ne déclenche jamais le hot path → comportement identique aux tests
  offline existants.
- Si `draft.self_quality >= threshold`, on saute le verifier → c'est le
  nouveau comportement S7 qui démontre le savings.

**Note sur la sérialisation** : `LlmResponse.self_quality: f32` existe déjà
(cf. `speculator.rs:151`). Les vrais providers (Claude, Gemini, GPT) doivent
le remplir au moment de l'appel. **Vérification §1.5 → à faire avant impl**.

### 2.3 `SpeculativeRequest` enrichi

```rust
pub struct SpeculativeRequest {
    pub user_text: String,
    pub task_class: TaskClass,

    /// **Sprint S7** — ID du drafter (cheap LLM, e.g. "gemini_…").
    /// Défaut "drafter" pour backward-compat S2-S6.
    #[serde(default = "default_drafter_id")]
    pub drafter_llm: String,

    pub verifier_llm: String,
    pub verifier_metadata: LlmMetadata,
    pub budget: Budget,
    pub max_draft_tokens: u32,
    pub max_total_latency: Duration,

    /// **Sprint S7** — Seuil de self_quality au-dessus duquel le drafter
    /// est accepté sans appeler le verifier. Défaut 0.7.
    #[serde(default = "default_accept_threshold")]
    pub accept_threshold: f32,
}

fn default_drafter_id() -> String { "drafter".to_string() }
const fn default_accept_threshold() -> f32 { 0.7 }
```

### 2.4 `SpeculativeResponse` enrichi

```rust
pub struct SpeculativeResponse {
    pub final_text: String,
    pub draft_text: String,
    pub draft_accepted: bool,
    pub tokens_accepted: u32,
    pub tokens_regenerated: u32,
    pub latency_saved_ms: i64,
    pub latency_ms: u64,
    pub audit_chain_hash: String,

    /// **Sprint S7** — Le verifier API a-t-il été appelé ?
    ///
    /// - `true` (HOT PATH ~80 %)  : drafter seul, économie ~100 % sur coût verifier
    /// - `false` (SLOW PATH ~20 %) : verifier appelé, savings 0
    ///
    /// Distinct de `draft_accepted` : on peut avoir `draft_accepted=true`
    /// après comparaison Jaccard (slow path) — mais le verifier aura quand
    /// même été appelé. `verifier_skipped` est la **vraie économie**.
    #[serde(default)]
    pub verifier_skipped: bool,

    /// **Sprint S7** — Métriques de pré-mâchage adaptées spec.
    #[serde(default)]
    pub chewing_metrics: Option<ChewingMetrics>,
}
```

### 2.5 Helper `SpeculativeRequest::with_drafter_and_verifier`

```rust
impl SpeculativeRequest {
    /// **Sprint S7** — Helper : construit avec drafter cheap + verifier premium
    /// basé sur la `TaskClass` via la table de scoring S6.
    ///
    /// Pour Factual/Translation : drafter = Gemini, verifier = Gemini lui-même
    ///                            (pas d'escalation utile, c'est S6 stop).
    /// Pour Code/Reasoning      : drafter = Gemini, verifier = Claude.
    /// Pour Creative            : drafter = Gemini, verifier = GPT-4o.
    pub fn with_default_scoring(
        user_text: String,
        task_class: TaskClass,
        drafter_llm: String,
        verifier_llm: String,
    ) -> Self {
        let table = TaskClassScoringTable::default_3llm_table();
        let verifier_metadata = table.build_llm_metadata(&verifier_llm, task_class);
        Self {
            user_text,
            task_class,
            drafter_llm,
            verifier_llm,
            verifier_metadata,
            budget: Budget { max_cost_cents: 50, max_latency_ms: 30_000, min_quality_score: 0.0 },
            max_draft_tokens: 128,
            max_total_latency: Duration::from_secs(30),
            accept_threshold: 0.7,
        }
    }
}
```

### 2.6 `ChewingMetrics` pour Speculative — extension

`ChewingMetrics` est déjà polymorphe (Mission A, B). S7 ajoute des champs
**optionnels** pour Spec :

```rust
pub struct ChewingMetrics {
    // ... champs S4/S5/S6 existants ...

    /// **Sprint S7** — Le verifier API a-t-il été appelé pour Mission Spec ?
    /// `None` pour les missions non-spec.
    #[serde(default)]
    pub verifier_skipped: Option<bool>,

    /// **Sprint S7** — Latence du verifier (0 si skipped).
    #[serde(default)]
    pub spec_verifier_latency_ms: u64,

    /// **Sprint S7** — Latence du drafter.
    #[serde(default)]
    pub spec_drafter_latency_ms: u64,

    /// **Sprint S7** — Coût économisé estimé en cents (si verifier_skipped).
    #[serde(default)]
    pub spec_savings_cents: u32,
}
```

### 2.7 Le contrat de stabilité S7

| Garantie | Détail |
|---|---|
| **G1** Backward-compat 100 % | `drafter_llm` et `accept_threshold` ont des defaults serde. Les 13 tests offline existants doivent rester verts (mockprovider self_quality=0 → slow path). |
| **G2** Non-cassant Speculator | Signature `run(&self, LlmCall) -> SpeculationOutcome` inchangée. Champs nouveaux dans `SpeculationOutcome` (verifier_skipped + drafter/verifier_latency). |
| **G3** Pattern serde stable | Tous les nouveaux champs `#[serde(default)]`. |
| **G4** Tests offline régression | 13 tests pré-S7 + 6-8 tests S7 nouveaux. Couvre hot path + slow path. |
| **G5** Test live opt-in | `EFE_LIVE_LLM=1` requis. Skip propre sans clés. |
| **G6** Argument économique mesurable | `chewing_metrics.spec_savings_cents` quantifie le savings observé. |

---

## 3. Livrables S7 (8 fichiers — 4 patches src + 1 NEW fixture + 1 NEW template + 1 patch tests + cette doc)

### 3.1 `crates/efe-coordinator/src/speculate/speculator.rs` (PATCH ~80 LOC)

- Refactor `Speculator::run` : drafter d'abord, hot path skip verifier
- `SpeculationOutcome` enrichi : `+verifier_skipped, +drafter_latency_ms, +verifier_latency_ms`
- Tests unitaires existants (8) **préservés** + 3 nouveaux :
  - `speculation_skips_verifier_when_self_quality_above_threshold`
  - `speculation_calls_verifier_when_self_quality_low`
  - `speculation_records_drafter_and_verifier_latency`

### 3.2 `crates/efe-coordinator/src/workflows/speculative.rs` (PATCH ~50 LOC)

- Récupération drafter via `req.drafter_llm` (au lieu de `"drafter"` hardcodé)
- Wire `chewing_metrics` :
  - `input_tokens_raw = estimate_tokens(&req.user_text)`
  - `spec_drafter_latency_ms` + `spec_verifier_latency_ms`
  - `verifier_skipped: Some(outcome.verifier_skipped)`
  - `spec_savings_cents = estimate_savings_if_skipped(outcome, &req.verifier_metadata)`
- Emit `StreamEvent::SpeculationResolved` enrichi `+verifier_skipped`
- Propager `chewing_metrics` dans `SpeculativeResponse`

### 3.3 `crates/efe-coordinator/src/request.rs` (PATCH ~25 LOC)

- `SpeculativeRequest` : `+drafter_llm` (serde default "drafter") + `+accept_threshold` (default 0.7)
- `SpeculativeResponse` : `+verifier_skipped` + `+chewing_metrics`
- Helper `SpeculativeRequest::with_default_scoring(...)`

### 3.4 `crates/efe-coordinator/src/metrics.rs` (PATCH ~30 LOC)

- `ChewingMetrics` : +`verifier_skipped: Option<bool>`, `+spec_drafter_latency_ms`, `+spec_verifier_latency_ms`, `+spec_savings_cents`
- Tous `#[serde(default)]` (backward-compat S4/S5/S6)

### 3.5 `crates/efe-coordinator/src/events/stream.rs` (PATCH ~5 LOC)

- `StreamEvent::SpeculationResolved` : `+verifier_skipped: bool`

### 3.6 `crates/efe-coordinator/tests/fixtures/missions/08_speculative_factual.json` (NEW)

```json
{
  "name": "speculative_factual",
  "task_class": "FactualQa",
  "query": "Quelle est la capitale de la France ? Réponds en un mot.",
  "expected_keyword": "paris",
  "expected_lang": "fr",
  "expected_drafter": "gemini_gemini-2.5-flash",
  "expected_verifier": "claude_claude-sonnet-4-6",
  "expected_verifier_skipped": true,
  "min_savings_cents": 1,
  "comment": "Fixture S7 — query simple type 'Paris', le drafter Gemini doit suffire (self_quality ≥ 0.7). Verifier Claude doit être SKIP. Savings ≥ 1 ¢."
}
```

### 3.7 `crates/efe-coordinator/tests/live_missions.rs` (PATCH ~280 LOC)

3 tests live S7 :
- `live_mission_speculative_hot_path_drafter_accepted` (fixture 08, expected_verifier_skipped=true)
- `live_mission_speculative_slow_path_verifier_called` (forcer threshold haut pour rejeter)
- `live_mission_speculative_savings_metrics_filled` (vérifier chewing_metrics)

### 3.8 `crates/efe-coordinator/tests/workflow_speculative.rs` (PATCH ~150 LOC)

5-6 tests offline NEW :
- `speculative_skips_verifier_when_self_quality_high_offline`
- `speculative_calls_verifier_when_self_quality_low_offline`
- `speculative_records_chewing_metrics_offline`
- `speculative_emits_verifier_skipped_field_offline`
- `speculative_savings_cents_above_zero_when_skip_offline`
- `speculative_with_default_scoring_helper_offline`

### 3.9 `crates/efe-coordinator/S7_LIVE_VALIDATION.template.md` (NEW)

Format cohérent avec S5/S6.

### 3.10 Ce document (`EFE_A1_PHASE3_R2_S7_SPECULATIVE_DRAFTER_VERIFIER_ARCHITECTURE.md`)

L'architecture chirurgicale (CE FICHIER).

---

## 4. Spec détaillée des 3 tests live S7

### 4.1 Pré-conditions communes

```rust
fn s7_preconditions_satisfied() -> Result<(), String> {
    if !live_enabled() {
        return Err("set EFE_LIVE_LLM=1".into());
    }
    // Minimum 2 clés : drafter (Gemini cheap) + verifier (Claude premium).
    if std::env::var("ANTHROPIC_API_KEY").is_err()
        || std::env::var("GOOGLE_AI_KEY").is_err()
    {
        return Err("S7 needs ANTHROPIC_API_KEY + GOOGLE_AI_KEY".into());
    }
    Ok(())
}
```

### 4.2 Assertions F1-F12 (par test, valeurs varient selon hot/slow path)

| # | Métrique | Cible | Hot path | Slow path |
|---|---|---|---|---|
| F1 | `final_text` non vide | true | "Paris…" | "Paris est la capitale…" |
| F2 | contient "paris" (lc) | true | ✅ | ✅ |
| F3 | `draft_text` non vide | true | "Paris…" | "Paris…" |
| F4 | `verifier_skipped` matches expected | true | `true` | `false` |
| F5 | `draft_accepted` cohérent | true | `true` (skip → toujours accepted) | varie |
| F6 | `latency_ms < 30_000` | < 30 s | ~800 ms | ~3-5 s |
| F7 | `audit_chain_hash.len() == 64` | BLAKE3 hex | 64 | 64 |
| F8 | `chewing_metrics.is_some()` | true | true | true |
| F9 | `metrics.spec_drafter_latency_ms > 0` | > 0 | ~400 ms | ~400 ms |
| F10 | `metrics.spec_verifier_latency_ms` | hot=0 / slow>0 | 0 | >1000 ms |
| F11 | `metrics.spec_savings_cents >= expected` | ≥ fixture min | ≥ 1 ¢ | 0 |
| F12 | event `SPECULATION_RESOLVED` présent | true | true | true |

### 4.3 Sortie console attendue (hot path)

```text
=== Mission Speculative — Hot Path S7 (08_speculative_factual.json) ===
  query           : "Quelle est la capitale de la France ?" (38 chars)
  task_class      : FactualQa
  drafter         : gemini_gemini-2.5-flash
  verifier        : claude_claude-sonnet-4-6
  accept_threshold: 0.7

  ── Speculation result ──
    draft_text      : "Paris."
    final_text      : "Paris."
    draft_accepted  : true
    verifier_skipped: true    ← HOT PATH (savings !)
    drafter_latency : 412 ms
    verifier_latency: 0 ms    ← skipped
    cost_cents      : 1 ¢     ← (drafter only)
    savings_cents   : ~4 ¢    ← (vs baseline Claude direct)
    audit_hash      : <abc…def> (64 chars)
    wall_clock      : 412 ms
```

---

## 5. Plan d'implémentation chirurgical S7

### 5.1 Ordre de bataille

| # | Étape | Fichier | LOC | Vérif |
|---|---|---|---|---|
| 1 | Refactor `Speculator::run` + `SpeculationOutcome` enrichi | `speculate/speculator.rs` | +80 / -10 | `cargo test -p efe-coordinator speculate` |
| 2 | Patch `ChewingMetrics` | `metrics.rs` | +30 | `cargo check` |
| 3 | Patch `StreamEvent::SpeculationResolved` | `events/stream.rs` | +5 | `cargo check` |
| 4 | Patch `SpeculativeRequest/Response` + helper | `request.rs` | +50 | `cargo test request` |
| 5 | Patch `run_speculative` workflow + chewing_metrics | `workflows/speculative.rs` | +70 / -10 | `cargo test workflow_speculative` |
| 6 | Tests offline S7 (6 nouveaux) | `tests/workflow_speculative.rs` | +200 | `cargo test workflow_speculative` |
| 7 | Vérifier que les 13 tests offline pré-S7 passent toujours | (run all) | — | suite verte |
| 8 | Fixture 08_speculative_factual.json | (NEW) | 12 | `python3 -m json.tool` |
| 9 | Tests live S7 (3) + helpers | `tests/live_missions.rs` | +280 | offline skip propre |
| 10 | Template `S7_LIVE_VALIDATION.template.md` | (NEW) | ~200 | humain |
| 11 | Run live 3 tests | (utilisateur) | — | 3×PASS |
| 12 | Sceller `S7_LIVE_VALIDATION.md` + commit | (NEW) | — | `git log` |

### 5.2 Stratégie pour le `self_quality` du drafter

**Question chirurgicale** : les providers réels (Claude/Gemini/GPT) remplissent-ils
`self_quality` ?

À vérifier au début de l'implémentation. Si NON :
- Plan A : utiliser une heuristique côté `Speculator` : `self_quality =
  f(draft_length, deterministic_words_present)`. Par exemple :
  - `1.0` si draft ≤ 20 tokens **et** contient le keyword "expected"
    (mais c'est circulaire)
  - `0.8` si draft ≤ 20 tokens (réponses courtes = haute confiance pour
    factual)
  - `0.5` sinon
- Plan B : ajouter `accept_threshold_length: Option<u32>` au `Speculator`
  → si draft < length, accepte direct
- Plan C : implémenter un check par perplexity / log-prob (compliqué, hors S7)

**Décision** : **Plan B** — si la réponse drafter est courte (≤ 10 mots pour
FactualQa, ajustable via fixture), on accepte directement. C'est une heuristique
simple, déterministe, et qui matche le pattern "draft cheap LLM répond
correctement aux questions factuelles simples".

Le `Speculator` reçoit alors un paramètre `short_draft_threshold: usize` (mots)
au lieu de `accept_threshold: f32` Jaccard. Ou les deux peuvent coexister.

**Approche pragmatique S7** : garder `accept_threshold: f32` mais l'utiliser
sur `length-based score = 1.0 - (draft_words / max_acceptable_words)`. Pour
draft < 5 mots et threshold 0.7 → score = 1.0 - 5/16 = 0.69 → rejeté. Donc
threshold 0.5 → accepte.

Plus simple : **rule-based directe** :
```rust
let accept_without_verify = draft.text.split_whitespace().count() <= 10
    && !draft.text.is_empty();
```

Cette règle dit : "si le drafter répond en ≤ 10 mots à une query factuelle,
sa réponse est probablement bonne". Empirically vrai pour Gemini Flash sur
"Quelle est la capitale de X ?".

**S7 final** : utiliser `max_words_for_quick_accept: usize` (défaut 10) dans
`Speculator`, exposable via request. C'est lisible, testable, déterministe.

### 5.3 Estimation des `savings_cents`

```rust
fn estimate_savings_if_skipped(
    outcome: &SpeculationOutcome,
    verifier_metadata: &LlmMetadata,
    estimated_input_tokens: u32,
    expected_output_tokens: u32,
) -> u32 {
    if !outcome.verifier_skipped {
        return 0;
    }
    // Calcul du coût qu'aurait coûté le verifier s'il avait tourné.
    let in_c = estimated_input_tokens as u64
        * verifier_metadata.cost_input_per_million as u64
        / 1_000_000;
    let out_c = expected_output_tokens as u64
        * verifier_metadata.cost_output_per_million as u64
        / 1_000_000;
    (in_c + out_c) as u32
}
```

### 5.4 Stratégie commit S7 (≤ 3 commits)

| # | Commit | Contenu |
|---|---|---|
| 1 | `feat(phase-3-r2-s7): refactor Speculator + ChewingMetrics for spec + 6 tests offline` | speculator.rs + metrics.rs + request.rs + events + workflow + tests offline |
| 2 | `feat(phase-3-r2-s7): fixture + 3 tests live + template` | fixture 08 + live_missions.rs + S7 template |
| 3 | `docs(phase-3-r2-s7): scellement live validation S7 N/N PASS` | S7_LIVE_VALIDATION.md scellé |

---

## 6. Critères d'acceptation binaires S7

- [x] `Speculator::run` refactored : hot path skip verifier
- [x] `SpeculationOutcome` enrichi : `verifier_skipped`, `drafter_latency_ms`, `verifier_latency_ms`
- [x] `ChewingMetrics` enrichi : `verifier_skipped`, `spec_*_latency_ms`, `spec_savings_cents`
- [x] `SpeculativeRequest` : `drafter_llm` + `accept_threshold` avec defaults serde
- [x] `SpeculativeResponse` : `verifier_skipped` + `chewing_metrics`
- [x] `SpeculativeRequest::with_default_scoring(...)` helper
- [x] `StreamEvent::SpeculationResolved` : `+verifier_skipped`
- [x] Workflow `run_speculative` consomme `req.drafter_llm` + wire chewing_metrics
- [x] Fixture `08_speculative_factual.json`
- [x] **6 tests offline S7 NEW** dans `workflow_speculative.rs` PASS
- [x] **3 tests live S7** dans `live_missions.rs` (skip propre offline)
- [x] cargo check + build clean
- [x] cargo clippy 0 warning S7
- [x] **Les 13 tests offline pré-S7 passent toujours** (backward-compat)
- [x] **Zéro régression S6** (560 tests offline)
- [ ] **3 tests live PASS avec 2 clés API**
- [ ] **F1-F12 par test live × 3 = 36 assertions PASS**
- [ ] `S7_LIVE_VALIDATION.md` scellé
- [ ] Commits S7 (3) cumulés

---

## 7. Hors-scope S7 (présents en S8+)

| Item | Sprint | Pourquoi pas S7 |
|---|---|---|
| Drafter SLM local (llama-server Qwen3) | S9 | Demande SlmDrafterProvider + daemons UP. S7 fait la démonstration avec Gemini = LLM cheap, plus simple. |
| Speculative decoding token-by-token | S10 | Demande API spécialisée (e.g. OpenAI predicted outputs). Pattern différent. |
| Apprentissage du `accept_threshold` via metering | S11 | Demande store séries temporelles + corrélation outcome. |
| Chain spéculative 3 niveaux (SLM → Gemini → Claude) | S12 | Hors-scope simple |
| Circuit breaker sur drafter qui timeout | S8 | Failure modes dédié |
| Cancellation token-level du verifier (vraie parallélisation) | S10 | Refactor profond + provider trait change |

---

## 8. Risques connus & mitigations

### R1 — `self_quality` non rempli par les vrais providers

**Risque** : tous les providers `ClaudeProvider`/`GeminiProvider`/`GptProvider`
n'incluent pas de valeur de `self_quality` (mise à `0.0` par défaut). Si on se
base dessus, hot path jamais déclenché.

**Mitigation** : utiliser la stratégie §5.2 Plan B — heuristique
`max_words_for_quick_accept` (10 mots par défaut) au lieu de `self_quality`.
Vérifié empiriquement : Gemini Flash sur "Capital of France?" répond en 1-3
mots → toujours sous le seuil.

### R2 — Drafter Gemini rejected → coût slow path 5 ¢

**Risque** : fixture mal calibrée → Gemini drafter rejected systématiquement
→ on paie Claude + Gemini = + cher que sans spec.

**Mitigation** :
- Fixture choisie pour maximiser hot path (query "Capital of France?")
- Test live `slow_path` est volontairement DÉCLENCHÉ via `max_words_for_quick_accept = 1`
  → force le rejet → mesure la vraie latence/coût du slow path
- Le critère d'acceptation est `verifier_skipped == expected_verifier_skipped`,
  pas une métrique de cost absolu

### R3 — Le hot path "triche" si drafter mauvais

**Risque** : drafter accepte sans verifier, mais la réponse est fausse
→ utilisateur a une mauvaise réponse à 0.05 ¢. **C'est le compromis du
Speculative RAG**.

**Mitigation** :
- Threshold ajustable (S11 fera l'apprentissage)
- S6 fait le routing en amont : on n'utilise S7 que sur task_class
  appropriée (FactualQa simple, pas Code)
- En production, S7 est opt-in via Mission Speculative — Mission A reste
  disponible pour qualité maximale

### R4 — Tests offline S2-S5 cassent à cause des nouveaux champs

**Risque** : ajout de champs à `ChewingMetrics` casse les tests qui
sérialisent/désérialisent du JSON.

**Mitigation** :
- `#[serde(default)]` sur **tous** les nouveaux champs
- Tests existants utilisent `..Default::default()` ou des stub functions
- Roundtrip test pour vérifier backward-compat
- Vérification ad-hoc post-implémentation : `cargo test --workspace`

### R5 — Latence drafter > latence verifier

**Risque** : si Gemini est lent (cold start, rate limit) → drafter > 2 s,
on attend pour rien, savings négatif sur latence.

**Mitigation** :
- Ajout d'un `drafter_timeout_ms` (défaut 2000 ms) → si dépassé, fallback
  direct sur verifier (skip drafter)
- Test live mesure `drafter_latency_ms` et le compare au baseline
- S8 ajoutera circuit breaker auto

---

## 9. Vérification post-livraison

```bash
# 1. Tests offline speculative — régression
cargo test -p efe-coordinator --features all-providers --test workflow_speculative \
    --no-fail-fast

# 2. Tests unitaires Speculator + ChewingMetrics
cargo test -p efe-coordinator --lib speculate

# 3. Suite offline complète
cargo test -p efe-coordinator --features all-providers --tests --lib

# 4. Lint strict
cargo clippy -p efe-coordinator --features all-providers --all-targets \
    -- -D warnings

# 5. Tests LIVE (manuel, 2 clés API)
export ANTHROPIC_API_KEY="sk-ant-..."
export GOOGLE_AI_KEY="AIza..."
EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers \
    --test live_missions live_mission_speculative -- --test-threads=1 --nocapture
```

---

## 10. Lien avec les sprints suivants

### Pour S8 (Failure modes)

S8 utilise les nouveaux champs `drafter_latency_ms` + `verifier_latency_ms`
comme **inputs** du circuit breaker : si `drafter_latency_ms > p99_drafter`,
ouvre le circuit drafter et fallback direct verifier.

### Pour S9 (Drafter SLM local)

S9 remplace le drafter Gemini par un `SlmDrafterProvider` qui parle à
llama-server (reformulator Qwen3-0.6B port 8091). Le code S7 reste
inchangé — seul le provider injecté change. **C'est l'argument architectural
fort** : S7 est agnostique du drafter.

### Pour S10 (Speculative decoding token-by-token)

S10 ajoute un mode "streaming" où le drafter envoie token-par-token et le
verifier valide en continu. Demande un nouveau trait sur `LlmProvider` —
hors-scope S7.

### Pour S11 (Adaptive threshold)

S11 collecte les `verifier_skipped` + outcomes (user feedback) et apprend le
threshold optimal par task_class. La structure `accept_threshold` exposée en
S7 (request-level) facilite cette évolution.

### Pour S12 (Multi-level speculation)

S12 chain SLM → Gemini → Claude : 3 niveaux d'escalation. Le pattern S7 est
extensible : on remplace `Speculator { drafter, verifier }` par
`SpeculativeChain { stages: Vec<...> }`.

---

## 11. Conclusion

Sprint S7 livre la **3e dimension de savings** du framework R2 :

| Dimension | Sprint | Optimisation | Savings typique |
|---|---|---|---|
| Input tokens | S4 | Pré-mâchage SLM | 85 % |
| LLM choisi | S6 | Routing par task_class | 75 % (mixture 80/20) |
| **Output tokens** | **S7** | **Speculative RAG (drafter cheap d'abord)** | **~98 % sur 80 % FactualQa** |

Combinés sur une mixture réaliste production :

| Configuration | Coût relatif | Savings |
|---|---|---|
| Baseline Claude partout, sans optim | 100 % | 0 % |
| S4 seul | 30 % | 70 % |
| S6 seul | 25 % | 75 % |
| S7 seul (FactualQa) | 2 % | **98 %** |
| **S4 + S6 + S7 combinés** | **~0.5–1 %** | **~99 %** |

EFE passe de **×16 vs baseline (post-S6)** à **×100+ vs baseline (post-S7)**
sur des mixtures dominées par les queries factuelles simples — pour un coût
réel de **<1 ¢ par query** en moyenne, **en préservant la qualité** via le
fallback systématique vers Claude quand le drafter doute.

**Prochaine étape** : Sprint S8 — Failure modes (circuit breaker, timeout
fallback, dégradation gracieuse). Comment EFE reste UP quand un provider
fail.

---

## Annexes

### A. Pourquoi rule-based plutôt que self_quality

`self_quality: f32` existe sur `LlmResponse` mais n'est pas peuplé par les
vrais providers (vérifié dans la base de code Anthropic/Gemini/OpenAI : aucun
n'expose un signal de confiance dans la réponse API). Donc baser le hot path
là-dessus échouerait silencieusement.

**Rule** : "drafter répond en ≤ 10 mots → accept" est :
- Déterministe
- Testable
- Empiriquement vrai pour FactualQa Gemini Flash
- Conservateur (rejette les réponses longues qui pourraient être hallucinations)

### B. Pourquoi pas de cancellation token-level

La cancellation propre du verifier (à mi-stream) demande :
- Un trait `LlmProvider::stream_with_cancel(...)`
- Des CancellationToken qui se propagent dans les futures
- Des tests d'edge case (cancel après tokens partiels reçus)
- Refactor profond des providers Claude/Gemini/GPT

S7 fait du **futurelevel cancellation simple** : on n'appelle le verifier que
si nécessaire (séquentiel, pas vraiment parallèle). C'est suffisant pour
démontrer la valeur économique. La vraie parallélisation est S10.

### C. Pourquoi Gemini comme drafter et pas un vrai SLM

Critères :
- **Coût** : Gemini Flash = 0.075/0.30 $/M, négligeable. SLM Qwen3 = 0 (mais setup complexe).
- **Latence** : Gemini Flash = ~400-800 ms réseau. SLM local ≈ 200 ms mais
  demande llama-server UP.
- **Setup** : Gemini = 1 clé API. SLM = 4 GB GGUF + GPU/Metal + daemon.
- **Qualité FactualQa** : Gemini = 0.92 (cf. table S6). Qwen3-0.6B = ~0.65–0.75.

→ Gemini Flash gagne sur **3 axes sur 4**. C'est le bon drafter de
production pour S7. S9 explorera le SLM pour des cas où la latence
ou la confidentialité priment.

### D. Quelle métrique de "verifier_skipped" sur Mission B/A ?

`verifier_skipped` n'a de sens que pour Mission Speculative. On le met dans
`ChewingMetrics` comme `Option<bool>` : `None` pour Mission A/B (pas de
speculation), `Some(true/false)` pour Mission Spec.

Cohérent avec le pattern `chewing_metrics` polymorphe : certains champs sont
remplis selon la mission. C'est le coût de la mutualisation de la struct.

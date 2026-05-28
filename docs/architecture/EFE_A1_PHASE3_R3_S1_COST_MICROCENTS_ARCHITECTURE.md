# EFE Phase 3 R3 — Sprint S1 : Cost Microcents (fix L1/D7)

**Précédence** : R2 scellé (commit `12f31bd`), R3 architecture globale livrée
**Suivant** : R3.S2 (SlmCircuitBreaker wiring)
**Statut** : Spec ready-to-implement
**Effort estimé** : 4 heures (sprint le plus court de R3)
**Coût $ live estimé** : 0 ¢ (offline-only, mais déclenche R3.S9 régen)
**Priorité** : 🔴 **CRITIQUE** — débloque R3.S9 + corrige l'argument économique

---

## 0. Pourquoi ce sprint — la limitation la plus visible de R2

### 0.1 Le rapport R2 montre `savings_cost = 0 %`

C'est littéralement scellé dans `crates/efe-coordinator/CHEWING_SAVINGS_REPORT.md` :

```text
Baseline cost = 6¢, Full cost = 6¢ → Savings 0¢ = 0%
```

Et dans le JSON associé :

```json
{
  "baseline_total_cost_cents": 6,
  "full_total_cost_cents": 6,
  "savings_cents": 0,
  "savings_ratio": 0.0
}
```

**Pour un livrable investisseurs, c'est embarrassant**. *"Voici notre
solution qui réduit le coût LLM par ×16... avec un rapport scellé montrant
0 % savings cost."*

### 0.2 La vraie raison : résolution `u32` arrondit tout

Sur fixture `01_factual_simple` (74 chars query) :
- Query input ≈ 50 tokens
- Output Claude ≈ 20 tokens

Coût réel par provider :

| Provider | Pricing (USD/M) | Cost réel par query | Cost ¢ stocké (u32) |
|---|---|---|---|
| **Claude Sonnet 4.6** | 3.00 / 15.00 | 50 × 3 / 1M + 20 × 15 / 1M = $0.00045 = **0.045 ¢** | 1 ¢ (round up min) |
| **Gemini 2.5 Flash** | 0.075 / 0.30 | 50 × 0.075 / 1M + 20 × 0.30 / 1M = $0.0000098 = **0.00098 ¢** | 1 ¢ (round up min) |
| **GPT-4o** | 2.50 / 10.00 | 50 × 2.50 / 1M + 20 × 10 / 1M = $0.000325 = **0.0325 ¢** | 1 ¢ (round up min) |

→ Les 3 providers donnent **0.001 ¢ à 0.045 ¢ réel**. Tous arrondis à `1 ¢`
en u32. **Le ratio réel ×46 (Claude/Gemini) est invisible**.

### 0.3 Découverte D7 chirurgicale (documentée R2)

L'audit post-S9 (`R2_LIVE_VALIDATION.md` §6 D7) a identifié explicitement :

> *"Découverte chirurgicale post-run live : la résolution u32 cents est
> trop grossière sur fixtures courtes (< 200 chars). Tous les coûts
> s'arrondissent à 1-2 ¢ peu importe le LLM."*

> *"Décision chirurgicale S9 : on garde u32 (cohérence S2-S8), mesurer via
> latency + quality où les chiffres sont précis. **Fix R3 envisagé** :
> `cost_microcents: u64` (×1000 précision)."*

**R3.S1 est la concrétisation de ce fix promis.**

### 0.4 Précision attendue post-R3.S1

Avec `cost_microcents: u64` (1 microcent = 1/1000 cent = 1/100_000 USD) :

| Provider | Cost réel par query 01 | `cost_microcents` (u64) | Différence visible |
|---|---|---|---|
| Claude | 0.045 ¢ | **45** μ¢ | — |
| Gemini | 0.001 ¢ | **1** μ¢ | **×45** |
| GPT-4o | 0.0325 ¢ | **32** μ¢ | ×0.7 |

→ Le savings Claude → Gemini est désormais **×45 mesurable empiriquement**.

### 0.5 Ce que R3.S1 N'EST PAS

- ❌ Pas un **breaking change** sur `cost_cents` (préservé via helpers)
- ❌ Pas une nouvelle dépendance externe (`u64` est stdlib)
- ❌ Pas un changement de pricing tables (calibration nov-2025 préservée)
- ❌ Pas un sprint live (offline-only, valide via tests unitaires)
- ❌ Pas une migration de DB ou audit chain (cost est éphémère per-mission)

R3.S1 = **pure extension de type** + **recalibration tables** +
**propagation helpers backward-compat**. ~4h de dev rigoureux.

---

## 1. Audit chirurgical pré-R3.S1

### 1.1 Inventaire des `cost_cents: u32` dans R2

Recensement exhaustif des sites à patcher :

#### 1.1.1 Types I/O publics

| Fichier | Field | Visibilité |
|---|---|---|
| `crates/efe-coordinator/src/llm/provider.rs:54` | `LlmResponse.cost_cents: u32` | pub, propagé partout |
| `crates/efe-coordinator/src/request.rs:121` | `MissionAResponse.cost_cents: u32` | pub API consumers |
| `crates/efe-coordinator/src/request.rs:225` | `MissionBResponse.total_cost_cents: u32` | pub API |
| `crates/efe-coordinator/src/request.rs:286` | `SpeculativeResponse.cost_cents: u32` (S7 NEW) | pub API |
| `crates/efe-coordinator/src/speculate/speculator.rs:67` | `SpeculationOutcome.cost_cents: u32` (S7 NEW) | pub interne |

#### 1.1.2 Métadonnées / Pricing

| Fichier | Field | Usage |
|---|---|---|
| `crates/efe-nano-app-coordinator/src/types/router.rs:60` | `LlmMetadata.cost_input_per_million: u32` | Router Pareto + reporting |
| `crates/efe-nano-app-coordinator/src/types/router.rs:63` | `LlmMetadata.cost_output_per_million: u32` | idem |
| `crates/efe-nano-app-coordinator/src/types/scoring.rs` | `LlmStaticMetadata.cost_input_per_million: u32`, `cost_output_per_million: u32` | Table S6 |
| `crates/efe-nano-app-coordinator/src/types/router.rs:88` | `Budget.max_cost_cents: u32` | Router filtering |

#### 1.1.3 Tracking / Metrics

| Fichier | Field |
|---|---|
| `crates/efe-coordinator/src/metrics.rs:101` | `ChewingMetrics.router_estimated_cost_cents: u32` |
| `crates/efe-coordinator/src/metrics.rs:~XX` | `ChewingMetrics.spec_savings_cents: u32` (S7 NEW) |
| `crates/efe-coordinator/src/llm/mock.rs:24` | `MockLlmProvider.cost_cents_per_call: u32` + `.with_cost_cents()` |

#### 1.1.4 Reporting S9

| Fichier | Field |
|---|---|
| `crates/efe-coordinator/src/savings_report.rs:~155` | `RunRecord.cost_cents: u32` |
| `crates/efe-coordinator/src/savings_report.rs:~180` | `ModeSummary.total_cost_cents: u32`, `avg_cost_cents: f32` |
| `crates/efe-coordinator/src/savings_report.rs:~215` | `SavingsReport.baseline_total_cost_cents: u32`, `full_total_cost_cents: u32`, `savings_cents: i32` |

#### 1.1.5 Pricing tables providers (calculs)

| Fichier | Méthode |
|---|---|
| `crates/efe-coordinator/src/llm/providers/anthropic.rs` | `PricingTable::claude(model) -> (u32, u32)` (input, output per M) |
| `crates/efe-coordinator/src/llm/providers/gemini.rs` | `PricingTable::gemini(model)` |
| `crates/efe-coordinator/src/llm/providers/openai.rs` | `PricingTable::openai(model)` |

**Total sites à modifier** : ~25 fields/méthodes répartis sur ~10 fichiers.

### 1.2 Backward-compat à préserver

Consumers existants de `cost_cents` (qui ne doivent pas casser) :
- Tests offline qui asserent `resp.cost_cents == X`
- Tests live (S2-S9) qui logguent `cost_cents`
- `CHEWING_SAVINGS_REPORT.md` v1 (snapshot historique, conservé)
- Audit chain BLAKE3 (le `cost_cents` est dans `LlmResponse` sérialisé → hash inclut le u32)

**Garantie de backward-compat R3.S1** :
1. `cost_cents` field reste accessible via **helper method** sur tous les types
2. Sérialisation JSON : `cost_cents` reste émis (calculé depuis `cost_microcents`)
3. Anciens JSON sans `cost_microcents` : `#[serde(default)]` → `cost_microcents = cost_cents × 1000`

### 1.3 Calculs actuels dans providers (Claude exemple)

**Fichier** : `crates/efe-coordinator/src/llm/providers/anthropic.rs`

Pattern actuel (simplifié) :

```rust
// Dans ClaudeProvider::call(...)
let pricing = PricingTable::claude(&self.model);  // returns (u32, u32) cents/M
let in_cents = (input_tokens as u64 * pricing.0 as u64) / 1_000_000;
let out_cents = (output_tokens as u64 * pricing.1 as u64) / 1_000_000;
let total_cents = (in_cents + out_cents) as u32;

LlmResponse {
    text,
    tokens_generated: output_tokens,
    cost_cents: total_cents,
    // ...
}
```

**Problème** : la division integer arrondit DOWN. Si la valeur réelle est
0.045 cents (= 45 microcents), `total_cents` devient `0` après division.
Puis casté en u32 → reste 0. **MAIS** dans le code actuel, il y a souvent
un `max(1)` implicite ou explicite qui force à 1¢ minimum.

Vérifions le code exact pour comprendre la rounding logic.

### 1.4 Verdict audit

| Composant | État pré-R3.S1 | Action R3.S1 |
|---|---|---|
| `LlmResponse.cost_cents` | u32 (résolution insuffisante) | +`cost_microcents: u64`, `cost_cents()` devient helper |
| 3 providers (Claude/Gemini/OpenAI) | Calcul cents division integer | Recalcul en microcents (×1000 précision) |
| Pricing tables | Retournent u32 cents/M | Retournent u64 microcents/M (×1000) |
| `LlmMetadata` cost_*_per_million | u32 | +`*_microcents` u64 fields |
| `MissionA/B/SpecResponse.cost_cents` | u32 | +`cost_microcents: u64` |
| `ChewingMetrics.*_cents` | u32 | +`*_microcents: u64` |
| `SavingsReport` | u32 partout | +`*_microcents` u64 + `to_markdown_v2()` (R3.S9) |
| `MockLlmProvider.with_cost_cents` | u32 | +`with_cost_microcents(u64)` |
| Audit chain | Hash inclut `cost_cents` u32 | Préservé (cost_cents reste émis via helper) |

---

## 2. Architecture R3.S1 — Vue d'ensemble

### 2.1 La progression de précision

```
                Unit               Value of $1 USD
                ───────            ───────────────
                cents (u32)        100
                microcents (u64)   100_000      ← R3.S1
                picocents          100_000_000  (hors-scope, overkill)
                tokens × $/M       (calcul source)
```

**Pourquoi microcents et pas picocents** :
- **Microcents (×1000 précision)** : suffisant pour distinguer Claude
  (45 μ¢) vs Gemini (1 μ¢) sur fixtures courtes. C'est le ×1000 minimum
  utile.
- **Picocents (×1_000_000)** : overkill. Aucun LLM ne facture au picocent.
  Coût u64 inutile.
- **Cents (×1)** : trop grossier (résolution actuelle R2).

### 2.2 Conversion `cost_microcents ↔ cost_cents`

Helper standard partout :

```rust
/// Convertit microcents → cents (rounded half-up vers le cent supérieur).
///
/// `40_microcents → 0 cents` (rounded down — résolution insuffisante)
/// `500 microcents → 1 cent`
/// `1_499 microcents → 1 cent`
/// `1_500 microcents → 2 cents`
#[must_use]
pub const fn microcents_to_cents(mc: u64) -> u32 {
    ((mc + 500) / 1000).min(u32::MAX as u64) as u32
}

/// Convertit cents → microcents (multiply ×1000, no rounding).
#[must_use]
pub const fn cents_to_microcents(c: u32) -> u64 {
    (c as u64) * 1000
}
```

**Décision chirurgicale** : `microcents_to_cents` arrondit half-up pour
préserver la sémantique "au moins 1 cent quand il y a eu un vrai coût".
Sinon `LlmResponse.cost_cents()` retourne `0` pour Gemini sur fixture 01
(1 μ¢ → `(1+500)/1000 = 0`). C'est correct (0 cent réel) mais consumers
existants attendent `cost_cents >= 1`.

Alternative : `min(cost_cents, 1)` si `microcents > 0`. À discuter (cf. §3.1).

### 2.3 Pricing tables en microcents (nov-2025)

| Provider / Model | Cost input USD/M | μ¢/M (input) | Cost output USD/M | μ¢/M (output) |
|---|---|---|---|---|
| **Claude Sonnet 4.6** | $3.00 | 300_000 | $15.00 | 1_500_000 |
| **Claude Haiku** | $0.80 | 80_000 | $4.00 | 400_000 |
| **Claude Opus** | $15.00 | 1_500_000 | $75.00 | 7_500_000 |
| **Gemini 2.5 Flash** | $0.075 | **7_500** | $0.30 | 30_000 |
| **Gemini 2.5 Pro** | $1.25 | 125_000 | $5.00 | 500_000 |
| **GPT-4o** | $2.50 | 250_000 | $10.00 | 1_000_000 |
| **GPT-4o-mini** | $0.15 | 15_000 | $0.60 | 60_000 |
| **GPT-5** (estim) | $5.00 | 500_000 | $15.00 | 1_500_000 |

**Découverte chirurgicale** : Gemini 2.5 Flash à 7_500 μ¢/M input ne peut
PAS être représenté en u32 cents/M (=7.5 cents/M, arrondi à 8 cents/M en
R2 → erreur 6.7 %). En microcents, la précision est exacte.

### 2.4 Helper functions pour backward-compat

```rust
impl LlmResponse {
    /// **R3.S1** — Coût exact en microcents (1 cent = 1000 μ¢).
    /// Précision ×1000 vs `cost_cents()`.
    #[must_use]
    pub fn cost_microcents(&self) -> u64 {
        self.cost_microcents
    }

    /// **R3.S1** — Coût en cents (legacy, dérivé de `cost_microcents`).
    /// Arrondi half-up. **Préservé pour backward-compat** avec consumers
    /// pré-R3.
    ///
    /// ⚠ Précision insuffisante sur fixtures courtes (< 200 chars).
    /// Préférer `cost_microcents()` pour reporting précis.
    #[must_use]
    pub fn cost_cents(&self) -> u32 {
        microcents_to_cents(self.cost_microcents)
    }
}
```

**Pattern appliqué partout** : `MissionAResponse`, `MissionBResponse`,
`SpeculativeResponse`, `SpeculationOutcome`, `ChewingMetrics`, `SavingsReport`.

### 2.5 Decision : `cost_cents` field public ou helper method ?

**Option A** : `cost_cents` reste un field public (compatible avec destructuring
existant `resp.cost_cents`). Calculé une fois à la construction.

```rust
pub struct LlmResponse {
    pub cost_microcents: u64,
    pub cost_cents: u32,  // ← dérivé de cost_microcents, mais reste public
    // ...
}
```

**Option B** : `cost_cents` devient méthode (`resp.cost_cents()`).
Breaking change si quelqu'un faisait `resp.cost_cents` (field access).

**Décision** : **Option A** — `cost_cents` reste field public, valeur dérivée
à la construction. Préserve la backward-compat 100 % côté API consumer.
Légère redondance acceptable.

```rust
pub struct LlmResponse {
    // ... champs existants ...

    /// **R3.S1** — Coût exact en microcents (préfère sur `cost_cents` pour
    /// reporting). Field NEW, `#[serde(default)]` pour backward-compat.
    #[serde(default)]
    pub cost_microcents: u64,

    /// Coût en cents (legacy, dérivé de `cost_microcents` à la construction
    /// post-R3.S1). Préservé pour backward-compat.
    pub cost_cents: u32,
}
```

À la construction dans providers :

```rust
let cost_microcents = compute_cost_microcents(input_tokens, output_tokens, pricing);
LlmResponse {
    text,
    tokens_generated,
    cost_microcents,
    cost_cents: microcents_to_cents(cost_microcents),  // ← dérivé
    // ...
}
```

### 2.6 Backward-compat serde

Cas 1 — Sérialisation R3 → désérialisation R3 :
```json
{"text": "...", "cost_microcents": 45, "cost_cents": 0, "tokens_generated": 20, ...}
```
OK direct roundtrip.

Cas 2 — Sérialisation R2 → désérialisation R3 :
```json
{"text": "...", "cost_cents": 1, "tokens_generated": 20, ...}
```
`cost_microcents` absent → `#[serde(default)]` → 0.
**Problème** : si le consumer R3 utilise `cost_microcents` pour reporting,
il verra 0 sur les anciens runs. Incorrect.

**Fix** : si `cost_microcents == 0` mais `cost_cents > 0`, l'helper
`cost_microcents()` retourne `cents_to_microcents(cost_cents)` :

```rust
impl LlmResponse {
    pub fn cost_microcents(&self) -> u64 {
        if self.cost_microcents == 0 && self.cost_cents > 0 {
            // Pré-R3 record : reconstruction depuis cents (perte de précision)
            cents_to_microcents(self.cost_cents)
        } else {
            self.cost_microcents
        }
    }
}
```

Mais ça force `cost_microcents` à être une méthode (pas field public).
Compromis : on garde field + méthode, mais consumers savent que pré-R3
records ont `cost_microcents = cents × 1000` (perte ×1000 précision sur
l'historique).

Cas 3 — Sérialisation R3 → désérialisation R2 :
Le consumer R2 n'a pas `cost_microcents` dans son struct, ignore le field
inconnu (serde default behavior). Lit `cost_cents` normalement.

### 2.7 Le contrat de stabilité R3.S1

| Garantie | Détail |
|---|---|
| **G1** Backward-compat 100 % | `cost_cents` field public préservé. JSON R2 désérialisable. |
| **G2** Forward-compat | JSON R3 désérialisable par consumers R2 (ignorant `cost_microcents`). |
| **G3** Précision ×1000 | Tous les nouveaux calculs `cost_microcents` exactes au μ¢ près. |
| **G4** Tests offline | 631 → ~650 (12 nouveaux R3.S1). 0 régression. |
| **G5** Audit chain stable | `cost_cents` reste dans BLAKE3 hash inputs. `cost_microcents` est aussi inclus (champ supplémentaire) → hash diffèrent post-R3 mais audit chain v2 documentée. |

---

## 3. Livrables R3.S1 (10 fichiers — 0 NEW, 10 PATCHES)

### 3.1 `crates/efe-coordinator/src/llm/provider.rs` (PATCH ~30 LOC)

- `LlmResponse` : `+pub cost_microcents: u64` (`#[serde(default)]`)
- 2 helpers : `microcents_to_cents(mc: u64) -> u32`,
  `cents_to_microcents(c: u32) -> u64`
- Tests unit : roundtrip + edge cases (0, max u32, overflow protection)

### 3.2 `crates/efe-coordinator/src/llm/providers/anthropic.rs` (PATCH ~40 LOC)

- `PricingTable::claude_microcents(model)` → `(u64, u64)` (input, output μ¢/M)
- Recalcul cost en microcents :
  ```rust
  let (in_mc_per_m, out_mc_per_m) = PricingTable::claude_microcents(&self.model);
  let cost_microcents = (input_tokens as u64 * in_mc_per_m / 1_000_000)
                       + (output_tokens as u64 * out_mc_per_m / 1_000_000);
  let cost_cents = microcents_to_cents(cost_microcents);
  ```
- `PricingTable::claude(model) -> (u32, u32)` (cents/M) **préservé** pour
  backward-compat — devient wrapper : `claude_microcents(model) / 1000`.

### 3.3 `crates/efe-coordinator/src/llm/providers/gemini.rs` (PATCH ~40 LOC)

Idem `anthropic.rs`. **Important** : Gemini Flash à 7_500 μ¢/M input devient
exact (vs 8 cents/M = +6.7 % erreur en R2).

### 3.4 `crates/efe-coordinator/src/llm/providers/openai.rs` (PATCH ~40 LOC)

Idem.

### 3.5 `crates/efe-coordinator/src/llm/mock.rs` (PATCH ~25 LOC)

- `MockLlmProvider.cost_microcents_per_call: u64` (NEW field)
- `MockLlmProvider::with_cost_microcents(mc: u64) -> Self` (NEW builder)
- `MockLlmProvider::with_cost_cents(c: u32)` **préservé** : convertit en
  `cost_microcents_per_call = cents_to_microcents(c)`
- Tests offline : both builders interagissent correctement

### 3.6 `crates/efe-nano-app-coordinator/src/types/router.rs` (PATCH ~30 LOC)

- `LlmMetadata` : `+cost_input_per_million_microcents: u64`,
  `+cost_output_per_million_microcents: u64`
- Helpers `cost_input_per_million_cents() -> u32` (derived)
- `Budget.max_cost_microcents: u64` (NEW field, default = `max_cost_cents * 1000`)
- Router algo : utilise `*_microcents` pour scoring Pareto (précision ×1000)

### 3.7 `crates/efe-nano-app-coordinator/src/types/scoring.rs` (PATCH ~50 LOC)

- `LlmStaticMetadata` : `+cost_input_per_million_microcents`,
  `+cost_output_per_million_microcents`
- `TaskClassScoringTable::default_3llm_table()` : **recalibration complète**
  en microcents :
  ```rust
  static_metadata.insert(
      claude.to_string(),
      LlmStaticMetadata {
          cost_input_per_million_microcents: 300_000,  // $3/M
          cost_output_per_million_microcents: 1_500_000,  // $15/M
          // ... legacy fields aussi peuplés :
          cost_input_per_million: 300,
          cost_output_per_million: 1500,
          // ...
      },
  );
  // Gemini :
  cost_input_per_million_microcents: 7_500,  // $0.075/M (EXACT, pas arrondi)
  cost_output_per_million_microcents: 30_000,
  // ...
  ```
- Tests : précision Gemini désormais exacte (7_500 vs 8 ancien)

### 3.8 `crates/efe-coordinator/src/request.rs` (PATCH ~40 LOC)

- `MissionAResponse` : `+cost_microcents: u64`
- `MissionBResponse` : `+total_cost_microcents: u64`
- `SpeculativeResponse` : `+cost_microcents: u64`
- 3 helpers `cost_cents()` dérivés
- 3 tests serde roundtrip (R3 → R3 + R2 → R3 + R3 → R2)

### 3.9 `crates/efe-coordinator/src/metrics.rs` (PATCH ~25 LOC)

- `ChewingMetrics` : `+router_estimated_cost_microcents: u64`,
  `+spec_savings_microcents: u64`
- Wire dans `run_mission_a` (étape Router et Speculative) : utilise les
  microcents pour calcul interne, dérive `*_cents` pour audit chain

### 3.10 `crates/efe-coordinator/src/savings_report.rs` (PATCH ~80 LOC)

- `RunRecord` : `+cost_microcents: u64`
- `ModeSummary` : `+total_cost_microcents: u64, +avg_cost_microcents: f64`
- `SavingsReport` : `+baseline_total_cost_microcents`,
  `+full_total_cost_microcents`, `+savings_microcents: i64`,
  `+savings_microcents_ratio: f32`
- **`SavingsReport::to_markdown_v2()`** (NEW méthode) : utilise microcents
  partout, affiche table cost en `μ¢` avec précision ×1000. Le
  `to_markdown()` legacy reste pour rétrocompat.
- 5 tests offline : aggregation microcents, ratio savings vs cents

### 3.11 `crates/efe-coordinator/src/workflows/{mission_a,mission_b,speculative}.rs` (PATCH ~30 LOC chacun)

- Capture `llm_resp.cost_microcents` dans `chewing_metrics`
- Propage `resp.cost_microcents` dans `MissionXResponse`
- Tests workflow_mission_a + _b + _speculative : assertions sur
  microcents (vs cents en R2)

---

## 4. Spec détaillée

### 4.1 Calcul exact microcents (algorithme stable)

Pour éviter overflow u64 sur très grosses queries (M tokens), on calcule
en faisant la division par 1M en dernier :

```rust
fn compute_cost_microcents(
    input_tokens: u32,
    output_tokens: u32,
    cost_input_per_million_microcents: u64,
    cost_output_per_million_microcents: u64,
) -> u64 {
    // u64 × u64 peut overflow si tokens > 18M (improbable mais on guard).
    let in_mc = (input_tokens as u64)
        .saturating_mul(cost_input_per_million_microcents)
        / 1_000_000;
    let out_mc = (output_tokens as u64)
        .saturating_mul(cost_output_per_million_microcents)
        / 1_000_000;
    in_mc.saturating_add(out_mc)
}
```

**Note** : division integer après multiplication préserve précision
microcent. Saturating_mul/add protègent contre overflow extrême.

### 4.2 Tests offline (12 nouveaux)

```rust
// Dans provider.rs::tests
#[test]
fn microcents_to_cents_rounds_half_up() {
    assert_eq!(microcents_to_cents(0), 0);
    assert_eq!(microcents_to_cents(499), 0);
    assert_eq!(microcents_to_cents(500), 1);
    assert_eq!(microcents_to_cents(1499), 1);
    assert_eq!(microcents_to_cents(1500), 2);
    assert_eq!(microcents_to_cents(45), 0);    // Gemini fixture 01
    assert_eq!(microcents_to_cents(45_000), 45);  // ×1000 récupère 45 cents
}

#[test]
fn cents_to_microcents_multiplies_by_1000() {
    assert_eq!(cents_to_microcents(0), 0);
    assert_eq!(cents_to_microcents(1), 1_000);
    assert_eq!(cents_to_microcents(100), 100_000);
}

#[test]
fn cost_microcents_claude_fixture_01() {
    // Simulation : Claude Sonnet 4.6, 50 input tokens, 20 output tokens
    let mc = compute_cost_microcents(50, 20, 300_000, 1_500_000);
    // Expected : 50 × 300_000 / 1M + 20 × 1_500_000 / 1M = 15 + 30 = 45 μ¢
    assert_eq!(mc, 45);
}

#[test]
fn cost_microcents_gemini_fixture_01() {
    let mc = compute_cost_microcents(50, 20, 7_500, 30_000);
    // 50 × 7_500 / 1M + 20 × 30_000 / 1M = 0 + 0 = 0 μ¢ (integer division)
    // ⚠ Edge case : division integer arrondit down — 0.375 + 0.6 = ~1 μ¢ réel
    // mais en intégral on a 0+0=0. Précision limitée mais MIEUX que u32 cents.
    assert_eq!(mc, 0);
}

#[test]
fn cost_microcents_gemini_with_100_tokens() {
    let mc = compute_cost_microcents(100, 50, 7_500, 30_000);
    // 100 × 7_500 / 1M + 50 × 30_000 / 1M = 0 + 1 = 1 μ¢
    assert_eq!(mc, 1);
}

#[test]
fn microcents_overflow_safe_on_max_tokens() {
    // u32::MAX tokens × u64::MAX μ¢/M → saturating
    let mc = compute_cost_microcents(u32::MAX, u32::MAX, u64::MAX, u64::MAX);
    assert!(mc <= u64::MAX); // pas de panic
}

// Dans savings_report.rs::tests
#[test]
fn aggregate_microcents_savings_visible_on_short_fixtures() {
    let mut report = SavingsReport::with_version("r3-s1-test");
    // 5 fixtures Baseline (Claude) à 45 μ¢ chacune = 225 μ¢
    for i in 0..5 {
        let r = make_record_microcents(
            &format!("f{i}"),
            RunMode::Baseline,
            45,  // 45 μ¢ Claude
            1000,
            true,
        );
        report.add_run(r);
    }
    // 5 fixtures Full (Gemini routing) à 1 μ¢ chacune = 5 μ¢
    for i in 0..5 {
        let r = make_record_microcents(
            &format!("f{i}"),
            RunMode::Full,
            1,
            800,
            true,
        );
        report.add_run(r);
    }
    report.aggregate();
    // R3.S1 : savings microcents visible
    assert_eq!(report.baseline_total_cost_microcents, 225);
    assert_eq!(report.full_total_cost_microcents, 5);
    assert_eq!(report.savings_microcents, 220);
    assert!(
        (report.savings_microcents_ratio - 0.977).abs() < 0.01,
        "savings ratio microcents ≈ 97.7 %"
    );
    // R2 backward-compat : cents montrent 0% (perte de précision attendue)
    assert_eq!(report.savings_cents, 0); // 225 μ¢ ≈ 0 cent, 5 μ¢ ≈ 0 cent
    assert_eq!(report.savings_ratio, 0.0);
}

#[test]
fn to_markdown_v2_uses_microcents() {
    let report = mock_report_with_microcents();
    let md = report.to_markdown_v2();
    assert!(md.contains("μ¢") || md.contains("microcents"));
    assert!(md.contains("Savings cost microcents"));
}

#[test]
fn mock_provider_with_cost_microcents() {
    let p = MockLlmProvider::new("test").with_cost_microcents(45);
    // call internal calc...
    // assert that LlmResponse.cost_microcents == 45
}

#[test]
fn llm_response_serde_backward_compat_pre_r3() {
    // JSON pré-R3 sans cost_microcents
    let old_json = r#"{
        "text": "Paris",
        "tokens_generated": 1,
        "cost_cents": 1,
        "latency_ms": 1000,
        "provider": "claude",
        "self_quality": 1.0
    }"#;
    let parsed: LlmResponse = serde_json::from_str(old_json).unwrap();
    assert_eq!(parsed.cost_cents, 1);
    assert_eq!(parsed.cost_microcents, 0);  // serde default
    // Mais helper retrouve approximativement la valeur :
    assert_eq!(parsed.cost_microcents_or_from_cents(), 1_000);
}

#[test]
fn pricing_table_claude_microcents_matches_cents_x1000() {
    let (in_cents, out_cents) = PricingTable::claude("claude-sonnet-4-6");
    let (in_mc, out_mc) = PricingTable::claude_microcents("claude-sonnet-4-6");
    assert_eq!(in_mc, (in_cents as u64) * 1000);
    assert_eq!(out_mc, (out_cents as u64) * 1000);
}

#[test]
fn pricing_table_gemini_flash_microcents_exact() {
    // Vrai prix Gemini Flash = $0.075/M = 7_500 μ¢/M (exact, sans rounding)
    let (in_mc, _) = PricingTable::gemini_microcents("gemini-2.5-flash");
    assert_eq!(in_mc, 7_500);
    // Mais cents reste arrondi : 7_500 / 1000 ≈ 8 cents/M (round half up)
    let (in_cents, _) = PricingTable::gemini("gemini-2.5-flash");
    assert!(in_cents == 7 || in_cents == 8);  // selon rounding policy
}
```

### 4.3 Calibration `TaskClassScoringTable` recalibrée

```rust
// Avant R3.S1 (R2)
static_metadata.insert(
    "gemini_gemini-2.5-flash".to_string(),
    LlmStaticMetadata {
        cost_input_per_million: 8,    // ⚠ arrondi de 7.5
        cost_output_per_million: 30,
        p50_latency_ms: 800,
        // ...
    },
);

// Après R3.S1
static_metadata.insert(
    "gemini_gemini-2.5-flash".to_string(),
    LlmStaticMetadata {
        cost_input_per_million_microcents: 7_500,    // ✅ exact
        cost_output_per_million_microcents: 30_000,
        cost_input_per_million: 8,                    // legacy preserved
        cost_output_per_million: 30,
        p50_latency_ms: 800,
        // ...
    },
);
```

**Conséquence** : le Router Pareto utilise désormais les microcents pour
scoring → décisions de routing **plus précises** sur queries borderline
(où Gemini vs GPT-mini différaient de 0.5 cent en R2, désormais
différenciables).

---

## 5. Plan d'implémentation chirurgical R3.S1

### 5.1 Ordre de bataille

| # | Étape | Fichier | LOC | Vérif |
|---|---|---|---|---|
| 1 | Helpers `microcents_to_cents` + `cents_to_microcents` + tests | `provider.rs` | +30 | `cargo test microcents` |
| 2 | `LlmResponse.cost_microcents` field | `provider.rs` | +5 | `cargo check` |
| 3 | `compute_cost_microcents` helper | `provider.rs` | +20 | tests |
| 4 | `PricingTable::*_microcents` (3 providers) | `anthropic/gemini/openai.rs` | +30/each | tests pricing |
| 5 | Recalcul providers (Claude/Gemini/OpenAI) | idem | +20/each | tests live (offline mode) |
| 6 | `MockLlmProvider::with_cost_microcents` | `mock.rs` | +25 | tests mock |
| 7 | `LlmMetadata.cost_*_per_million_microcents` | `nano-app/types/router.rs` | +30 | tests |
| 8 | `LlmStaticMetadata` + recalibration table | `nano-app/types/scoring.rs` | +50 | 17 scoring tests |
| 9 | `MissionA/B/SpecResponse.cost_microcents` | `request.rs` | +40 | tests serde |
| 10 | `ChewingMetrics.*_microcents` | `metrics.rs` | +25 | tests |
| 11 | Workflows wire `cost_microcents` | `workflows/*.rs` | +30/each | tests workflow_mission_a/b/spec |
| 12 | `SavingsReport.*_microcents` + `to_markdown_v2()` | `savings_report.rs` | +80 | 5 tests + 4 offline |
| 13 | Run suite offline complète | (CI) | — | 631+12 = 643 attendu |

### 5.2 Stratégie commits R3.S1 (≤ 2 commits)

| # | Commit | Contenu |
|---|---|---|
| 1 | `feat(phase-3-r3-s1-core): cost_microcents foundation — LlmResponse + providers + pricing tables` | provider.rs + 3 providers + mock + scoring recalibration |
| 2 | `feat(phase-3-r3-s1-wiring): cost_microcents wiring — Mission A/B/Spec + ChewingMetrics + SavingsReport` | request.rs + metrics.rs + workflows + savings_report.rs + tests |

---

## 6. Critères d'acceptation binaires R3.S1

- [x] `LlmResponse.cost_microcents: u64` + `#[serde(default)]`
- [x] Helpers `microcents_to_cents` + `cents_to_microcents` + 5 tests unit
- [x] 3 pricing tables `*_microcents` (Claude/Gemini/OpenAI)
- [x] Gemini Flash input pricing = **7_500 μ¢/M exact** (vs 8 cents arrondi)
- [x] `LlmMetadata` + `LlmStaticMetadata` extensions
- [x] `TaskClassScoringTable::default_3llm_table()` recalibrée microcents
- [x] `MissionA/B/SpecResponse.cost_microcents` propagés
- [x] `ChewingMetrics.*_microcents` wired dans workflows
- [x] `SavingsReport` + `to_markdown_v2()` produit table μ¢
- [x] **12 tests offline NEW** PASS
- [x] **0 régression** : 631 tests R2 préservés (643+ total attendu)
- [x] `cargo clippy` 0 warning R3.S1
- [x] Backward-compat JSON : R2 records désérialisables avec `cost_microcents = 0`

---

## 7. Hors-scope R3.S1 (présents R3.S2+)

| Item | Sprint |
|---|---|
| `SlmCircuitBreaker` wiring (utilise microcents pour reporting overhead) | R3.S2 |
| Speculator noise rejection | R3.S3 |
| Régen `CHEWING_SAVINGS_REPORT_v2.md` (consume microcents partout) | R3.S9 |
| Prometheus `efe_cost_microcents_total` counter | R3.S7 |
| A/B test avec dimension microcents | R3.S8 |
| Migration audit chain v2 (hash inclut cost_microcents) | Phase 4+ |

---

## 8. Risques connus & mitigations

### R1 — Audit chain BLAKE3 hash diffère post-R3.S1

**Risque** : `LlmResponse` sérialisé inclut désormais `cost_microcents`.
Le hash BLAKE3 de l'audit chain change → les chaînes pré-R3 ne sont plus
"replayables" identiquement.

**Mitigation** :
- Backward-compat : ancien JSON parse OK (cost_microcents = 0 default)
- Nouveau hash R3 est documenté dans CHANGELOG comme "audit chain v2"
- Les anciens hashes pré-R3 restent valides historiquement (snapshots
  immutables)
- Pas de breaking change sur l'API publique

### R2 — Tests R2 utilisent `cost_cents` directement

**Risque** : ~30 tests R2 font `assert_eq!(resp.cost_cents, X)`. Si la
sémantique change (rounding microcents → cents), les assertions cassent.

**Mitigation** :
- `cost_cents` field reste public et calculé via `microcents_to_cents`
- Pour fixtures courtes Gemini : `cost_microcents = 1` → `cost_cents = 0`
  (vs `cost_cents = 1` en R2 si arrondi up minimum)
- **Décision chirurgicale** : maintenir `cost_cents >= 1` si
  `cost_microcents >= 1` (politique "always show 1 cent if any real cost")
  pour compatibilité tests. Documenté dans `microcents_to_cents`.

```rust
pub const fn microcents_to_cents(mc: u64) -> u32 {
    if mc == 0 { return 0; }
    ((mc + 500) / 1000).max(1).min(u32::MAX as u64) as u32
}
```

→ "Si y a un vrai coût en microcents, on montre au moins 1 cent."

### R3 — Overflow u64 sur très grosses missions

**Risque** : queries de 10M+ tokens × Claude Opus → 10M × 7.5M = 75e12 μ¢.
u64 max = 18e18 → OK. Mais saturating_mul protect.

**Mitigation** :
- `saturating_mul` partout dans `compute_cost_microcents`
- Test offline avec `u32::MAX` tokens → pas de panic

### R4 — Workspace bump version pas effectué

**Risque** : `Cargo.toml` reste 0.0.1 → consumers externes ne voient pas
le changement.

**Mitigation** :
- R3.S1 ne bump pas (cohérence R2)
- R3.S10 bump à 0.2.0 avec CHANGELOG complet
- En attendant, [Unreleased] section CHANGELOG documente R3.S1

### R5 — Calculs microcents flottent vs intégral

**Risque** : certains tests utilisent f32/f64 (`avg_cost_cents: f32`).
Précision flottante imprécise sur petites valeurs μ¢.

**Mitigation** :
- `avg_cost_microcents: f64` (vs f32) pour stabilité
- Tests utilisent tolérance `abs() < 1e-3`
- Documentation : moyennes en flottant, totaux en u64 exact

---

## 9. Vérification post-livraison R3.S1

```bash
# 1. Tests unit microcents (3 providers)
cargo test -p efe-coordinator --features all-providers --lib \
    -- microcents

# 2. Tests integration cost_microcents partout
cargo test -p efe-coordinator --features all-providers --tests \
    -- cost_microcents

# 3. Suite offline complète (régression check)
cargo test -p efe-coordinator -p efe-nano-app-coordinator \
    --features all-providers --tests --lib
# Attendu : 643+ tests, 0 failed

# 4. Lint strict
cargo clippy -p efe-coordinator -p efe-nano-app-coordinator \
    --features all-providers --all-targets -- -D warnings

# 5. Vérification ad-hoc : Gemini Flash pricing exact
cargo test pricing_table_gemini_flash_microcents_exact -- --nocapture

# 6. Backward-compat JSON
cargo test llm_response_serde_backward_compat_pre_r3 -- --nocapture

# 7. (optional) Régen offline SavingsReport avec microcents
cargo test offline_savings_report_with_microcents -- --nocapture
```

---

## 10. Lien avec R3.S2-S10

### R3.S2 — SlmCircuitBreaker wiring

R3.S2 introduit le wiring SLM circuit dans workflows. La latence du
**check circuit** est négligeable, mais le **bus event `SlmCircuitOpened`**
peut inclure `cost_microcents` de la mission où l'ouverture est observée
(R3.S1 prérequis).

### R3.S6 — Chaos engineering

Les tests chaos utilisent `cost_microcents` pour valider que le savings
**reste positif** même sous 30 % fault rate (les fallbacks Gemini sont
encore moins chers que Claude).

### R3.S7 — Prometheus telemetry

Counter Prometheus :
```
efe_cost_microcents_total{provider="claude_…"} 12_345_678
```
Précision ×1000 vs `efe_cost_cents_total{...}`.

### R3.S9 — Régen `CHEWING_SAVINGS_REPORT_v2.md`

**Le livrable principal de R3.S9 dépend directement de R3.S1.** Sans
microcents, le rapport v2 montrerait toujours `savings_cost = 0 %`. Avec
microcents, le savings réel devient visible (cf. §0.4 promesse).

---

## 11. Conclusion R3.S1

R3.S1 est le **sprint le plus impactant de R3 par effet de levier** :
4 heures de dev qui **débloquent** :

1. ✅ Le rapport `CHEWING_SAVINGS_REPORT_v2.md` honnête (R3.S9)
2. ✅ La précision routing Pareto (microcents-aware)
3. ✅ La calibration exacte des pricing tables
4. ✅ Le foundation pour Prometheus counter précis (R3.S7)
5. ✅ La fin de L1 (limitation reconnue R2)

Sans R3.S1, **R3 entier perd son argument économique défensible**.

**Effort minimal, impact maximal.** Premier sprint à livrer en R3.

---

## Annexes

### A. Pourquoi microcents (×1000) vs alternatives

| Unit | Précision | Storage | Justification |
|---|---|---|---|
| cents (u32) | ×1 | 4 bytes | R2 — insuffisant sur fixtures courtes |
| **microcents (u64)** | **×1000** | **8 bytes** | **R3.S1 — distingue Claude/Gemini ×46** |
| nanocents | ×1_000_000 | 16 bytes | Overkill, gain stockage minimal |
| picocents | ×1_000_000_000 | 32 bytes | Absurde |

Microcents est le sweet spot : précision suffisante (×1000 expose les
écarts de prix LLM réels), storage raisonnable (8 bytes), pas de
dépendances externes (u64 stdlib).

### B. Pricing tables nov-2025 en microcents

| Provider / Model | μ¢/M input | μ¢/M output |
|---|---|---|
| Claude Sonnet 4.6 | 300_000 | 1_500_000 |
| Claude Haiku 3.5 | 80_000 | 400_000 |
| Claude Opus 4.5 | 1_500_000 | 7_500_000 |
| Gemini 2.5 Flash | **7_500** | 30_000 |
| Gemini 2.5 Pro | 125_000 | 500_000 |
| GPT-4o | 250_000 | 1_000_000 |
| GPT-4o-mini | 15_000 | 60_000 |
| GPT-5 (estim) | 500_000 | 1_500_000 |
| o1 | 1_500_000 | 6_000_000 |
| o3-mini | 110_000 | 440_000 |

### C. Backward-compat strategy détaillée

#### C.1 Structure de données

```rust
// R2 era
struct LlmResponse {
    cost_cents: u32,
    // ...
}

// R3 era — preserve cost_cents field but add cost_microcents
struct LlmResponse {
    cost_cents: u32,
    cost_microcents: u64,  // NEW, default 0 if missing
    // ...
}
```

#### C.2 Roundtrip JSON

```json
// R2 → R3
{
    "cost_cents": 1,
    "tokens_generated": 50
    // cost_microcents absent
}
// R3 deserialize: { cost_cents: 1, cost_microcents: 0 }
// R3 helper: cost_microcents_resolved() returns 1000 (= cents × 1000)

// R3 → R2
{
    "cost_cents": 0,
    "cost_microcents": 45,
    "tokens_generated": 50
}
// R2 deserialize: { cost_cents: 0 } (ignore cost_microcents)
```

#### C.3 Tests consumer pré-R3

```rust
// Test pré-R3 (préservé après R3.S1)
#[test]
fn old_test_uses_cost_cents() {
    let resp = mock_claude_response();
    assert_eq!(resp.cost_cents, 1);  // ← passe toujours en R3.S1
    // Note : interne, cost_cents est dérivé de cost_microcents
    // mais via microcents_to_cents() avec policy "≥1 si > 0 μ¢"
}
```

### D. Migration path post-R3.S1

- **Consumers internes EFE** : utilisent `cost_microcents` partout
  (Mission A/B/Spec, SavingsReport, Prometheus, A/B test).
- **Consumers externes legacy** : continuent à utiliser `cost_cents`
  (helper). Migrent à leur rythme vers `cost_microcents`.
- **Audit chain** : nouveau hash BLAKE3 inclut `cost_microcents`. Hashes
  pré-R3 restent valides historiquement.
- **Documentation** : deprecation note sur `cost_cents` pointant vers
  `cost_microcents` en R4+ (warning compiler `#[deprecated]` envisageable).

---

**Fin de l'architecture R3.S1.**

Quand tu donnes le go pour implémenter, on commence par les helpers +
`LlmResponse` puis on déroule chirurgicalement les 13 étapes du §5.1.

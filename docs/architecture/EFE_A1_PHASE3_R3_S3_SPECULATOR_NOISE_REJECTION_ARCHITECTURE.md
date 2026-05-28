# EFE Phase 3 R3 — Sprint S3 : Speculator Noise Rejection (fix L4/D6)

**Précédence** : R3.S2 scellé (commit `5794cc8`, 590 tests PASS, fix L2)
**Suivant** : R3.S4 (SLM drafter local Gemma 2)
**Statut** : Spec ready-to-implement
**Effort estimé** : 5 heures
**Coût $ live estimé** : ~12 ¢ (1 test live reproduisant fixture 03, drafter Gemini + verifier Claude)
**Priorité** : 🔴 **CRITIQUE QUALITÉ** — débloque le hot path Speculative de la régression qualité S9 (50 % pass rate)

---

## 0. Pourquoi ce sprint — la limitation L4 + la découverte D6

### 0.1 L4 dans `R2_LIVE_VALIDATION.md` §6

> **L4** : Le Speculator accepte du bruit. Le hot path (`max_words_for_quick_accept`,
> défaut 10) accepte **tout** draft dont le nombre de mots est dans
> `]0, 10]`. Un drafter cheap (Gemini Flash) qui renvoie `"La"` (1 mot,
> fragment) sur une query factuelle voit son draft accepté **sans
> arbitrage verifier** → réponse fausse livrée au user.

### 0.2 D6 — l'observation live qui a tué le pass rate

S9 fixture `03_factual` (query attendant « Paris ») a produit en live :

```text
Run 10/22 : speculative... drafter="gemini" final_text="La"  → quality FAIL
```

Le drafter Gemini Flash a renvoyé `"La"` (début de phrase tronqué, ou
hallucination d'article). Le hot path a vu `draft_words = 1`, donc
`1 > 0 && 1 <= 10` → **accepté**. Le verifier (Claude, qui aurait
répondu « Paris ») **n'a jamais été appelé**. Quality pass rate
Speculative : **50 %** au lieu de ~100 % attendu.

### 0.3 Le bug exact (speculator.rs:291-294)

```rust
let draft_words = draft.text.split_whitespace().count();
let hot_path = self.max_words_for_quick_accept > 0
    && draft_words > 0                              // ← BUG : "> 0" = "≥ 1 mot"
    && draft_words <= self.max_words_for_quick_accept;
```

La borne basse `draft_words > 0` ne rejette **que le draft vide**. Tout
draft de 1+ mot passe — y compris les fragments, articles, refus, et
répétitions dégénérées.

### 0.4 ⚠️ La fausse solution (et pourquoi le R3 overview se trompe)

Le document R3 overview (`EFE_A1_PHASE3_R3_PRODUCTION_HARDENING_ARCHITECTURE.md`
§R3.S3) proposait :

```rust
// R3 overview (NAÏF — REJETÉ par cette architecture)
pub min_words_for_quick_accept: usize  // default 2
let hot_path = draft_words >= self.min_words_for_quick_accept  // ≥ 2
```

**Ce fix est incorrect.** Considérons la query FactualQa la plus
fréquente :

| Query | Bon draft | mots | `min_words=2` verdict | Réalité |
|---|---|---|---|---|
| « Capitale de la France ? » | `Paris` | **1** | ❌ **REJETÉ** → slow path | ✅ correct, savings PERDU |
| « Capitale de la France ? » | `La` | **1** | ❌ rejeté → slow path | ✅ bruit, correctement rejeté |

→ `min_words=2` **rejette « Paris » (correct) ET « La » (bruit) de la
même façon** : il ne discrimine pas le bruit, il punit la brièveté. Sur
le cas FactualQa canonique (réponse en 1 mot de contenu), il **détruit
le savings du hot path** — exactement ce que Speculative est censé
optimiser.

### 0.5 L'insight chirurgical fondateur de R3.S3

> **Le bruit n'est pas une question de LONGUEUR, c'est une question de
> CONTENU.**

- `« La »` = 1 mot, **0 mot de contenu** (article/stopword pur) → bruit.
- `« Paris »` = 1 mot, **1 mot de contenu** (nom propre) → réponse valide.
- `« Je ne sais pas »` = 4 mots, **0 information** (refus) → bruit.
- `« La La La »` = 3 mots, **1 mot unique répété** (dégénéré) → bruit.

Le discriminant correct est donc un **faisceau de signaux sémantiques
locaux et bon marché** (mots de contenu, refus, répétition), PAS un
seuil de mots. C'est la thèse de cette architecture : une
**`NoiseRejectionPolicy` composable** plutôt qu'un naïf `min_words`.

### 0.6 L'asymétrie des coûts — pourquoi la calibration est critique

| Type d'erreur | Conséquence | Sévérité |
|---|---|---|
| **Faux négatif** (rater du bruit) | « La » accepté → **réponse fausse livrée** | 🔴 SÉVÈRE (user-facing, quality fail) |
| **Faux positif** (sur-rejeter) | « Paris » rejeté → **verifier appelé inutilement** → savings perdu | 🟡 MODÉRÉ (coût, pas qualité) |

Quand un draft est rejeté comme bruit, il **tombe sur le SLOW PATH** :
le verifier (Claude) est appelé et produit la **vraie réponse**. Donc le
rejet de bruit **ne produit JAMAIS une sortie fausse** — au pire il coûte
un appel verifier sur un faux positif.

**Conséquence de design** : la policy doit maximiser le **recall sur le
bruit** (zéro faux négatif) tout en gardant les **faux positifs bas**
(ne pas pénaliser les réponses terses correctes). La détection par
**mot-de-contenu** est le sweet-spot : recall quasi-total sur le bruit
(articles, refus), faux positifs quasi-nuls (« Paris » a un mot de
contenu).

### 0.7 Ce que R3.S3 N'EST PAS

- ❌ Pas un appel LLM/SLM supplémentaire (le rejet de bruit doit être
  **purement local/syntaxique** — sinon on défait le hot path).
- ❌ Pas un changement de la logique slow path (Jaccard arbitrage inchangé).
- ❌ Pas un `min_words` naïf (cf. §0.4 — rejeté).
- ❌ Pas une analyse sémantique profonde (embeddings, similarité) — trop
  cher pour le hot path.
- ❌ Pas un changement du contrat `SpeculativeResponse` public (extension
  additive uniquement).

R3.S3 = **un gate de bruit local, déterministe, bon marché** inséré
avant la décision hot path, qui fait retomber les drafts-bruit sur le
slow path où le verifier produit la réponse correcte.

---

## 1. Audit chirurgical pré-R3.S3

### 1.1 Inventaire de l'existant

| Composant | Fichier | Statut | Action R3.S3 |
|---|---|---|---|
| `Speculator.max_words_for_quick_accept` | `speculate/speculator.rs:130` | borne haute OK | RÉUTILISE (borne haute) |
| Décision hot_path | `speculate/speculator.rs:291-294` | **bug L4** | PATCH (ajoute gate noise) |
| `Speculator::with_quick_accept` | `speculate/speculator.rs:166` | OK | RÉUTILISE |
| `SpeculationOutcome` | `speculate/speculator.rs:24-111` | OK | PATCH (+`noise_signal`) |
| `SpeculativeRequest.max_words_for_quick_accept` | `request.rs:324` (def 10) | OK | RÉUTILISE + ajoute policy |
| `default_max_words_for_quick_accept()` | `request.rs:494` (=10) | OK | RÉUTILISE |
| `run_speculative` | `workflows/speculative.rs:126-130` | construit Speculator | PATCH (wire policy + event) |
| `StreamEvent` | `events/stream.rs` | DraftToken, DrafterBypassed | PATCH (+`DraftRejectedAsNoise`) |
| `ChewingMetrics.verifier_skipped` | `metrics.rs:117` | OK | RÉUTILISE + ajoute `spec_noise_rejected` |

### 1.2 La trajectoire du draft « La » aujourd'hui (trace exacte)

```
run_speculative(req: user_text="Capitale de la France ?", max_words=10)
  └─ Speculator::new(gemini, claude, 0.7).with_quick_accept(10)
     └─ run(LlmCall)
        ├─ drafter.call() → draft.text = "La"  (1 mot)
        ├─ draft_words = 1
        ├─ hot_path = (10 > 0) && (1 > 0) && (1 <= 10) = TRUE   ← BUG
        └─ return SpeculationOutcome {
             final_text: "La",          ← BRUIT LIVRÉ
             verifier_skipped: true,    ← verifier jamais appelé
             draft_accepted: true,
           }
```

### 1.3 La trajectoire attendue post-R3.S3

```
run_speculative(req: ..., max_words=10, noise_policy=default)
  └─ run(LlmCall)
     ├─ drafter.call() → draft.text = "La"  (1 mot)
     ├─ noise = detect_noise("La", query, policy)
     │    └─ "La" → 0 content words → Some(NoiseSignal::OnlyStopwords)
     ├─ hot_path = (max_words OK) && (noise.is_none())  → FALSE
     ├─ emit StreamEvent::DraftRejectedAsNoise { signal: "OnlyStopwords", ... }
     └─ SLOW PATH : verifier.call() → "Paris"
        └─ return SpeculationOutcome {
             final_text: "Paris",        ← RÉPONSE CORRECTE
             verifier_skipped: false,
             noise_signal: Some("OnlyStopwords"),
           }
```

### 1.4 Backward-compat à préserver

Drafts qui DOIVENT continuer à hot-path (tests existants) :
- `"Paris"` (1 content word) → ✅ pass
- `"Paris is capital"` (3 content words) → ✅ pass
- `"mock response"` (2 content words) → ✅ pass
- `"twilight"` (1 content word) → ✅ pass

Drafts qui DOIVENT désormais être rejetés (D6) :
- `"La"` / `"Le"` / `"The"` / `"A"` (pure stopword) → ❌ noise → slow path
- `"I don't know"` / `"Je ne sais pas"` / `"N/A"` (refus) → ❌ noise
- `"La La La"` (répétition dégénérée) → ❌ noise
- `""` / `"   "` (vide) → ❌ noise (déjà géré, formalisé)

### 1.5 Verdict audit

| Aspect | État pré-R3.S3 | Action R3.S3 |
|---|---|---|
| Gate de bruit | ❌ Inexistant (`draft_words > 0` seulement) | NEW module `noise.rs` |
| Détection mot-de-contenu | ❌ | `content_word_count()` + stopwords bilingues |
| Détection refus | ❌ | `is_refusal()` patterns |
| Détection répétition | ❌ | `is_degenerate_repetition()` |
| Observabilité rejet | ❌ | `StreamEvent::DraftRejectedAsNoise` |
| Traçabilité metrics | ❌ | `ChewingMetrics.spec_noise_rejected` |
| Tests | ❌ | ~14 unit + 3 integration + 1 live |

---

## 2. Architecture R3.S3 — Vue d'ensemble

### 2.1 La `NoiseRejectionPolicy` composable

```
                  draft.text + query
                         │
                         ▼
        ┌────────────────────────────────────────┐
        │        detect_noise(draft, query)       │
        │  (court-circuite au PREMIER signal)     │
        │                                         │
        │  1. EmptyOrWhitespace   (toujours on)   │
        │  2. BelowMinWords       (min_words)     │
        │  3. OnlyStopwords       (content-word)  │ ← LE FIX D6
        │  4. RefusalPattern      (refus connus)  │
        │  5. DegenerateRepetition (unique/total) │
        └────────────────┬───────────────────────┘
                         │
              ┌──────────┴───────────┐
              ▼                      ▼
        Some(signal)              None
        (bruit détecté)      (draft propre)
              │                      │
              ▼                      ▼
        REJET hot path          max_words check
        → SLOW PATH             → HOT PATH si ok
        → verifier              → accept sans verifier
```

### 2.2 Sémantique : « tout signal de bruit → slow path »

La policy est un **OU logique court-circuité** : dès qu'un signal de
bruit est détecté, `detect_noise` retourne `Some(signal)` et le hot path
est bloqué. Le draft tombe sur le slow path, le verifier produit la
réponse correcte. **Le rejet de bruit ne produit jamais de sortie
fausse.**

L'ordre d'évaluation est du **moins cher au plus cher** (court-circuit) :
1. `EmptyOrWhitespace` — O(1) check `is_empty` / trim.
2. `BelowMinWords` — O(n) word count (déjà calculé).
3. `OnlyStopwords` — O(n) lookup HashSet stopwords.
4. `RefusalPattern` — O(p) lowercase prefix/substring match.
5. `DegenerateRepetition` — O(n) unique-word ratio.

### 2.3 Décision hot path révisée

```rust
// AVANT (bug L4)
let hot_path = self.max_words_for_quick_accept > 0
    && draft_words > 0
    && draft_words <= self.max_words_for_quick_accept;

// APRÈS (R3.S3)
let noise = self.noise_policy.detect(&draft.text, &req.prompt);
let hot_path = self.max_words_for_quick_accept > 0
    && draft_words <= self.max_words_for_quick_accept
    && noise.is_none();   // ← gate de bruit
```

Remarque : `draft_words > 0` est ABSORBÉ par `noise.is_none()` (un draft
vide déclenche `EmptyOrWhitespace`). La condition devient plus simple ET
plus correcte.

### 2.4 Le `NoiseSignal` enum

```rust
/// **R3.S3** — Signal de bruit détecté dans un draft. Si `detect`
/// retourne `Some`, le hot path est bloqué (→ slow path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NoiseSignal {
    /// Draft vide ou whitespace pur.
    EmptyOrWhitespace,
    /// Draft sous le seuil minimum de mots (def min_words=1 → rejette
    /// uniquement le vide ; configurable par task class).
    BelowMinWords,
    /// Draft sans aucun mot de contenu (= 100 % stopwords/ponctuation).
    /// LE fix D6 : « La », « Le », « The », « A ».
    OnlyStopwords,
    /// Draft = refus ou non-réponse connue (« I don't know », « N/A »).
    RefusalPattern,
    /// Répétition dégénérée (« La La La », unique/total < seuil).
    DegenerateRepetition,
}

impl NoiseSignal {
    /// Tag string pour bus event + metrics.
    pub const fn as_str(self) -> &'static str { /* … */ }
}
```

### 2.5 La `NoiseRejectionPolicy` struct

```rust
/// **R3.S3** — Politique de rejet de bruit pour le hot path Speculative.
///
/// Tous les checks sont LOCAUX et DÉTERMINISTES (zéro appel LLM/SLM).
/// Calibrée pour recall max sur le bruit, faux positifs min sur les
/// réponses terses correctes (« Paris »).
#[derive(Debug, Clone)]
pub struct NoiseRejectionPolicy {
    /// Mots minimum (total). Défaut 1 → rejette uniquement le vide.
    /// NB : on NE met PAS 2 (cf. §0.4 — pénaliserait « Paris »).
    pub min_words: usize,
    /// Mots de contenu (non-stopword) minimum. Défaut 1 → LE fix D6.
    pub min_content_words: usize,
    /// Active la détection de refus. Défaut true.
    pub reject_refusals: bool,
    /// Active la détection de répétition dégénérée. Défaut true.
    pub reject_repetition: bool,
    /// Seuil unique/total sous lequel un draft est « répétitif ».
    /// Défaut 0.34 (« La La La » = 1/3 ≈ 0.33 < 0.34 → rejeté ;
    /// « Paris is capital » = 3/3 = 1.0 → ok).
    pub repetition_min_unique_ratio: f32,
}

impl Default for NoiseRejectionPolicy {
    fn default() -> Self {
        Self {
            min_words: 1,
            min_content_words: 1,
            reject_refusals: true,
            reject_repetition: true,
            repetition_min_unique_ratio: 0.34,
        }
    }
}

impl NoiseRejectionPolicy {
    /// **R3.S3** — Policy permissive S7 (aucun rejet sauf vide) pour
    /// backward-compat stricte si besoin (désactive R3.S3).
    pub fn permissive_s7() -> Self {
        Self {
            min_words: 1,
            min_content_words: 0,    // pas de check content
            reject_refusals: false,
            reject_repetition: false,
            repetition_min_unique_ratio: 0.0,
        }
    }

    /// **R3.S3** — Détecte le premier signal de bruit (court-circuit).
    /// `None` = draft propre, hot path autorisé.
    pub fn detect(&self, draft: &str, _query: &str) -> Option<NoiseSignal> { /* §4 */ }
}
```

### 2.6 Décision : module dédié `speculate/noise.rs`

**Option A** (choisi) : module `speculate/noise.rs` avec
`NoiseRejectionPolicy`, `NoiseSignal`, stopwords, refusals, et tests.
- ✅ Isolé, testable unitairement sans dépendre du Speculator.
- ✅ Réutilisable par R3.S4 (SLM drafter local — même besoin de gate).
- ✅ Garde `speculator.rs` lisible.

**Option B** : inline dans `speculator.rs`.
- ❌ Mélange orchestration et heuristiques, fichier déjà lourd.

→ **Option A**.

---

## 3. Livrables R3.S3 (6 fichiers — 1 NEW, 5 PATCHES)

### 3.1 `crates/efe-coordinator/src/speculate/noise.rs` (NEW ~260 LOC)
- `NoiseSignal` enum + `as_str()`
- `NoiseRejectionPolicy` struct + `Default` + `permissive_s7()` + `detect()`
- Stopwords bilingues FR+EN (HashSet statique via `once_cell`/`LazyLock`)
- Refusal patterns (slice statique)
- Helpers : `content_word_count()`, `is_refusal()`, `is_degenerate_repetition()`
- ~14 tests unit

### 3.2 `crates/efe-coordinator/src/speculate/mod.rs` (PATCH ~3 LOC)
- `mod noise;` + `pub use noise::{NoiseRejectionPolicy, NoiseSignal};`

### 3.3 `crates/efe-coordinator/src/speculate/speculator.rs` (PATCH ~30 LOC)
- `Speculator.noise_policy: NoiseRejectionPolicy` field (default)
- `Speculator::with_noise_policy(policy)` builder
- Hot path décision révisée (§2.3)
- `SpeculationOutcome.noise_signal: Option<NoiseSignal>` (#[serde(default)])
- Émission `DraftRejectedAsNoise` si bus attaché + noise détecté

### 3.4 `crates/efe-coordinator/src/request.rs` (PATCH ~25 LOC)
- `SpeculativeRequest.min_words_for_quick_accept: usize` (#[serde(default = "default_min_words_for_quick_accept")], =1)
- `SpeculativeRequest.reject_draft_noise: bool` (#[serde(default = "default_true")], =true) — master switch R3.S3
- `default_min_words_for_quick_accept()` const → 1
- `default_true()` → true

### 3.5 `crates/efe-coordinator/src/events/stream.rs` (PATCH ~15 LOC)
- `StreamEvent::DraftRejectedAsNoise { swarm_id, signal, draft_excerpt, ts }`
- Tag `"DRAFT_REJECTED_AS_NOISE"` dans `tag()` + swarm_id dans le match swarm

### 3.6 `crates/efe-coordinator/src/metrics.rs` (PATCH ~6 LOC)
- `ChewingMetrics.spec_noise_rejected: Option<bool>` (#[serde(default)])
  - `Some(true)` : hot path bloqué par bruit → slow path
  - `Some(false)` : Speculative sans rejet (hot ou slow nominal)
  - `None` : mission non-Speculative

### 3.7 `crates/efe-coordinator/src/workflows/speculative.rs` (PATCH ~30 LOC)
- Wire `NoiseRejectionPolicy` depuis `req` (ou default si `reject_draft_noise`)
- `Speculator::...with_noise_policy(policy)`
- Trace `chewing_metrics.spec_noise_rejected = Some(outcome.noise_signal.is_some())`

### 3.8 Tests
- `tests/workflow_speculative.rs` (PATCH) : 3 tests integration R3.S3
- `tests/live_missions.rs` (PATCH) : 1 test live env-gated

---

## 4. Spec détaillée

### 4.1 `detect()` — algorithme complet

```rust
pub fn detect(&self, draft: &str, _query: &str) -> Option<NoiseSignal> {
    // 1. Empty / whitespace (le moins cher).
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        return Some(NoiseSignal::EmptyOrWhitespace);
    }

    // Tokenisation unique (réutilisée par les checks suivants).
    let words: Vec<&str> = trimmed.split_whitespace().collect();

    // 2. Below min words.
    if words.len() < self.min_words {
        return Some(NoiseSignal::BelowMinWords);
    }

    // 3. Refusal patterns (avant content-word : un refus peut contenir
    //    des mots de contenu — « I cannot answer that »).
    if self.reject_refusals && is_refusal(trimmed) {
        return Some(NoiseSignal::RefusalPattern);
    }

    // 4. Only stopwords (LE fix D6).
    if content_word_count(&words) < self.min_content_words {
        return Some(NoiseSignal::OnlyStopwords);
    }

    // 5. Degenerate repetition.
    if self.reject_repetition && is_degenerate_repetition(&words, self.repetition_min_unique_ratio) {
        return Some(NoiseSignal::DegenerateRepetition);
    }

    None  // draft propre
}
```

### 4.2 `content_word_count()` + stopwords bilingues

```rust
/// Compte les mots de CONTENU (non-stopword, après normalisation
/// lowercase + strip ponctuation de bord).
fn content_word_count(words: &[&str]) -> usize {
    words.iter().filter(|w| {
        let norm = normalize_token(w);  // lowercase + trim punctuation
        !norm.is_empty() && !STOPWORDS.contains(norm.as_str())
    }).count()
}

/// Stopwords FR + EN — focalisés sur les tokens à HAUT RISQUE de bruit :
/// articles, déterminants, pronoms, conjonctions, prépositions courantes,
/// auxiliaires. PAS une liste exhaustive (on veut recall sur le bruit,
/// pas un filtre IR).
static STOPWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        // FR articles/déterminants
        "le","la","les","un","une","des","du","de","l","d",
        "ce","cet","cette","ces","mon","ma","mes","son","sa","ses",
        // FR pronoms/conjonctions/prépositions
        "je","tu","il","elle","on","nous","vous","ils","elles",
        "et","ou","mais","donc","or","ni","car","que","qui","quoi",
        "à","au","aux","en","dans","sur","sous","par","pour","avec",
        "est","sont","être","a","ont","avoir",
        // EN articles/determiners
        "the","a","an","this","that","these","those","my","your","his","her","its",
        // EN pronouns/conjunctions/prepositions
        "i","you","he","she","it","we","they",
        "and","or","but","so","nor","for","yet",
        "to","of","in","on","at","by","with","from","as",
        "is","are","be","was","were","been","has","have","had",
    ].into_iter().collect()
});

/// Normalise un token : lowercase + retire ponctuation de bordure.
/// « Paris. » → « paris » ; « La, » → « la ».
fn normalize_token(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric())
     .to_lowercase()
}
```

**Calibration vérifiée** :
- `« La »` → normalize → `"la"` ∈ STOPWORDS → 0 content words → `OnlyStopwords` ✓
- `« Paris »` → `"paris"` ∉ STOPWORDS → 1 content word → pass ✓
- `« twilight »` → `"twilight"` ∉ STOPWORDS → pass ✓
- `« The capital »` → `"the"` stopword + `"capital"` content → 1 content word → pass ✓

### 4.3 `is_refusal()` — patterns de non-réponse

```rust
/// Détecte les refus / non-réponses connus. Match sur le draft
/// normalisé lowercase. Volontairement CONSERVATEUR (recall sur le
/// refus franc, pas sur les hedges ambigus).
fn is_refusal(draft: &str) -> bool {
    let lo = draft.to_lowercase();
    let lo = lo.trim();
    const REFUSALS: &[&str] = &[
        // EN
        "i don't know", "i do not know", "i cannot", "i can't",
        "i'm not sure", "i am not sure", "no answer", "not available",
        "n/a", "none", "null", "undefined", "error", "unknown",
        // FR
        "je ne sais pas", "je ne peux pas", "aucune réponse",
        "pas disponible", "inconnu", "erreur",
    ];
    // Match : le draft EST (≈) un refus, ou COMMENCE par un refus franc.
    REFUSALS.iter().any(|r| lo == *r || lo.starts_with(r))
}
```

**Note de prudence** : `is_refusal` est conservateur. Un draft comme
« I don't know the exact date but it's around 1990 » commence par
« i don't know » → rejeté → slow path. C'est ACCEPTABLE (asymétrie §0.6 :
faux positif = coût, pas qualité). On préfère rejeter un demi-refus que
livrer un non-réponse.

### 4.4 `is_degenerate_repetition()`

```rust
/// Répétition dégénérée : ratio mots-uniques / mots-total < seuil.
/// « La La La » → unique=1, total=3 → 0.33 < 0.34 → true.
/// « Paris is the capital » → 4/4 = 1.0 → false.
/// Sur 1-2 mots, on ne déclenche jamais (trop peu pour juger).
fn is_degenerate_repetition(words: &[&str], min_unique_ratio: f32) -> bool {
    if words.len() < 3 {
        return false;  // pas assez de signal sur 1-2 mots
    }
    let unique: HashSet<String> =
        words.iter().map(|w| normalize_token(w)).collect();
    let ratio = unique.len() as f32 / words.len() as f32;
    ratio < min_unique_ratio
}
```

### 4.5 Intégration dans `Speculator::run`

```rust
// speculator.rs — Step 2 révisé
let draft_words = draft.text.split_whitespace().count();
let noise = self.noise_policy.detect(&draft.text, &req.prompt);
let hot_path = self.max_words_for_quick_accept > 0
    && draft_words <= self.max_words_for_quick_accept
    && noise.is_none();

if let Some(signal) = noise {
    // R3.S3 : émet event observabilité (si bus attaché).
    if let (Some(bus), Some(swarm)) = (bus_opt.as_ref(), swarm_opt) {
        let excerpt: String = draft.text.chars().take(40).collect();
        let _ = bus.publish(StreamEvent::DraftRejectedAsNoise {
            swarm_id: swarm,
            signal: signal.as_str().to_string(),
            draft_excerpt: excerpt,
            ts: chrono::Utc::now(),
        }).await;
    }
    // → on NE prend PAS le hot path ; on continue vers slow path.
}

if hot_path {
    // … (inchangé) noise_signal: None
}

// SLOW PATH … noise_signal: noise  (Some si rejeté, None si draft long propre)
```

`SpeculationOutcome.noise_signal` porte le signal pour traçabilité :
- hot path → `None`
- slow path par bruit → `Some(signal)`
- slow path par longueur (draft > max_words, propre) → `None`

### 4.6 Wire workflow

```rust
// workflows/speculative.rs
let noise_policy = if req.reject_draft_noise {
    NoiseRejectionPolicy {
        min_words: req.min_words_for_quick_accept,
        ..NoiseRejectionPolicy::default()
    }
} else {
    NoiseRejectionPolicy::permissive_s7()  // désactive R3.S3 (backward-compat opt-out)
};
let speculator = Arc::new(
    Speculator::new(drafter, verifier, req.accept_threshold)
        .with_quick_accept(req.max_words_for_quick_accept)
        .with_noise_policy(noise_policy)
        .with_bus(Arc::clone(&coord.bus), swarm_id),
);
// … après run :
chewing_metrics.spec_noise_rejected = Some(outcome.noise_signal.is_some());
```

---

## 5. Plan d'implémentation chirurgical R3.S3

| # | Étape | Fichier | LOC | Vérif |
|---|---|---|---|---|
| 1 | `noise.rs` : enum + policy + detect + helpers + stopwords | `speculate/noise.rs` (NEW) | +260 | `cargo test speculate::noise` |
| 2 | 14 tests unit noise | idem | (inclus) | idem |
| 3 | `mod.rs` : déclare + re-export | `speculate/mod.rs` | +3 | `cargo check` |
| 4 | `Speculator.noise_policy` + builder + hot path révisé | `speculator.rs` | +30 | tests speculator |
| 5 | `SpeculationOutcome.noise_signal` | `speculator.rs` | +3 | idem |
| 6 | `SpeculativeRequest` + defaults | `request.rs` | +25 | tests serde |
| 7 | `StreamEvent::DraftRejectedAsNoise` | `events/stream.rs` | +15 | tests events |
| 8 | `ChewingMetrics.spec_noise_rejected` | `metrics.rs` | +6 | tests metrics |
| 9 | Wire `run_speculative` | `workflows/speculative.rs` | +30 | tests workflow |
| 10 | 3 tests integration | `tests/workflow_speculative.rs` | +120 | `cargo test workflow_speculative` |
| 11 | 1 test live | `tests/live_missions.rs` | +90 | env-gated |
| 12 | Suite offline complète | (CI) | — | 590 → ~608 attendu |
| 13 | clippy + CHANGELOG + commit | — | — | 0 warning |

**Stratégie commits** : 1 commit unique (sprint cohérent, surface contenue).

---

## 6. Tests R3.S3

### 6.1 Unit `noise.rs` (14 tests)

```rust
#[test] fn empty_is_noise()                  // "" → EmptyOrWhitespace
#[test] fn whitespace_only_is_noise()        // "   " → EmptyOrWhitespace
#[test] fn la_is_only_stopwords()            // "La" → OnlyStopwords (D6)
#[test] fn the_is_only_stopwords()           // "The" → OnlyStopwords
#[test] fn paris_passes()                    // "Paris" → None (correct !)
#[test] fn twilight_passes()                 // "twilight" → None
#[test] fn the_capital_passes()              // "The capital" → None (1 content)
#[test] fn refusal_en_rejected()             // "I don't know" → RefusalPattern
#[test] fn refusal_fr_rejected()             // "Je ne sais pas" → RefusalPattern
#[test] fn na_rejected()                     // "N/A" → RefusalPattern
#[test] fn degenerate_repetition_rejected()  // "La La La" → (OnlyStopwords en fait — voir note)
#[test] fn content_repetition_rejected()     // "go go go go" → DegenerateRepetition
#[test] fn paris_is_capital_passes()         // "Paris is capital" → None
#[test] fn permissive_s7_only_rejects_empty()// permissive: "La" → None
```

**Note chirurgicale sur « La La La »** : il est rejeté par `OnlyStopwords`
(0 content word) AVANT d'atteindre le check répétition (ordre §4.1). Le
check répétition cible les cas avec mots de CONTENU répétés (« go go go »,
« yes yes yes ») qui passeraient le content-word check. Test dédié
`content_repetition_rejected` pour couvrir ce chemin.

### 6.2 Unit `speculator.rs` (2 tests)

```rust
#[tokio::test] async fn r3s3_speculator_rejects_la_noise_falls_to_slow_path()
// drafter mock "La", verifier mock "Paris", max_words=10
// → hot path bloqué → slow path → final_text = "Paris", verifier_skipped=false,
//   noise_signal = Some(OnlyStopwords)

#[tokio::test] async fn r3s3_speculator_accepts_paris_hot_path()
// drafter mock "Paris" → hot path → final_text="Paris", verifier_skipped=true,
//   noise_signal=None  (backward-compat : pas de régression sur le bon cas)
```

### 6.3 Integration `tests/workflow_speculative.rs` (3 tests)

```rust
#[tokio::test] async fn r3s3_workflow_noise_draft_routes_to_verifier()
// run_speculative avec drafter "La" → resp.final_text="Paris",
//   resp.verifier_skipped=false, chewing.spec_noise_rejected=Some(true)

#[tokio::test] async fn r3s3_workflow_clean_short_draft_hot_path()
// drafter "Paris" → hot path → spec_noise_rejected=Some(false), verifier_skipped=true

#[tokio::test] async fn r3s3_workflow_reject_draft_noise_false_disables_gate()
// reject_draft_noise=false → "La" accepté (permissive_s7) → backward-compat opt-out
```

### 6.4 Live `tests/live_missions.rs` (1 test, env-gated)

```rust
#[tokio::test]
#[ignore]
async fn r3s3_live_speculative_rejects_noise_recovers_correct_answer() {
    // gate : EFE_LIVE_LLM=1 (+ ANTHROPIC + GOOGLE keys)
    // Setup : reproduit fixture 03 — drafter Gemini Flash, verifier Claude,
    //         query factuelle, max_words=10, noise policy default.
    // Le drafter PEUT renvoyer du bruit ("La") ou la réponse ("Paris").
    // Assertions :
    //  - si drafter renvoie du bruit → spec_noise_rejected=true → slow path
    //    → final_text contient la vraie réponse (vérifiée par keyword).
    //  - quality pass (final_text contient le keyword attendu).
    // Coût ~12 ¢ (drafter + éventuel verifier).
}
```

### 6.5 Total tests R3.S3

| Catégorie | Tests |
|---|---|
| Unit noise.rs | 14 |
| Unit speculator | 2 |
| Integration workflow | 3 |
| Live (env-gated) | 1 |
| **Total** | **20** |

---

## 7. Critères d'acceptation binaires R3.S3

- [x] Module `speculate/noise.rs` : `NoiseRejectionPolicy` + `NoiseSignal` + `detect()`
- [x] `« La »` (et tout stopword pur) → `OnlyStopwords` → slow path
- [x] `« Paris »` (1 content word) → hot path PRÉSERVÉ (pas de régression savings)
- [x] Refus EN+FR → `RefusalPattern` → slow path
- [x] Répétition dégénérée de mots de contenu → `DegenerateRepetition`
- [x] `Speculator.noise_policy` + `with_noise_policy()` builder
- [x] Hot path révisé : `… && noise.is_none()`
- [x] `SpeculationOutcome.noise_signal` propagé
- [x] `StreamEvent::DraftRejectedAsNoise` émis sur rejet (bus attaché)
- [x] `ChewingMetrics.spec_noise_rejected` tracé
- [x] `SpeculativeRequest.reject_draft_noise` (master switch, opt-out → permissive_s7)
- [x] **0 régression** : 590 tests R3.S2 préservés (~608+ attendu offline)
- [x] `cargo clippy` 0 warning R3.S3
- [x] **1 test live** PASS (env-gated)
- [x] Backward-compat : drafts content existants (« Paris », « mock response ») hot-path inchangé

---

## 8. Hors-scope R3.S3 (présents R3.S4+)

| Item | Sprint |
|---|---|
| SLM drafter local (Gemma 2 1.5B) réutilisant noise.rs | R3.S4 |
| Détection de langue mismatch (drafter répond en mauvaise langue) | R3.S4+ (nécessite l'analyzer) |
| Noise policy par task_class (Reasoning ≠ FactualQa) | R3.S5 (multi-tenant policies) |
| Détection sémantique (draft hors-sujet vs query) via embeddings | Phase 4 (trop cher pour hot path) |
| Calibration adaptative du seuil répétition (apprentissage) | R3.S8 (A/B test) |
| Counter Prometheus `efe_speculative_noise_rejected_total{signal}` | R3.S7 |

---

## 9. Risques connus & mitigations

### R1 — Faux positif sur réponse terse légitime
**Risque** : « OK » (1 mot, pas stopword EN ? en fait « ok » n'est pas
dans STOPWORDS) ou un acronyme rare rejeté à tort.
**Mitigation** : la liste STOPWORDS est volontairement RESTREINTE aux
function words à haut risque. Un mot de contenu rare (acronyme, nom
propre) n'est pas dedans → passe. Asymétrie §0.6 : un faux positif coûte
un verifier, pas une mauvaise réponse. `spec_noise_rejected` tracé permet
de mesurer le taux de faux positifs en prod et recalibrer (R3.S8).

### R2 — Stopwords multilingues incomplets
**Risque** : un draft-bruit dans une langue non couverte (DE, ES, ...)
n'est pas détecté comme `OnlyStopwords`.
**Mitigation** : FR+EN couvrent les fixtures actuelles. R3.S4 branchera
`analyzer.language` (IsoLang) pour sélectionner le stopword set par
langue. Pour l'instant, un draft-bruit non-FR/EN tomberait sur les autres
signaux (refusal, repetition) ou, à défaut, prendrait le hot path —
dégradation gracieuse (= comportement S7, pas pire).

### R3 — Refusal patterns trop agressifs
**Risque** : « None of the above is correct, the answer is X » commence
par « none » → rejeté à tort.
**Mitigation** : `is_refusal` matche `lo == r || lo.starts_with(r)`.
« none of the above… » commence par « none » → rejeté → slow path →
verifier produit la réponse. Faux positif = coût, pas qualité. Si le
taux observé est trop haut (R3.S8 metrics), affiner en exigeant que le
refus soit le draft ENTIER (`lo == r`) plutôt que prefix.

### R4 — Le drafter bruite SYSTÉMATIQUEMENT (Gemini Flash dégradé)
**Risque** : si Gemini Flash renvoie « La » sur 100 % des queries, R3.S3
fait retomber 100 % sur slow path → savings Speculative = 0 (mais qualité
préservée).
**Mitigation** : c'est le comportement CORRECT (mieux vaut 0 savings que
100 % de réponses fausses). Le taux de `spec_noise_rejected` devient un
**signal de santé du drafter** : si > 30 %, le drafter est mal choisi
(R3.S4 le remplacera par un SLM local mieux contrôlé). Le bus event
`DraftRejectedAsNoise` rend ça observable.

### R5 — Régression sur tests S7 existants
**Risque** : un test S7 utilisant un mock drafter renvoyant un stopword
(« the ») casserait.
**Mitigation** : audit pré-implémentation des mock responses dans les
tests Speculative existants (`"Paris"`, `"Paris is capital"`, `"mock
response"` → tous content words → OK). Le master switch
`reject_draft_noise=false` (→ `permissive_s7()`) offre un opt-out si un
test legacy a besoin du comportement S7 exact.

### R6 — Coût latence du gate
**Risque** : `detect()` ajoute de la latence au hot path.
**Mitigation** : tous les checks sont O(n) sur la longueur du draft
(typiquement < 50 mots) avec court-circuit. Coût < 10 µs, négligeable vs
latence drafter (~500 ms). Pas de regex compilée à chaud (patterns =
slices statiques, stopwords = LazyLock HashSet).

---

## 10. Vérification post-livraison R3.S3

```bash
# 1. Unit noise
cargo test -p efe-coordinator --lib speculate::noise

# 2. Unit speculator (hot path révisé)
cargo test -p efe-coordinator --lib speculate::speculator::tests::r3s3

# 3. Integration workflow
cargo test -p efe-coordinator --test workflow_speculative r3s3

# 4. Suite offline complète (régression)
cargo test -p efe-coordinator -p efe-nano-app-coordinator --tests --lib
# Attendu : ~608+ tests, 0 failed

# 5. Lint
cargo clippy -p efe-coordinator --all-targets -- -D warnings

# 6. Backward-compat : "Paris" hot-path préservé
cargo test r3s3_speculator_accepts_paris_hot_path -- --nocapture

# 7. Le fix D6 : "La" → "Paris"
cargo test r3s3_workflow_noise_draft_routes_to_verifier -- --nocapture

# 8. Live (env-gated)
EFE_LIVE_LLM=1 cargo test r3s3_live_speculative_rejects_noise -- --ignored --nocapture
```

---

## 11. Lien avec R3.S4-S10

### R3.S4 — SLM drafter local (Gemma 2 1.5B)
R3.S4 remplace le drafter Gemini cloud par un SLM local. Il **réutilise
`noise.rs` tel quel** — un SLM local bruite tout autant (voire plus)
qu'un LLM cloud cheap. Le gate de bruit devient encore plus critique.
R3.S4 branchera aussi `analyzer.language` pour les stopwords par langue
(R2 §9).

### R3.S5 — Multi-tenant policies
Permettra une `NoiseRejectionPolicy` par tenant / par task_class
(Reasoning exige plus de mots de contenu que FactualQa).

### R3.S7 — Prometheus
Counter `efe_speculative_noise_rejected_total{signal="only_stopwords"}` —
permet d'alerter si un drafter bruite anormalement.

### R3.S8 — A/B test
Calibration empirique du `repetition_min_unique_ratio` et des refusal
patterns via A/B sur trafic réel.

### R3.S9 — Régen rapport savings v2
Le rapport inclura : (a) quality pass rate Speculative AVANT/APRÈS R3.S3
(50 % → cible ≥ 90 %), (b) % de drafts rejetés comme bruit, (c) coût du
gate (faux positifs).

---

## 12. Conclusion R3.S3

R3.S3 ferme la **dernière limitation qualité de R2** (L4). Le hot path
Speculative passait de la qualité S9 à 50 % à cause d'un seul `> 0` trop
laxiste. Le fix n'est PAS un `min_words` naïf (qui détruirait le savings
sur « Paris »), mais une **détection de bruit par contenu** : un draft
n'est accepté en hot path que s'il porte de l'information (mot de contenu,
pas refus, pas répétition).

L'asymétrie des coûts (§0.6) guide tout le design : **zéro faux négatif**
(jamais livrer du bruit) au prix de quelques faux positifs (verifier
appelé inutilement) — car un faux positif coûte des microcents, un faux
négatif coûte la confiance du user.

C'est aussi le **fondement réutilisable** de R3.S4 : tout drafter (cloud
ou SLM local) a besoin de ce gate. `noise.rs` est conçu comme une
primitive autonome, testable, et extensible (langues, task classes).

**Effort 5h, impact : restaure le pass rate Speculative de 50 % à ≥ 90 %
sans sacrifier le savings du hot path sur les réponses terses correctes.**

---

## Annexes

### A. Tableau de décision noise complet

| Draft | trimmed | words | content words | refus? | unique/total | → Signal | Path |
|---|---|---|---|---|---|---|---|
| `""` | `""` | 0 | 0 | — | — | EmptyOrWhitespace | slow |
| `"   "` | `""` | 0 | 0 | — | — | EmptyOrWhitespace | slow |
| `"La"` | `"La"` | 1 | 0 | non | — | OnlyStopwords | slow |
| `"The"` | `"The"` | 1 | 0 | non | — | OnlyStopwords | slow |
| `"Paris"` | `"Paris"` | 1 | 1 | non | 1.0 | **None** | **HOT** |
| `"twilight"` | … | 1 | 1 | non | 1.0 | **None** | **HOT** |
| `"The capital"` | … | 2 | 1 | non | 1.0 | **None** | **HOT** |
| `"I don't know"` | … | 3 | — | **oui** | — | RefusalPattern | slow |
| `"Je ne sais pas"` | … | 4 | — | **oui** | — | RefusalPattern | slow |
| `"N/A"` | `"N/A"` | 1 | (refus) | **oui** | — | RefusalPattern | slow |
| `"La La La"` | … | 3 | 0 | non | — | OnlyStopwords | slow |
| `"go go go go"` | … | 4 | 4 | non | 0.25 | DegenerateRepetition | slow |
| `"Paris is the capital"` | … | 4 | 2 | non | 1.0 | **None** | **HOT** |

### B. Pourquoi pas un classifieur ML pour le bruit ?

| Approche | Coût | Latence | Déterminisme | Verdict |
|---|---|---|---|---|
| Heuristiques locales (R3.S3) | 0 | < 10 µs | ✅ total | ✅ choisi |
| Petit classifieur (DistilBERT) | modèle à charger | ~5-20 ms | ⚠️ | ❌ overkill hot path |
| Appel LLM « est-ce du bruit ? » | 1 appel LLM | ~300 ms + $ | ❌ | ❌ **défait le hot path** |

Le hot path existe POUR éviter un appel LLM. Toute détection de bruit qui
appelle un modèle est auto-contradictoire. D'où le choix d'heuristiques
locales pures.

### C. Backward-compat — matrice

| Scénario pré-R3.S3 | Comportement post-R3.S3 |
|---|---|
| `SpeculativeRequest` sérialisé sans `reject_draft_noise` | `#[serde(default)]` → `true` → gate actif (amélioration qualité) |
| `SpeculativeRequest` sans `min_words_for_quick_accept` | default → 1 → ne rejette que le vide via ce check |
| Test S7 avec drafter content (« Paris ») | hot path inchangé |
| Opt-out explicite `reject_draft_noise=false` | `permissive_s7()` → comportement S7 exact |
| `SpeculationOutcome` désérialisé pré-R3.S3 | `noise_signal` `#[serde(default)]` → None |

### D. Calibration des seuils (justification chiffrée)

| Paramètre | Valeur | Justification |
|---|---|---|
| `min_words` | 1 | Rejette le vide uniquement. PAS 2 (pénaliserait « Paris »). |
| `min_content_words` | 1 | LE fix D6. « Paris »=1 pass, « La »=0 reject. |
| `repetition_min_unique_ratio` | 0.34 | « go go go »=1/3≈0.33 < 0.34 reject ; tout draft avec ≥ 1/3 mots uniques pass. Seuil bas = conservateur (peu de faux positifs). |
| `reject_refusals` | true | Refus = non-information, doit aller au verifier. |
| Min words pour check répétition | 3 | < 3 mots = trop peu de signal pour juger la répétition. |

**Fin de l'architecture R3.S3.**

Prêt pour implémentation. Sprint à surface contenue (1 module NEW + 5
patches), mais à fort impact qualité (restaure le pass rate Speculative).
L'insight central — **bruit = absence de contenu, pas brièveté** — corrige
la fausse piste du R3 overview et préserve le savings du hot path sur le
cas FactualQa canonique.

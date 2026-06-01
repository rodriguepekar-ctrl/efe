# M8 — Output Projection & Dissolution Engine
## Architecture complète, ultra-détaillée

---

**Document** : spécification d'architecture
**Module** : `efe_core::dissolution` (renommé depuis le stub) + `efe_core::output`
**Sprints ciblés** : Sprint 7 (MVP extracteur + projecteur) → Sprint 8 (decoders)
**Dépend de** : M1 (Core), M4 (ELS), M5 (FusionStats), M6 (ConsensusVerdict), M9 (Alignment — déjà partiel dans `els::projection`)
**Consommé par** : M1 controller (dernière étape du pipeline), M12 (FFI/Server — packaging), M11 (telemetry)
**Statut** : spec gelée pour implémentation — aucune ambiguïté subsistante

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP](#2-scope-mvp)
3. [Théorie : dissolution & projection](#3-théorie--dissolution--projection)
4. [Vue d'ensemble architecturale](#4-vue-densemble-architecturale)
5. [Traits et abstractions](#5-traits-et-abstractions)
6. [Structures de données](#6-structures-de-données)
7. [Flux d'une extraction](#7-flux-dune-extraction)
8. [Algorithmes détaillés](#8-algorithmes-détaillés)
9. [Gestion d'anomalies et qualité](#9-gestion-danomalies-et-qualité)
10. [Performance et budget](#10-performance-et-budget)
11. [Interface publique](#11-interface-publique)
12. [Stratégie de test](#12-stratégie-de-test)
13. [Dépendances inter-modules](#13-dépendances-inter-modules)
14. [Plan de sprint M8](#14-plan-de-sprint-m8)
15. [Critères d'acceptation](#15-critères-dacceptation)
16. [Risques et mitigations](#16-risques-et-mitigations)
17. [Annexes](#17-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M8 est la **dernière étape** du pipeline EFE. Une fois le swarm convergé (M6
émet `Converged { confidence }`), M8 transforme l'ELS latent — une collection
de `N` vecteurs bf16 dans un espace `D`-dimensionnel — en **une réponse
unique, projetée et décodée**.

Concrètement, M8 répond à trois questions en séquence :

1. **Quel est le point central du swarm ?** → extraction d'un centroïde
   (moyenne robuste, "Fréchet mean" dans M8+).
2. **Ce point respecte-t-il l'alignement ?** → projection sur le cône
   d'anchors fourni dans `SwarmSpec::alignment_cone`.
3. **Qu'est-ce que cela signifie en sortie ?** → décodage via un
   `OutputDecoder` injectable (decode vers embedding, texte, ou action
   selon la politique du client).

Le nom *dissolution* vient de la métaphore opposée à *genesis* (M3) : M3 fait
naître le swarm à partir d'un problème, M8 dissout le swarm en une réponse.

### 1.2 Contraintes et garanties

1. **Read-only sur ELS** : M8 n'écrit **jamais** dans l'ELS. Il lit le latent,
   le mask, les anchors, et retourne des résultats.
2. **Alignment breach = fatale** : si la distance projeté-centroïde excède
   `cone.max_radius`, M8 retourne `EfeError::AlignmentBreach` — le client ne
   reçoit jamais un résultat hors-cône.
3. **Déterministe** : même ELS + même config → même `OutputBundle`. Pas de
   PRNG, pas d'I/O non-déterministe.
4. **Borné en temps** : `extract()` termine en moins de 20 ms pour
   `N=128, D=2048, K=4 anchors`.
5. **Observable** : chaque extraction émet un `ExtractionTelemetry` consommable
   par M11.

### 1.3 Position dans le pipeline EFE

```
  ┌─────────────────────────────────────────────────────────────┐
  │                     Controller (M1)                          │
  └───┬──────┬──────┬──────┬──────┬──────┬──────┐
      │      │      │      │      │      │      │
    spawn  fuse  check  collapse extract decode return
      │      │      │      │      │      │      │
      ▼      ▼      ▼      ▼      ▼      ▼      ▼
    ┌─────┐┌────┐┌─────┐┌──────┐┌──────────────────┐
    │ M3  ││ M5 ││ M6  ││  M7  ││       M8         │
    │Gene-││Fus.││Cons.││Colla-││Output Projection │
    │sis  ││    ││Det. ││pse   ││  & Dissolution   │
    └─────┘└────┘└─────┘└──────┘└────────┬─────────┘
                                         │
                                         ▼
                                    OutputBundle
                                    (centroid, projected,
                                     confidence, anchor_dist,
                                     decoded_output, timing)
                                         │
                                         ▼
                                    M12 Server / FFI
                                         │
                                         ▼
                                       Client
```

### 1.4 Garanties offertes aux modules avals

1. **`OutputBundle::validate()` passe** après toute extraction réussie.
2. **`projected_centroid.is_finite()` pour tous les éléments**.
3. **`confidence ∈ [threshold, 1]`** (hérité de M6).
4. **`alignment_distance ≤ cone.max_radius + ε`** (ε = 1e-5 pour robustesse
   numérique).
5. **Trace causale** : `OutputBundle::extraction_telemetry.source_round`
   permet de retrouver le round M5 qui a produit le state final.

---

## 2. Scope MVP

### 2.1 IN

| Fonctionnalité | Justification MVP |
|---|---|
| Trait `OutputProjector` (DI) | Permet swap d'extracteur (mean / median / Fréchet). |
| `CentroidExtractor` MVP | Weighted mean sur agents actifs. |
| `AlignmentProjector` wrapper | Réutilise `els::projection::project_vector_onto_anchors`. |
| `OutputDecoder` trait + `IdentityDecoder` MVP | Identity passe le centroïde projeté directement. |
| `OutputBundle` sérialisable | M11/M12 consommateurs. |
| Validation `max_radius` | Garantie d'alignement. |
| `ExtractionConfig` avec stratégie/tolerances | Tunable en prod. |
| Tests determinism + E2E M5→M6→M8 | Complet pipeline. |
| `ExtractionTelemetry` pour M11 | Observabilité. |

### 2.2 OUT (reporté Sprint 8+)

| Fonctionnalité | Raison du report |
|---|---|
| Fréchet mean itérative (géodésique) | Complexe, vectoriel simple suffit pour MVP. |
| Decoders externes (HF, ONNX) | Dépend du backend GPU et des poids. Sprint 9. |
| Multi-tête d'extraction (top-k) | Cas avancé (output multiple). Sprint 10. |
| Cross-swarm dissolution | Multi-swarm nécessite M9. |
| Learned alignment scoring | Nécessite dataset, Sprint 11+. |

### 2.3 Principe "extract-project-decode"

M8 est strictement structuré en **3 étapes obligatoires**, dans l'ordre :

```
ELS ──► (1) EXTRACT ──► centroid_raw
                             │
                             ▼
                     (2) PROJECT ──► centroid_projected
                                          │
                                          ▼
                                  (3) DECODE ──► decoded_output
                                                      │
                                                      ▼
                                              OutputBundle
```

Aucune étape ne peut être skippée. Chaque étape est un trait injectable
→ testable, swappable, profilable indépendamment.

---

## 3. Théorie : dissolution & projection

### 3.1 Extraction du centroïde — 3 variantes MVP

Le centroïde brut est calculé par un `CentroidExtractor`. Le MVP en fournit **deux** :

**`WeightedMeanExtractor` (default)** :
```
μ = (1 / Σ w_i) * Σ_i w_i * h_i    pour i tel que mask_i = 1
```
où `w_i` peut être :
- `1.0` (uniforme, "mean") — default MVP.
- `1/(1 + dist(h_i, μ_raw)²)` (robuste aux outliers, "attention-weighted") — Sprint 8.

**`MedianCoordinateExtractor`** :
```
μ[d] = median({h_i[d] | mask_i = 1})
```
Robuste aux outliers, stable sous petite perturbation. Coût `O(N·D·log N)`.

### 3.2 Projection sur le cône d'alignement

Entrée : `centroid_raw: Vec<f32>`, `anchors: &[Vec<f32>]`, `cone_radius: f32`.

Algorithme (adapté de `els::projection`, variant "strict" de M8) :
```
1. Si cone est vide :
     return Err(AlignmentBreach { delta: ∞, max: 0 })
2. Compute weights_i = max(cosine(centroid_raw, anchor_i), 0)
3. Si Σ weights_i < ε :
     // centroïde orthogonal → fail, pas d'alignement possible
     return Err(AlignmentBreach { delta: ∞, max: cone_radius })
4. Normalize: weights_i /= Σ weights_i
5. anchor_centroid = Σ weights_i * anchor_i
6. projected_centroid = (1 - α) * centroid_raw + α * anchor_centroid
                      où α est config.alignment_strength (default 0.5)
7. delta = ||centroid_raw - projected_centroid||_2
8. Si delta > cone_radius * (1 + tolerance) :
     return Err(AlignmentBreach { delta, max: cone_radius })
9. return projected_centroid, delta
```

**Différence majeure avec `els::projection`** (utilisé par M5 pendant les
rounds) : M5 utilise une projection **douce** (ne fail jamais), M8 utilise
une projection **stricte** qui émet `AlignmentBreach` si le centroïde est
trop loin du cône — c'est la dernière chance de rejeter un output hors
politique.

### 3.3 Décodage

Transforme `projected_centroid: Vec<f32>` en `DecodedOutput`. Le trait
`OutputDecoder` a une API asynchrone pour permettre les decoders externes
(Sprint 9+).

**MVP : `IdentityDecoder`** :
```
return DecodedOutput::Embedding(centroid_projected.clone())
```
Simple pass-through. La "décision" de décodage est laissée au client, qui
reçoit le vecteur brut et fait ses propres transformations.

**Sprint 8 : `NearestAnchorDecoder`** :
```
best = argmax_k cosine(projected_centroid, anchor_k)
return DecodedOutput::AnchorLabel(labels[best], score = cosine(best))
```

**Sprint 9 : `HuggingFaceDecoder`** :
```
tokens = external_decoder(projected_centroid)
return DecodedOutput::Text(tokens)
```

### 3.4 Propriétés mathématiques

1. **Convexité** : weighted mean d'un ensemble convexe est dans ce
   ensemble convexe. Donc `centroid_raw` est dans l'enveloppe convexe
   des latents actifs. ✓
2. **Projection contractante** : `||projected - anchor_centroid|| ≤
   ||centroid_raw - anchor_centroid||`. Distance au cône monotonement
   décroissante. ✓
3. **Idempotence** : `project(project(x)) = project(x)` à la précision de
   l'interpolation près. Pour MVP avec `α ≠ 1`, non-strict mais bornée.
4. **Determinisme** : pas de PRNG, pas d'I/O, O(1) state mutable → mêmes
   entrées = mêmes sorties.

---

## 4. Vue d'ensemble architecturale

### 4.1 Disposition

```
efe-core/src/dissolution/
├── mod.rs                     # Facade + re-exports
├── config.rs                  # ExtractionConfig
├── bundle.rs                  # OutputBundle + DecodedOutput
├── telemetry.rs               # ExtractionTelemetry
├── projector.rs               # OutputProjector trait
├── extractors/
│   ├── mod.rs                 # Re-exports
│   ├── weighted_mean.rs       # WeightedMeanExtractor
│   └── median.rs              # MedianCoordinateExtractor
├── alignment.rs               # AlignmentProjector (strict)
├── decoders/
│   ├── mod.rs
│   ├── identity.rs            # IdentityDecoder
│   └── nearest_anchor.rs      # NearestAnchorDecoder (Sprint 8)
├── engine.rs                  # DissolutionEngine (orchestration)
└── validator.rs               # Post-extraction invariant checks
```

### 4.2 Diagramme de classes

```
                ┌──────────────────────────────┐
                │   OutputProjector (trait)     │
                │   async fn extract → Bundle   │
                └──────────────┬───────────────┘
                               │ impl
                        ┌──────┴──────┐
                        ▼             ▼
           ┌─────────────────┐  ┌─────────────┐
           │ MvpProjector    │  │ StubProjector│
           │ (orchestrates)  │  │  (tests)     │
           └────────┬────────┘  └─────────────┘
                    │ uses
          ┌─────────┼─────────────────┐
          ▼         ▼                 ▼
  ┌────────────┐ ┌──────────────┐ ┌─────────────┐
  │ Centroid   │ │ Alignment    │ │  Output     │
  │ Extractor  │ │  Projector   │ │  Decoder    │
  │ (trait)    │ │ (strict)     │ │ (trait)     │
  └────────────┘ └──────────────┘ └─────────────┘
    │  │                              │  │
    │  │                              │  │
   mean median                     identity nearest
    weighted                       (Sprint 8)
```

### 4.3 Pipeline d'une extraction

```
  ┌─────────────────────────────────────────────────┐
  │    projector.extract(els, arena, cfg, ctx)      │
  └─────────────────────────────────────────────────┘
                        │
               ┌────────┼────────┐
               ▼                 ▼
      ┌──────────────┐     ┌──────────────┐
      │ validate_    │     │ lookup anchor │
      │ input_state  │     │ cone from ELS │
      └──────┬───────┘     └──────┬───────┘
             │                    │
             └──────────┬─────────┘
                        ▼
                ┌──────────────┐
                │  extractor   │ → centroid_raw
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │  projector   │ → centroid_projected, delta
                │  (strict)    │ → maybe AlignmentBreach
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │   decoder    │ → decoded_output
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │  validator   │ → OK / Err
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │   bundle     │ → OutputBundle
                └──────────────┘
```

---

## 5. Traits et abstractions

### 5.1 `OutputProjector` — façade top-level

```rust
/// Dissolve a converged swarm into a final output.
///
/// # Contract
///
/// - Called exactly once per swarm, after M6 emits `Converged`.
/// - Read-only over ELS (no mutation).
/// - Returns `OutputBundle` on success, `EfeError::AlignmentBreach`
///   on cone violation, or `EfeError::Internal` on structural issues.
#[async_trait::async_trait]
pub trait OutputProjector: Send + Sync + std::fmt::Debug {
    /// Run the full extract → project → decode pipeline.
    ///
    /// # Errors
    /// - [`EfeError::AlignmentBreach`] if projected-centroid distance
    ///   exceeds `cone.max_radius * (1 + config.tolerance)`.
    /// - [`EfeError::Internal`] if the ELS is a stub or not populated.
    async fn extract(
        &self,
        els: &EpistemicLatentSpace,
        arena: &Arena,
        config: &ExtractionConfig,
        ctx: &ExtractionContext,
    ) -> Result<OutputBundle>;

    /// Stable short name for telemetry.
    fn name(&self) -> &'static str;
}
```

### 5.2 `CentroidExtractor` — step 1

```rust
/// Extract a single centroid vector from the swarm's active agents.
///
/// Pure math, no async — called in extractor threads during Sprint 9+.
pub trait CentroidExtractor: Send + Sync + std::fmt::Debug {
    /// Produce a `Vec<f32>` of length = latent dim.
    ///
    /// # Preconditions
    /// - `latent.n_agents() == mask.n_agents()`.
    /// - At least one mask bit is set.
    ///
    /// # Errors
    /// - [`EfeError::Internal`] on pre-condition violation.
    fn extract(
        &self,
        latent: &LatentView<'_>,
        mask: &MaskView<'_>,
    ) -> Result<Vec<f32>>;

    fn name(&self) -> &'static str;
}
```

### 5.3 `AlignmentProjector` — step 2 (strict)

```rust
/// Project a centroid onto the alignment cone, with strict
/// breach-detection on output.
pub trait AlignmentProjector: Send + Sync + std::fmt::Debug {
    /// Returns `(projected_centroid, distance_from_raw, anchor_weights)`.
    ///
    /// # Errors
    /// - [`EfeError::AlignmentBreach`] if the projection would violate
    ///   `cone_radius * (1 + tolerance)`.
    /// - [`EfeError::Internal`] on empty anchors.
    fn project(
        &self,
        centroid_raw: &[f32],
        anchors: &[Vec<f32>],
        cone_radius: f32,
        config: &ExtractionConfig,
    ) -> Result<ProjectionOutcome>;

    fn name(&self) -> &'static str;
}

pub struct ProjectionOutcome {
    pub projected_centroid: Vec<f32>,
    pub distance_from_raw: f32,
    pub anchor_weights: Vec<f32>,
}
```

### 5.4 `OutputDecoder` — step 3

```rust
/// Transform a projected centroid into a final output form.
#[async_trait::async_trait]
pub trait OutputDecoder: Send + Sync + std::fmt::Debug {
    async fn decode(
        &self,
        projected: &[f32],
        anchor_weights: &[f32],
    ) -> Result<DecodedOutput>;

    fn name(&self) -> &'static str;
}
```

### 5.5 `DissolutionEngine` — top-level orchestrator

```rust
pub struct DissolutionEngine {
    extractor: Arc<dyn CentroidExtractor>,
    projector: Arc<dyn AlignmentProjector>,
    decoder: Arc<dyn OutputDecoder>,
}

impl DissolutionEngine {
    pub fn builder() -> DissolutionEngineBuilder { … }
    pub fn mvp_default() -> Self { … }
}
```

---

## 6. Structures de données

### 6.1 `ExtractionConfig`

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ExtractionConfig {
    /// Blend factor between raw centroid and anchor-centroid.
    /// α = 0.0 → return raw (no projection); 1.0 → fully projected.
    pub alignment_strength: f32,           // default: 0.5

    /// Tolerance multiplier on `cone.max_radius`: breach fires only if
    /// `delta > max_radius * (1 + tolerance)`. Default 0.05 (5% slack).
    pub breach_tolerance: f32,             // default: 0.05

    /// Minimum required confidence for extraction (cross-check with M6).
    /// If `ConsensusVerdict::Converged { confidence }` has confidence
    /// below this, return `Internal` without running extractors.
    pub min_confidence: f32,               // default: 0.5

    /// Wall-clock deadline (ms).
    pub deadline_ms: u32,                  // default: 20

    /// When true, run post-extraction validator (expensive in hot-path).
    pub validate_post: bool,               // default: cfg!(debug_assertions)
}
```

Validation : `alignment_strength ∈ [0,1]`, `breach_tolerance ∈ [0,1]`,
`min_confidence ∈ [0,1]`, `deadline_ms ≥ 1`.

### 6.2 `ExtractionContext`

```rust
#[derive(Clone, Debug)]
pub struct ExtractionContext {
    /// Final round number (the one that produced the converged state).
    pub final_round: u32,
    /// Confidence emitted by M6 at convergence.
    pub convergence_confidence: f32,
    /// Swarm identifier (for telemetry correlation).
    pub swarm_id: SwarmId,
}
```

### 6.3 `OutputBundle`

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct OutputBundle {
    /// Swarm identifier.
    pub swarm_id: SwarmId,
    /// Raw centroid extracted from the ELS (pre-projection).
    pub centroid_raw: Vec<f32>,
    /// Projected centroid (post-alignment).
    pub centroid_projected: Vec<f32>,
    /// Confidence from M6 (passes through unchanged).
    pub confidence: f32,
    /// Alignment distance `||centroid_raw - centroid_projected||`.
    pub alignment_distance: f32,
    /// Normalized weights per anchor (how much each contributed).
    pub anchor_weights: Vec<f32>,
    /// Final decoded output.
    pub decoded: DecodedOutput,
    /// Telemetry for M11.
    pub telemetry: ExtractionTelemetry,
}

impl OutputBundle {
    /// Verify post-condition invariants.
    pub fn validate(&self) -> Result<()>;
}
```

### 6.4 `DecodedOutput` — tagged union

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum DecodedOutput {
    /// Identity pass-through: the raw f32 embedding.
    Embedding(Vec<f32>),
    /// Anchor chosen by nearest-cosine + its score.
    AnchorLabel { index: u32, score: f32 },
    /// Decoded tokens (Sprint 9+).
    Text(String),
    /// Raw bytes from an opaque decoder (Sprint 9+).
    Bytes(Vec<u8>),
}

impl DecodedOutput {
    pub fn kind(&self) -> &'static str { … }  // "embedding" | "anchor_label" | …
    pub fn byte_size(&self) -> usize { … }
}
```

### 6.5 `ExtractionTelemetry`

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ExtractionTelemetry {
    pub source_round: u32,
    pub extractor_name: String,
    pub projector_name: String,
    pub decoder_name: String,
    pub extract_us: u32,
    pub project_us: u32,
    pub decode_us: u32,
    pub total_us: u32,
    pub n_active_agents: u32,
    pub n_anchors: u32,
}
```

---

## 7. Flux d'une extraction

### 7.1 Séquence chronologique

```
T=0μs       projector.extract(els, arena, cfg, ctx) entry.
T=~10μs     cfg.validate() — InvalidConfig on bad inputs.
T=~20μs     min_confidence guard — if ctx.convergence_confidence <
            cfg.min_confidence, return Internal without extracting.
T=~30μs     els.latent_view(arena) → LatentView.
T=~50μs     els.mask_view(arena) → MaskView.
T=~70μs     els.anchors_view(arena)?.to_host() → Vec<Vec<f32>>.
T=~100μs    extractor.extract(&latent, &mask) — O(N·D), ~5ms.
T=~5100μs   projector.project(&raw, &anchors, radius, cfg) — O(K·D), ~300μs.
            (may return AlignmentBreach here)
T=~5400μs   decoder.decode(&projected, &weights) — depends.
            (Identity: ~100μs. Text decoders: Sprint 9.)
T=~5500μs   validator.validate(&bundle) if cfg.validate_post.
T=~5700μs   Build OutputBundle with telemetry. Return.
```

### 7.2 Invariants maintenus

- Si `Ok(bundle)` retourné, `bundle.validate()` passe.
- `total_us ≤ cfg.deadline_ms * 1000` (or DeadlineExceeded).
- `centroid_raw.len() == centroid_projected.len() == els.dim().get()`.
- `anchor_weights.len() == anchors.n_anchors()`.
- Tous les floats sont finis.

### 7.3 Allocations

M8 alloue :
- `centroid_raw: Vec<f32>` — `D * 4` bytes
- `centroid_projected: Vec<f32>` — `D * 4` bytes
- `anchor_weights: Vec<f32>` — `K * 4` bytes
- `anchors_host: Vec<Vec<f32>>` — `K * D * 4` bytes
- Decoder output — variable

**Total** : ≈ `D * 8 + K * D * 4 + K * 4` bytes. Pour D=2048, K=4 :
**~48 KB**, négligeable.

---

## 8. Algorithmes détaillés

### 8.1 `WeightedMeanExtractor::extract`

```rust
pub struct WeightedMeanExtractor {
    pub weighting: WeightingScheme,
}

pub enum WeightingScheme {
    Uniform,
    // Sprint 8+: DistanceInverse, AttentionEntropy
}

impl CentroidExtractor for WeightedMeanExtractor {
    fn extract(
        &self,
        latent: &LatentView<'_>,
        mask: &MaskView<'_>,
    ) -> Result<Vec<f32>> {
        let n = latent.n_agents();
        let d = latent.dim() as usize;
        let mut sum = vec![0.0f32; d];
        let mut weight_total = 0.0f32;
        let mut active = 0u32;

        for i in 0..n {
            if !mask.is_active(i) { continue; }
            active += 1;
            let w = match self.weighting {
                WeightingScheme::Uniform => 1.0,
                // Future: DistanceInverse computes w from h_i and running mean.
            };
            weight_total += w;
            let row = latent.row(i)?;
            for (s, v) in sum.iter_mut().zip(row.iter()) {
                *s += w * v.to_f32();
            }
        }
        if active == 0 || weight_total < f32::EPSILON {
            return Err(EfeError::Internal(
                "WeightedMeanExtractor: no active agents",
            ));
        }
        for v in &mut sum { *v /= weight_total; }
        Ok(sum)
    }

    fn name(&self) -> &'static str { "weighted-mean" }
}
```

### 8.2 `MedianCoordinateExtractor::extract`

```rust
pub struct MedianCoordinateExtractor;

impl CentroidExtractor for MedianCoordinateExtractor {
    fn extract(
        &self,
        latent: &LatentView<'_>,
        mask: &MaskView<'_>,
    ) -> Result<Vec<f32>> {
        let n = latent.n_agents();
        let d = latent.dim() as usize;

        // Collect active rows as f32.
        let mut active_rows: Vec<Vec<f32>> = Vec::new();
        for i in 0..n {
            if !mask.is_active(i) { continue; }
            let row = latent.row(i)?;
            active_rows.push(row.iter().map(|v| v.to_f32()).collect());
        }
        if active_rows.is_empty() {
            return Err(EfeError::Internal(
                "MedianCoordinateExtractor: no active agents",
            ));
        }

        let mut result = vec![0.0f32; d];
        let mut coord = Vec::<f32>::with_capacity(active_rows.len());
        for j in 0..d {
            coord.clear();
            for row in &active_rows {
                coord.push(row[j]);
            }
            coord.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mid = coord.len() / 2;
            result[j] = if coord.len() % 2 == 0 {
                0.5 * (coord[mid - 1] + coord[mid])
            } else {
                coord[mid]
            };
        }
        Ok(result)
    }

    fn name(&self) -> &'static str { "median-coord" }
}
```

### 8.3 `StrictAlignmentProjector::project`

```rust
pub struct StrictAlignmentProjector;

impl AlignmentProjector for StrictAlignmentProjector {
    fn project(
        &self,
        centroid_raw: &[f32],
        anchors: &[Vec<f32>],
        cone_radius: f32,
        config: &ExtractionConfig,
    ) -> Result<ProjectionOutcome> {
        if anchors.is_empty() {
            return Err(EfeError::AlignmentBreach {
                delta: f32::INFINITY,
                max: cone_radius,
            });
        }

        // Compute positive cosine weights.
        let mut weights = Vec::with_capacity(anchors.len());
        let mut total = 0.0f32;
        for anchor in anchors {
            if anchor.len() != centroid_raw.len() {
                return Err(EfeError::InvalidConfig(format!(
                    "anchor dim {} != centroid dim {}",
                    anchor.len(), centroid_raw.len()
                )));
            }
            let cos = cosine_similarity(centroid_raw, anchor);
            let w = cos.max(0.0);
            weights.push(w);
            total += w;
        }
        if total < f32::EPSILON {
            return Err(EfeError::AlignmentBreach {
                delta: f32::INFINITY,
                max: cone_radius,
            });
        }
        for w in &mut weights { *w /= total; }

        // Anchor centroid.
        let mut anchor_centroid = vec![0.0f32; centroid_raw.len()];
        for (anchor, &w) in anchors.iter().zip(weights.iter()) {
            for (ac, a) in anchor_centroid.iter_mut().zip(anchor.iter()) {
                *ac += w * a;
            }
        }

        // Projected = (1-α) * raw + α * anchor_centroid.
        let alpha = config.alignment_strength;
        let mut projected = Vec::with_capacity(centroid_raw.len());
        for (r, ac) in centroid_raw.iter().zip(anchor_centroid.iter()) {
            projected.push((1.0 - alpha) * r + alpha * ac);
        }
        let delta = l2_distance(centroid_raw, &projected);

        // Strict breach check.
        if delta > cone_radius * (1.0 + config.breach_tolerance) {
            return Err(EfeError::AlignmentBreach {
                delta,
                max: cone_radius,
            });
        }

        Ok(ProjectionOutcome {
            projected_centroid: projected,
            distance_from_raw: delta,
            anchor_weights: weights,
        })
    }

    fn name(&self) -> &'static str { "strict-alignment" }
}
```

### 8.4 `IdentityDecoder::decode`

```rust
#[derive(Debug, Default, Clone)]
pub struct IdentityDecoder;

#[async_trait]
impl OutputDecoder for IdentityDecoder {
    async fn decode(
        &self,
        projected: &[f32],
        _weights: &[f32],
    ) -> Result<DecodedOutput> {
        Ok(DecodedOutput::Embedding(projected.to_vec()))
    }

    fn name(&self) -> &'static str { "identity" }
}
```

### 8.5 `NearestAnchorDecoder::decode` (Sprint 8)

```rust
pub struct NearestAnchorDecoder {
    pub labels: Arc<Vec<String>>,
    /// Copy of the anchors, for offline comparison at decode time.
    pub anchors: Arc<Vec<Vec<f32>>>,
}

#[async_trait]
impl OutputDecoder for NearestAnchorDecoder {
    async fn decode(
        &self,
        projected: &[f32],
        _weights: &[f32],
    ) -> Result<DecodedOutput> {
        if self.anchors.is_empty() {
            return Err(EfeError::Internal("NearestAnchorDecoder: no anchors"));
        }
        let (best_idx, best_score) = self.anchors.iter()
            .enumerate()
            .map(|(i, a)| (i, cosine_similarity(projected, a)))
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();
        Ok(DecodedOutput::AnchorLabel {
            index: best_idx as u32,
            score: best_score,
        })
    }

    fn name(&self) -> &'static str { "nearest-anchor" }
}
```

### 8.6 `MvpProjector::extract` orchestration

```rust
pub struct MvpProjector {
    engine: DissolutionEngine,
}

#[async_trait]
impl OutputProjector for MvpProjector {
    async fn extract(
        &self,
        els: &EpistemicLatentSpace,
        arena: &Arena,
        config: &ExtractionConfig,
        ctx: &ExtractionContext,
    ) -> Result<OutputBundle> {
        let start = Instant::now();
        config.validate()?;

        // Convergence threshold cross-check.
        if ctx.convergence_confidence < config.min_confidence {
            return Err(EfeError::Internal(
                "extract: confidence below min_confidence",
            ));
        }

        // Load views.
        let latent = els.latent_view(arena)?.ok_or(EfeError::Internal(
            "extract: ELS missing latent",
        ))?;
        let mask = els.mask_view(arena)?.ok_or(EfeError::Internal(
            "extract: ELS missing mask",
        ))?;
        let anchors_view = els.anchors_view(arena)?.ok_or(EfeError::Internal(
            "extract: ELS missing anchors",
        ))?;
        let anchors_host = anchors_view.to_host();

        // Phase 1: extract.
        let t_extract = Instant::now();
        let centroid_raw = self.engine.extractor.extract(&latent, &mask)?;
        let extract_us = t_extract.elapsed().as_micros() as u32;

        // Phase 2: project.
        //
        // Cone radius is passed via the ExtractionContext or via a
        // separate config field. Here we treat the cone's max_radius
        // as an ELS-provided attribute — M8 retrieves it via
        // alignment_cone accessors on the ELS.
        let cone_radius = /* retrieved from SwarmSpec or ELS metadata */ ;
        let t_project = Instant::now();
        let projection = self.engine.projector.project(
            &centroid_raw, &anchors_host, cone_radius, config,
        )?;
        let project_us = t_project.elapsed().as_micros() as u32;

        // Phase 3: decode.
        let t_decode = Instant::now();
        let decoded = self.engine.decoder.decode(
            &projection.projected_centroid,
            &projection.anchor_weights,
        ).await?;
        let decode_us = t_decode.elapsed().as_micros() as u32;

        let total_us = start.elapsed().as_micros() as u32;
        let bundle = OutputBundle {
            swarm_id: ctx.swarm_id,
            centroid_raw,
            centroid_projected: projection.projected_centroid,
            confidence: ctx.convergence_confidence,
            alignment_distance: projection.distance_from_raw,
            anchor_weights: projection.anchor_weights,
            decoded,
            telemetry: ExtractionTelemetry {
                source_round: ctx.final_round,
                extractor_name: self.engine.extractor.name().into(),
                projector_name: self.engine.projector.name().into(),
                decoder_name: self.engine.decoder.name().into(),
                extract_us,
                project_us,
                decode_us,
                total_us,
                n_active_agents: mask.active_count(),
                n_anchors: anchors_view.n_anchors(),
            },
        };
        if config.validate_post {
            bundle.validate()?;
        }
        Ok(bundle)
    }

    fn name(&self) -> &'static str { "mvp-projector" }
}
```

### 8.7 `OutputBundle::validate`

```rust
impl OutputBundle {
    pub fn validate(&self) -> Result<()> {
        // 1. Centroid dims match.
        if self.centroid_raw.len() != self.centroid_projected.len() {
            return Err(EfeError::Internal(
                "OutputBundle: dim mismatch raw/projected",
            ));
        }
        // 2. All elements finite.
        if !self.centroid_raw.iter().all(|x| x.is_finite()) {
            return Err(EfeError::Internal(
                "OutputBundle: centroid_raw has non-finite",
            ));
        }
        if !self.centroid_projected.iter().all(|x| x.is_finite()) {
            return Err(EfeError::Internal(
                "OutputBundle: centroid_projected has non-finite",
            ));
        }
        // 3. Weights sum to ~1 (or zero — edge case).
        let sum: f32 = self.anchor_weights.iter().sum();
        if !sum.is_finite() || (sum > 0.0 && (sum - 1.0).abs() > 1e-3) {
            return Err(EfeError::Internal(
                "OutputBundle: anchor_weights sum not 1",
            ));
        }
        // 4. Confidence bounded.
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err(EfeError::Internal(
                "OutputBundle: confidence out of [0, 1]",
            ));
        }
        // 5. Alignment distance finite.
        if !self.alignment_distance.is_finite() {
            return Err(EfeError::Internal(
                "OutputBundle: alignment_distance non-finite",
            ));
        }
        Ok(())
    }
}
```

---

## 9. Gestion d'anomalies et qualité

### 9.1 Tableau : erreurs × actions M1

| Erreur M8 | Cause | Action M1 |
|---|---|---|
| `AlignmentBreach { delta, max }` | centroïde hors du cône après projection | Retourner erreur au client ; swarm échec politique |
| `Internal("extract: confidence below min_confidence")` | M6 a convergé mais < seuil | Retourner erreur (policy: don't trust low-confidence output) |
| `Internal("extract: ELS missing …")` | Stub ELS | Bug upstream, fail fast |
| `Internal("WeightedMeanExtractor: no active agents")` | Mask tout-inactif | Respawn M3 ou abort |
| `InvalidConfig(…)` | Config cassée | Rejeter avant même d'essayer |
| `EfeError::Internal` du decoder | Decoder externe HS | Fallback identity ou respawn |

### 9.2 Signaux de qualité exposés

`OutputBundle` expose plusieurs métriques de qualité :

- `confidence` — de M6, propagé tel quel.
- `alignment_distance / cone_radius` — "quality of alignment" (0 = parfait,
  1 = à la limite, >1 = breach = déjà détecté via `AlignmentBreach`).
- `anchor_weights` — distribution sur les anchors ; si une seule est
  dominante, output déterministe ; si uniforme, output ambigu.
- `n_active_agents / spec.population.total` — fraction du swarm qui a
  contribué. Faible = risque de biais.

### 9.3 Post-extraction degradation detection

Sprint 9+ : `PostExtractionGuard` vérifie que le bundle est "plausible"
par comparaison à une distribution de bundles passés. Sort de scope MVP.

---

## 10. Performance et budget

### 10.1 Budget MVP

Pour `N=128, D=2048, K=4` :

| Étape | Budget | Complexité |
|---|---|---|
| `WeightedMeanExtractor` | ~3 ms | O(N·D) |
| `MedianCoordinateExtractor` | ~10 ms | O(N·D·log N) |
| `StrictAlignmentProjector` | ~0.5 ms | O(K·D) |
| `IdentityDecoder` | <0.1 ms | O(D) copy |
| **Total (mean+identity)** | **~4 ms** | — |

Sous budget 20 ms. Marge pour decoders externes Sprint 9.

### 10.2 Mémoire

**Per-extraction peak** : ~50 KB (voir §7.3). Pas de scratch persistant.

### 10.3 Concurrence

`OutputProjector` est `Send + Sync` → peut être partagé via `Arc` entre
swarms concurrents. Pas de Mutex nécessaire (read-only sur ELS).

---

## 11. Interface publique

### 11.1 Façade `efe_core::dissolution`

```rust
pub use dissolution::config::ExtractionConfig;
pub use dissolution::bundle::{DecodedOutput, OutputBundle};
pub use dissolution::telemetry::ExtractionTelemetry;
pub use dissolution::projector::{ExtractionContext, OutputProjector};
pub use dissolution::alignment::{AlignmentProjector, ProjectionOutcome, StrictAlignmentProjector};
pub use dissolution::decoders::{IdentityDecoder, NearestAnchorDecoder, OutputDecoder};
pub use dissolution::extractors::{
    CentroidExtractor, MedianCoordinateExtractor, WeightedMeanExtractor, WeightingScheme,
};
pub use dissolution::engine::{DissolutionEngine, DissolutionEngineBuilder, MvpProjector};
```

### 11.2 Exemple — intégration controller

```rust
let projector: Arc<dyn OutputProjector> = Arc::new(
    MvpProjector::builder()
        .with_extractor(Arc::new(WeightedMeanExtractor {
            weighting: WeightingScheme::Uniform,
        }))
        .with_projector(Arc::new(StrictAlignmentProjector))
        .with_decoder(Arc::new(IdentityDecoder))
        .build()?
);

// After M6 emits Converged:
if let ConsensusVerdict::Converged { confidence } = verdict {
    let ctx = ExtractionContext {
        final_round: round,
        convergence_confidence: confidence,
        swarm_id: els.swarm_id(),
    };
    let cfg = ExtractionConfig::default();
    let bundle = projector.extract(&els, &arena, &cfg, &ctx).await?;
    // Pass bundle to M12 for packaging
}
```

### 11.3 Custom extractor

```rust
#[derive(Debug)]
struct MyExtractor;

impl CentroidExtractor for MyExtractor {
    fn extract(&self, latent: &LatentView<'_>, mask: &MaskView<'_>) -> Result<Vec<f32>> {
        // custom logic
        todo!()
    }
    fn name(&self) -> &'static str { "my-extractor" }
}

let engine = DissolutionEngine::builder()
    .with_extractor(Arc::new(MyExtractor))
    .build()?;
```

---

## 12. Stratégie de test

### 12.1 Plan de couverture

| Niveau | Exemples |
|---|---|
| Unit extractors | mean on uniform data, median on skewed data |
| Unit projector | orthogonal → breach, aligned → ok, empty anchors → breach |
| Unit decoders | identity passes through, nearest matches expected |
| Unit bundle | validate rejects NaN, dim mismatch, non-normalized weights |
| Integration engine | extract → project → decode full pipeline |
| E2E M5→M6→M8 | real convergence → bundle |
| Determinism | same ELS → same bundle bytes |
| Adversarial | empty mask, unit-dim latent, single-anchor, single-agent |

### 12.2 Tests unitaires

Listes exhaustives :

- `weighted_mean_uniform_all_ones_returns_ones`
- `weighted_mean_returns_active_mean_only`
- `weighted_mean_rejects_no_active_agents`
- `weighted_mean_handles_bf16_precision_loss`
- `median_centroid_on_skewed_data`
- `median_even_count_returns_midpoint_average`
- `strict_projector_returns_raw_when_aligned`
- `strict_projector_emits_breach_when_far`
- `strict_projector_rejects_empty_anchors`
- `strict_projector_handles_orthogonal_centroid`
- `strict_projector_respects_tolerance`
- `identity_decoder_preserves_embedding`
- `nearest_anchor_decoder_picks_best_cosine` (Sprint 8)
- `bundle_validate_rejects_nan`
- `bundle_validate_rejects_dim_mismatch`
- `bundle_validate_accepts_empty_weights_for_zero_anchors`
- `config_validates_alignment_strength_range`
- `config_rejects_zero_deadline`

### 12.3 Tests property-based

```rust
proptest! {
    #[test]
    fn centroid_mean_is_bounded_by_input_range(
        n in 1..32usize,
        d in 1..16usize,
        scale in -100.0..100.0f32,
    ) {
        // Build latent filled with `scale`; centroid must equal `scale`.
    }

    #[test]
    fn projected_never_exceeds_cone_radius_after_breach_check(
        alpha in 0.0..=1.0f32,
    ) {
        // Synthetic centroid + anchors; after successful project, delta ≤ cone_radius.
    }

    #[test]
    fn identity_decoder_preserves_length(d in 1..64usize) {
        // Decode then compare lengths.
    }
}
```

### 12.4 Tests E2E

```rust
#[tokio::test]
async fn full_pipeline_extracts_bundle_on_convergence() {
    let (batch, mut els) = spawn_swarm(16, 32).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = default_fusion();
    let mut detector = MultiCriteriaDetector::new(ConsensusConfig::default())?;
    let cfg = FusionConfig::default();

    // Run until converged.
    let mut final_confidence = None;
    for r in 0..cfg.max_rounds {
        let round = fusion.run_round(&mut els, arena, &cfg, r).await?;
        let verdict = detector.ingest(&round.stats)?;
        if let ConsensusVerdict::Converged { confidence } = verdict {
            final_confidence = Some((r, confidence));
            break;
        }
    }

    let (final_round, conf) = final_confidence.expect("must converge");
    let projector = MvpProjector::mvp_default();
    let ctx = ExtractionContext {
        final_round,
        convergence_confidence: conf,
        swarm_id: els.swarm_id(),
    };
    let cfg = ExtractionConfig::default();
    let bundle = projector.extract(&els, arena, &cfg, &ctx).await?;
    assert!(bundle.validate().is_ok());
    assert_eq!(bundle.centroid_projected.len(), 16);
    assert!(bundle.alignment_distance >= 0.0);
}
```

### 12.5 Tests d'alignement (adversarial)

```rust
#[tokio::test]
async fn extraction_rejects_orthogonal_centroid() {
    // Build a synthetic ELS where centroid is orthogonal to all anchors.
    // Assert AlignmentBreach.
}

#[tokio::test]
async fn extraction_rejects_below_min_confidence() {
    let ctx = ExtractionContext { convergence_confidence: 0.1, ... };
    let cfg = ExtractionConfig { min_confidence: 0.5, ... };
    let err = projector.extract(&els, arena, &cfg, &ctx).await.unwrap_err();
    assert!(matches!(err, EfeError::Internal(_)));
}
```

---

## 13. Dépendances inter-modules

### 13.1 Crates

```toml
[dependencies]
serde = { workspace = true, features = ["derive"] }
thiserror.workspace = true
async-trait = "0.1"
tokio = { workspace = true, features = ["rt"] }

[dev-dependencies]
proptest = "1.4"
```

### 13.2 Contrats avec M4/M5/M6

- **M4** : consomme `LatentView`, `MaskView`, `AnchorsView` (read-only).
  Appelle `to_host()` sur anchors. Utilise `els::metrics::{cosine_similarity, l2_distance}`.
- **M5** : pas de lien direct. M8 s'exécute après M5 a terminé ses rounds.
- **M6** : consomme `confidence` de `Converged { confidence }`. Refuse
  d'extraire si < `min_confidence`.
- **M7** : indirect — si M7 fait un rollback + M6 converge ensuite, M8
  extrait depuis l'état post-rollback (transparent).

### 13.3 Contrats avec M9 (Alignment)

M8 lit les anchors **via M4** (`els.anchors_view(arena)`). Le
`cone_radius` vient du `SwarmSpec::alignment_cone`, accessible via
`EpistemicLatentSpace::alignment_cone_radius()` (nouvelle méthode à
ajouter pour M8).

**Extension requise à `EpistemicLatentSpace`** : ajouter un accesseur
`pub fn alignment_cone_radius(&self) -> f32` qui retourne la valeur
stockée depuis le spec (ou persistée).

### 13.4 Contrat avec M11 & M12

- **M11** : sérialise `ExtractionTelemetry` et `OutputBundle` (sans
  `centroid_raw` si trop gros).
- **M12** : receives `OutputBundle`, serializes `DecodedOutput`
  dispatch vers le protocole (gRPC `oneof`, REST JSON, etc.).

---

## 14. Plan de sprint M8

### 14.1 Sprint 7 — MVP (1 semaine)

**J1** :
- Scaffolding `dissolution/` complet.
- `ExtractionConfig`, `ExtractionContext`, `OutputBundle`, `DecodedOutput`.
- Tests unit types.

**J2** :
- Trait `CentroidExtractor` + `WeightedMeanExtractor` + tests.
- Ajouter `alignment_cone_radius()` à `EpistemicLatentSpace`.

**J3** :
- Trait `AlignmentProjector` + `StrictAlignmentProjector` + tests.
- Tests breach detection.

**J4** :
- Trait `OutputDecoder` + `IdentityDecoder` + tests.
- `DissolutionEngine` + `MvpProjector` orchestration.

**J5** :
- `OutputBundle::validate()` + `ExtractionTelemetry`.
- Tests E2E M5→M6→M8.

**J6** :
- `MedianCoordinateExtractor` + tests.
- Benchmarks criterion.

**J7** :
- Doc complète, audit, clippy pédantique.

### 14.2 Sprint 8 — Decoders enrichis (0.5 semaine)

- `NearestAnchorDecoder` avec label mapping.
- Distance-inverse weighting scheme.
- Post-extraction quality scoring.

### 14.3 Sprint 9+ — Decoders externes

- HuggingFace tokenizer integration.
- ONNX runtime wrapper.
- Fréchet mean (iterative geodesic).

---

## 15. Critères d'acceptation

### 15.1 Fonctionnels

- [ ] Un swarm convergé à confidence ≥ 0.8 → extraction returns
  `OutputBundle::validate().is_ok()`.
- [ ] Un centroïde orthogonal au cône → `AlignmentBreach`.
- [ ] Confidence < `min_confidence` → `Internal` rejection avant extract.
- [ ] `IdentityDecoder` retourne bundle.centroid_projected verbatim.
- [ ] Pipeline E2E M3→M5→M6→M8 produit un bundle cohérent.
- [ ] Determinism : même ELS → même bundle (hash-comparable).

### 15.2 Qualité

- [ ] `cargo fmt --all` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean pedantic.
- [ ] `cargo doc --no-deps --workspace` clean.
- [ ] Couverture ≥ 90% sur `dissolution/`.
- [ ] Zéro `unsafe` dans M8 (délègue aux vues M4).
- [ ] Toutes les traits publics documentées.

### 15.3 Performance

- [ ] Extract+project+identity-decode < 5 ms pour N=128, D=2048.
- [ ] Median extractor < 15 ms pour N=128, D=2048.
- [ ] Memory footprint par extraction < 100 KB.

### 15.4 Observabilité

- [ ] `ExtractionTelemetry` contient tous les timings par phase.
- [ ] `OutputBundle` serialize en JSON compact (<2KB sans centroids).
- [ ] Tracing spans `dissolution.extract`, `dissolution.project`, `dissolution.decode`.

---

## 16. Risques et mitigations

| # | Risque | Proba | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Breach fires sur faux positif (centroïde légitime mais proche frontière) | Moyen | Élevé | `breach_tolerance=0.05` par défaut, tunable via config. |
| R2 | Median extractor O(NDlogN) ne passe pas le budget | Bas | Moyen | Benchmarks CI. Fallback sur weighted-mean si timeout. |
| R3 | `IdentityDecoder` ne produit rien d'utile en prod | Moyen | Bas | MVP assumption — clients décodent eux-mêmes. Sprint 8 ajoute vrais decoders. |
| R4 | bf16 precision loss sur centroid | Bas | Moyen | Accumulation en f32, sortie en f32. Tests precision. |
| R5 | Extraction sur swarm non-convergé produit garbage | Moyen | Élevé | `min_confidence` gate bloque. |
| R6 | Anchors vide → tout casse | Bas | Élevé | Check explicit dans strict projector. |
| R7 | n_active=0 (tout désactivé après respawn) | Bas | Élevé | Extract rejette avec Internal. |
| R8 | Concurrent extractions sur même ELS | Bas | Bas | `&EpistemicLatentSpace` → read-only, safe. |
| R9 | Decoder async block controller too long | Moyen | Moyen | `deadline_ms` enforced. Decoders implementent timeout. |
| R10 | Alignment cone semantics shift (M9 evolves) | Bas | Moyen | Stable API : M8 lit `cone_radius` via accessor M4/M9. |

---

## 17. Annexes

### 17.1 Glossaire

| Terme | Définition |
|---|---|
| **Dissolution** | Étape finale : transformer le swarm convergé en réponse. |
| **Centroid (raw)** | Moyenne/médiane des latents actifs, avant projection. |
| **Alignment projection** | Blend entre centroïde raw et centroïde des anchors pondéré. |
| **Breach** | Violation du cône `max_radius` après projection. |
| **Decoder** | Dernier stage : transforme le f32 projeté en format final. |
| **Identity decoder** | Passthrough : renvoie l'embedding tel quel. |
| **Anchor weights** | Poids normalisés (somme=1) attribués à chaque anchor. |
| **Alignment distance** | `||centroid_raw - centroid_projected||_2`. |
| **Cone radius** | Distance max tolérée entre raw et projected. |
| **Bundle** | Objet sérialisable regroupant toutes les sorties de M8. |

### 17.2 Pseudo-code complet `extract()`

```
function extract(els, arena, cfg, ctx):
    cfg.validate()

    if ctx.convergence_confidence < cfg.min_confidence:
        return Err(Internal)

    latent = els.latent_view(arena)
    mask = els.mask_view(arena)
    anchors_view = els.anchors_view(arena)
    anchors_host = anchors_view.to_host()
    cone_radius = els.alignment_cone_radius()

    t_extract = now()
    centroid_raw = engine.extractor.extract(latent, mask)
    extract_us = elapsed(t_extract)

    t_project = now()
    projection = engine.projector.project(centroid_raw, anchors_host, cone_radius, cfg)
    project_us = elapsed(t_project)

    t_decode = now()
    decoded = engine.decoder.decode(projection.projected_centroid, projection.anchor_weights)
    decode_us = elapsed(t_decode)

    bundle = OutputBundle {
        swarm_id: ctx.swarm_id,
        centroid_raw,
        centroid_projected: projection.projected_centroid,
        confidence: ctx.convergence_confidence,
        alignment_distance: projection.distance_from_raw,
        anchor_weights: projection.anchor_weights,
        decoded,
        telemetry: …,
    }

    if cfg.validate_post:
        bundle.validate()?

    return Ok(bundle)
```

### 17.3 Exemple numérique

**Setup** : `N=4, D=2`, mask all-active, centroids = `[(1,1), (1,2), (2,1), (2,2)]`.
Anchor = `[(1.5, 1.5)]`, cone_radius = `0.5`, α = `0.5`.

**Étape 1 (mean)** :
```
centroid_raw = mean = [1.5, 1.5]
```

**Étape 2 (project)** :
```
cos(centroid_raw, anchor) = cos([1.5, 1.5], [1.5, 1.5]) = 1.0
w = 1.0, total = 1.0, w_norm = 1.0
anchor_centroid = 1.0 * [1.5, 1.5] = [1.5, 1.5]
projected = 0.5 * [1.5, 1.5] + 0.5 * [1.5, 1.5] = [1.5, 1.5]
delta = 0.0 ≤ 0.5 (cone_radius), OK
```

**Étape 3 (identity decode)** :
```
decoded = Embedding([1.5, 1.5])
```

**Bundle** : `centroid_raw=[1.5,1.5], centroid_projected=[1.5,1.5], alignment_distance=0.0`.

### 17.4 Mapping metrics (M11)

| Champ | Label metric |
|---|---|
| `extractor_name` | `extractor="weighted_mean"` |
| `projector_name` | `projector="strict_alignment"` |
| `decoder_name` | `decoder="identity"` |
| `alignment_distance / cone_radius` | histogram `alignment_quality` |
| `confidence` | histogram `final_confidence` |
| AlignmentBreach triggered | counter `breach_total` |

### 17.5 Invariants formels

- Si `Ok(bundle)` : `bundle.validate().is_ok()`.
- `∀i : bundle.centroid_projected[i].is_finite()`.
- `Σ bundle.anchor_weights ∈ {0.0} ∪ [1-ε, 1+ε]`.
- `bundle.alignment_distance ≤ cone_radius * (1 + tolerance)`.
- `bundle.confidence ∈ [cfg.min_confidence, 1]`.
- `telemetry.total_us ≤ cfg.deadline_ms * 1000` (or DeadlineExceeded).

### 17.6 Diagramme d'état d'une extraction

```
  ┌────────────────┐
  │    START       │
  └────────┬───────┘
           │
           ▼
  ┌────────────────┐
  │ validate cfg   │ ─X→ InvalidConfig
  └────────┬───────┘
           │
           ▼
  ┌────────────────┐
  │ min_confidence │ ─X→ Internal (confidence too low)
  └────────┬───────┘
           │
           ▼
  ┌────────────────┐
  │ load views     │ ─X→ Internal (ELS stub)
  └────────┬───────┘
           │
           ▼
  ┌────────────────┐
  │ extract        │ ─X→ Internal (no active agents)
  └────────┬───────┘
           │
           ▼
  ┌────────────────┐
  │ project strict │ ─X→ AlignmentBreach
  └────────┬───────┘
           │
           ▼
  ┌────────────────┐
  │ decode         │ ─X→ decoder-specific error
  └────────┬───────┘
           │
           ▼
  ┌────────────────┐
  │ validate_post  │ ─X→ Internal (invariant broken)
  └────────┬───────┘
           │
           ▼
  ┌────────────────┐
  │ Return Bundle  │
  └────────────────┘
```

### 17.7 Checklist d'audit post-implémentation

- [ ] Chaque trait a au moins 3 tests unitaires.
- [ ] `validate()` sur bundle rejette tous les cas d'invariants cassés.
- [ ] Median et mean produisent des résultats identiques sur données
  symétriques.
- [ ] Sprite decoders sont testés en isolation.
- [ ] `ExtractionConfig::default()` passe `validate()`.
- [ ] Pipeline E2E complet M5→M6→M8 termine en <100ms pour swarm typique.
- [ ] Bundle est sérialisable en JSON **et** bincode.
- [ ] Signature publique de `OutputProjector` permet trait objects.
- [ ] Aucune allocation hot-path après premier call (si scratch pool activé).
- [ ] Tracing spans correctement namespacés.

---

**Fin du document M8 ARCHITECTURE.md** — prêt pour implémentation sprint 7.

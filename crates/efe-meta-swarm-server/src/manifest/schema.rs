//! YAML schema types for the manifest.

use serde::{Deserialize, Serialize};

/// Top-level manifest.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub version: String,
    pub meta: MetaSpaceManifest,
    #[serde(default)]
    pub participants: Vec<ParticipantManifest>,
    #[serde(default)]
    pub iteration: IterationManifest,
}

/// Meta-space config.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetaSpaceManifest {
    pub dim: u32,
    pub safety: SafetyManifest,
}

/// Safety geometry — tagged enum (kind: box | poly | hull).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SafetyManifest {
    Box {
        radius: f32,
    },
    Poly {
        dim_a: u32,
        a_flat: Vec<f32>,
        b: Vec<f32>,
    },
    Hull {
        version: String,
        serialized_model_path: String,
    },
}

/// Participant declaration.
///
/// **Engines (Couche 3 M1.d).** A row declares exactly ONE of two engines :
///
/// * **mock** (`controller` absent) — the Phase-2 synthetic participant
///   `Φ(x) = κ·x`.  `kappa` and `seed` are REQUIRED (validated in
///   `validate_manifest`).  Byte-identical to the historical behaviour.
/// * **efe-core** (`controller` present) — a REAL efe-core latent swarm
///   (Mode-1 numerical fusion on stub templates — **not** an SLM).  `kappa`
///   and `seed` are mock-only concepts and MUST be **absent** (fail-closed) ;
///   the block is identified on the wire by its `config_hash` instead.
///
/// Making `kappa`/`seed` optional is itself a **fail-closed** design choice :
/// an efe-core row *omits* them, so a pre-M1.d participant binary (which
/// requires both fields) refuses to parse the manifest at all — a real row
/// can never silently degrade to the κ·x mock on an old binary.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParticipantManifest {
    pub id: String,
    pub dim: u32,
    /// Mock-engine contraction constant.  REQUIRED for mock rows ;  FORBIDDEN
    /// for efe-core rows (see `validate_manifest`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kappa: Option<f32>,
    /// Mock-engine initial-state seed.  Same presence rules as `kappa`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default = "default_blas")]
    pub blas_backend: String,
    /// Couche 3 M1.d — the REAL efe-core engine block.  Absent ⇒ mock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<ControllerManifest>,
}

fn default_blas() -> String {
    "none".into()
}

/// Couche 3 M1.d — versioned declaration of a REAL efe-core participant
/// engine.  Feeds the server-side latent assembly in `participant::core_engine`
/// + a `SwarmSpec` + the participant's safety box.  The `topology` field picks
/// the recipe : `fixed` (default) → `build_fixed_topology_controller`
/// (x-independent `population` plan) ;  `dynamic` → `build_mvp_controller`
/// (heuristic meta-analyzer/router/cardinality, `population` shapes only the
/// spec).
///
/// `PartialEq` is derived because the block is compared verbatim for
/// **reload drift** (a SIGHUP that edits it is rejected fail-closed — the
/// running participant keeps its boot-time engine, exactly like the mock's
/// `dim`/`kappa`/`seed` immutability).
///
/// # Honesty (never over-sell)
///
/// The engine this block builds coordinates a **numerical Mode-1 fusion on
/// stub templates** : no LM weights, no tokenizer on the step path.  Its Φ is
/// deterministic (M1.b : zero-noise genesis) and its non-expansiveness is
/// **property-tested, not certified** (M1.c canary — see `h1_canary`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ControllerManifest {
    /// Schema version of THIS block.  Only `1` is understood ;  any other
    /// value is rejected fail-closed (`validate_manifest`).
    pub schema: u32,
    /// Population mix : one row per template kind.
    pub population: Vec<PopulationManifest>,
    /// `SwarmSpec.max_rounds` (the participant step itself always runs the
    /// single-round Φ — this bounds the spec, not the step).
    pub max_rounds: u32,
    /// Alignment cone of the spec (`AlignmentCone`).
    pub alignment_cone: AlignmentConeManifest,
    /// The participant's SAFETY BOX — its state projection domain AND the
    /// sampling domain of the H1 Lipschitz canary.  Per-coordinate uniform
    /// bounds `[lo, hi]` (e.g. `[0.05, 0.30]`, inside the alignment cone).
    pub safety_box: SafetyBoxManifest,
    /// **M1 Palier-2 — genesis topology class.**  Default `fixed` (the safe
    /// profile) : the swarm population — agent count, template mix, active
    /// mask — is the CONSTANT declared in `population`, independent of the
    /// input `x`, so the operator `Φ` is **continuous by construction** (the
    /// dynamic genesis routing's integer jumps — the one mechanism that can
    /// break H1 — are eliminated).  `dynamic` opts back into the heuristic
    /// meta-analyzer/router/cardinality pipeline : `population` then only
    /// shapes the `SwarmSpec`, the actual topology varies with `x`, and H1
    /// rests SOLELY on the empirical canary (logged loudly at boot).
    #[serde(default)]
    pub topology: TopologyManifest,
    /// Tenant attribution forwarded to efe-core telemetry.
    pub tenant_id: String,
    /// H1 boot canary policy.  Defaults to `mode: boot` (fail-closed honest).
    #[serde(default)]
    pub h1_canary: H1CanaryManifest,
}

/// One `(template, count)` row of the population mix.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PopulationManifest {
    /// Stable template code (e.g. `"na-sem"` — parsed by
    /// `TemplateCode::from_str` at construction, fail-closed on unknown).
    pub template: String,
    pub count: u32,
}

/// Alignment cone of the participant's `SwarmSpec`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AlignmentConeManifest {
    /// Anchor vectors ;  each must have length == the participant `dim`.
    pub anchors: Vec<Vec<f32>>,
    pub max_projection_distance: f32,
}

/// Per-coordinate uniform safety box `[lo, hi]^dim`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SafetyBoxManifest {
    pub lo: f32,
    pub hi: f32,
}

/// M1 Palier-2 — genesis topology class of an efe-core row.
///
/// `Fixed` removes the *discontinuity* threat to H1 (continuity by
/// construction) — it does **not** certify a contraction modulus : the only
/// honest H1 evidence remains the measured canary bound (property-tested,
/// NOT certified).  The architecture doc's closed-form `≤ β₁ ≈ 0.17` claim
/// is deliberately NOT minted as a certificate : the M1.c canary measured
/// `L̂ ≈ 0.37` on the fixed operator (and `≈ 0.69` on the dynamic one) —
/// both above `β₁`, so measure, don't assume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TopologyManifest {
    /// Population plan honoured EXACTLY, independent of `x` (default).
    #[default]
    Fixed,
    /// Heuristic (x-dependent) genesis routing — H1 unproven by
    /// construction ;  explicitly opted into, logged at boot.
    Dynamic,
}

/// H1 (non-expansiveness) boot-canary policy for an efe-core row.
///
/// `mode: boot` (default) runs the M1.c adversarial Lipschitz canary once at
/// participant boot, BEFORE serving ;  a certificate that is inconclusive or
/// measures `L̂ ≥ 1 − margin` **refuses to serve** (fail-closed).  `mode: off`
/// skips it — the manifest then carries an explicitly unverified participant
/// (logged loudly ;  intended for tests and for deployments that run the
/// canary out-of-band).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct H1CanaryManifest {
    /// `"boot"` (default) or `"off"`.
    pub mode: String,
    /// Probe sizing — kept deliberately small : the real Φ costs ~0.5-3 s per
    /// evaluation, so the DEFAULT boot canary budget is ~60 evaluations
    /// (≈ 0.5-3 min).  A thorough audit should run out-of-band with a larger
    /// probe (see `efe_meta_swarm::participants::LipschitzProbe`).
    pub n_seeds: u32,
    pub n_pairs: u32,
    pub n_directions: u32,
    pub n_bisect_pairs: u32,
    pub bisect_depth: u32,
    pub max_evals: u32,
    /// Acceptance margin : the canary accepts iff `L̂ < 1 − margin`.
    pub margin: f64,
}

impl Default for H1CanaryManifest {
    fn default() -> Self {
        Self {
            mode: "boot".into(),
            n_seeds: 12,
            n_pairs: 32,
            n_directions: 2,
            n_bisect_pairs: 2,
            bisect_depth: 10,
            max_evals: 64,
            margin: 0.05,
        }
    }
}

/// Iteration knobs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct IterationManifest {
    pub max_iters: u32,
    pub tolerance: f64,
    pub alpha_default: f64,
    pub coupling_gamma: f64,
    pub deterministic: bool,
    pub max_wall_ms: u32,
    /// Couche 3 M1.d — **fail-closed non-convergence** (architecture §4).
    /// When `true`, an `Iterate` that stops on `MaxItersHit` returns a gRPC
    /// `ABORTED` error instead of an OK response carrying a non-converged
    /// "consensus" the caller might silently trust.  `StepOnce`
    /// (`max_iters = 1`, definitionally non-converged) is exempt.
    ///
    /// **`None` = not stated, and the default then DEPENDS ON THE ENGINE**
    /// — resolve it with [`Manifest::require_convergence`], never by reading
    /// this field directly.  A flat `false` default was a fail-open on the
    /// only path that can actually diverge : a mock participant is `Φ(x) = κ·x`
    /// with `κ < 1`, so it converges by construction and the guard is moot ;
    /// the real efe-core operator is the one whose contraction is merely
    /// *property-tested* (M1.c canary, `L̂` is a lower bound — it falsifies H1,
    /// it never certifies it).  The soundness hole architecture §4 called a
    /// blocker therefore stayed wide open exactly where it mattered, while the
    /// audit ticked the box because the mechanism existed.
    ///
    /// So: stated wins (an explicit `false` is a legitimate, LOUDLY logged
    /// opt-out — we require the choice to be MADE, not to go a given way) ;
    /// unstated resolves to `true` as soon as any participant row carries a
    /// `controller` block.  v0.2.0 mock manifests carry none, so they keep the
    /// historical lenient behaviour untouched.
    #[serde(default)]
    pub require_convergence: Option<bool>,
}

impl Default for IterationManifest {
    fn default() -> Self {
        Self {
            max_iters: 100,
            tolerance: 1.0e-4,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 15_000,
            require_convergence: None,
        }
    }
}

impl Manifest {
    /// Does this round coordinate at least one **real efe-core participant** ?
    ///
    /// The `controller` block is what `LocalParticipant::from_manifest_row`
    /// turns into an `Engine::EfeCore` bridge ; a row without one is a mock.
    #[must_use]
    pub fn has_efe_core_participant(&self) -> bool {
        self.participants.iter().any(|p| p.controller.is_some())
    }

    /// **The effective fail-closed policy for non-convergence** — resolves
    /// [`IterationManifest::require_convergence`] against the engine actually
    /// being served.  Read this, never the raw field.
    ///
    /// `Some(v)` ⟹ `v` (the operator stated a choice, and it is honoured —
    /// including an explicit `false`, which [`super::load_and_validate`] logs
    /// loudly when a real engine is present).  `None` ⟹ `true` iff at least
    /// one participant carries a `controller` block.
    #[must_use]
    pub fn require_convergence(&self) -> bool {
        self.iteration
            .require_convergence
            .unwrap_or_else(|| self.has_efe_core_participant())
    }
}

impl ControllerManifest {
    /// **La** définition du `config_hash` déclaré : SHA-256 (hex) du JSON canonique du bloc.
    ///
    /// Extraite de `core_engine::build` (Couche 3 v4) parce qu'elle a désormais DEUX
    /// consommateurs : le participant, qui l'annonce sur le fil (M1.d), et le contrôleur, qui la
    /// scelle dans la liaison du reçu. Deux calculs séparés auraient dérivé — et la liaison
    /// aurait attesté un hash que le participant n'annonce pas, incohérence qu'aucun des deux
    /// côtés n'aurait pu détecter seul. Un seul producteur d'octets, donc.
    ///
    /// # Panics
    /// Jamais en pratique : `ControllerManifest` est composé de types serde-sérialisables.
    #[must_use]
    pub fn config_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(self).expect("bloc controller sérialisable");
        let digest = Sha256::digest(canonical.as_bytes());
        let mut out = String::with_capacity(64);
        for b in digest {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }
}

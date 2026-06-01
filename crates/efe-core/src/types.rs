//! Fundamental types shared across all EFE modules.
//!
//! This module defines newtypes for identifiers, configuration structs
//! with validated constructors, and builder patterns for complex specs.
//!
//! # Design principles
//!
//! - **Newtype wrapping**: `SwarmId`, `NanoAgentId`, `LatentDim` prevent
//!   accidental mixing at the type level.
//! - **Non-constructible invariants**: types like [`PopulationMix`] cannot
//!   be built in an invalid state (empty / zero total).
//! - **Builder pattern**: [`SwarmSpecBuilder`] validates cross-field
//!   constraints at `build()` time.
//! - **`#[non_exhaustive]`** on public structs/enums to allow adding
//!   fields/variants without a MAJOR bump.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{EfeError, Result};

// =====================================================================
// Identifiers (newtypes)
// =====================================================================

/// Identifier of a swarm (one per inference request).
///
/// Opaque wrapper around a `UUIDv4`. Generated at spawn time and used for
/// telemetry correlation across all modules.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SwarmId(Uuid);

impl SwarmId {
    /// Generate a fresh random swarm id (UUIDv4).
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Access the raw UUID (for serialization boundaries only).
    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for SwarmId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SwarmId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Ephemeral identifier of a nano-agent within a swarm.
///
/// Indexed from 0 per swarm. Not globally unique; valid only for the
/// lifetime of its parent swarm.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NanoAgentId(pub u32);

/// Opaque request identifier propagated from the API layer.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub String);

impl RequestId {
    /// Create a new random request id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Construct from an externally supplied string (e.g. gRPC header).
    #[must_use]
    pub fn from_raw(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Raw string value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// =====================================================================
// Template code
// =====================================================================

/// Code identifying a nano-agent specialization template.
///
/// ## MVP set (D1-D9, immutable forever)
///
/// `NaSem` (=0), `NaLog` (=1), `NaCrit` (=2). The repr-u8 values are
/// **stable across versions** because the CUDA buffers in `efe-kernels`
/// encode them as `u8`. Reordering would silently corrupt every active
/// catalog.
///
/// ## GA set (Sprint **D33**, M1 extension)
///
/// `NaQual` (=3), `NaCausal` (=4), `NaTemp` (=5), `NaSafety` (=6),
/// `NaConsist` (=7). Five new templates required for the production
/// GA contract — cf. `EFE_CORE_EXTENSION_ARCHITECTURE.md` § 9.
///
/// Each variant has a stable `as_str()` representation used in
/// configuration files (catalog YAML), telemetry labels, and audit
/// events. Renaming = breaking change → major version bump.
///
/// The `#[non_exhaustive]` attribute lets us add future templates
/// (post-GA) without breaking pattern-matching consumers — they
/// always have a `_ => fallback` arm.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
#[non_exhaustive]
pub enum TemplateCode {
    /// Semantic density template (MVP).
    NaSem = 0,
    /// Logical inference template (MVP).
    NaLog = 1,
    /// Critique / diversification template (MVP).
    NaCrit = 2,

    // ──────────────────────────────────────────────────────────────────
    // Sprint **D33** — GA set extension. Numeric values are immutable.
    // ──────────────────────────────────────────────────────────────────
    /// Quality-focused template (D33). Optimises output coherence,
    /// fluency, pertinence. Default share 15 %. Consensus weight 1.2.
    NaQual = 3,
    /// Causal-reasoning template (D33). Specialises on directed
    /// causal relations (X → Y) vs spurious correlations. Default
    /// share 10 %. Consensus weight 1.0.
    NaCausal = 4,
    /// Temporal-aware template (D33). Encodes ordering, durations,
    /// recurrence, timeline reasoning. Default share 8 %. Consensus
    /// weight 1.0.
    NaTemp = 5,
    /// Safety / refusal template (D33). Detects harmful, illegal,
    /// alignment-violating content. Default share 5 % — small but
    /// always present. Consensus weight 2.0 (a refusal vote
    /// outweighs most positive votes).
    NaSafety = 6,
    /// Internal-consistency template (D33). Detects contradictions
    /// inside a single response. Default share 7 %. Consensus
    /// weight 1.5.
    NaConsist = 7,
}

impl TemplateCode {
    /// Templates available in the MVP (Sprint D1-D9).
    ///
    /// Preserved intact for backward compatibility ; a catalog
    /// configured with `template_set: "mvp"` references this set.
    pub const MVP_SET: &'static [Self] = &[Self::NaSem, Self::NaLog, Self::NaCrit];

    /// Sprint **D33** — full GA template set (8 entries).
    ///
    /// Production deployments post-D33 default to this set. The
    /// catalog field `template_set: "ga"` selects it explicitly.
    pub const GA_SET: &'static [Self] = &[
        Self::NaSem,
        Self::NaLog,
        Self::NaCrit,
        Self::NaQual,
        Self::NaCausal,
        Self::NaTemp,
        Self::NaSafety,
        Self::NaConsist,
    ];

    /// Stable string name used in configuration files and telemetry.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NaSem => "na-sem",
            Self::NaLog => "na-log",
            Self::NaCrit => "na-crit",
            Self::NaQual => "na-qual",
            Self::NaCausal => "na-causal",
            Self::NaTemp => "na-temp",
            Self::NaSafety => "na-safety",
            Self::NaConsist => "na-consist",
        }
    }

    /// Numeric value for CUDA buffers (stable across versions).
    #[must_use]
    pub const fn as_u8(&self) -> u8 {
        *self as u8
    }

    /// Parse from the stable string representation.
    #[must_use]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "na-sem" => Some(Self::NaSem),
            "na-log" => Some(Self::NaLog),
            "na-crit" => Some(Self::NaCrit),
            "na-qual" => Some(Self::NaQual),
            "na-causal" => Some(Self::NaCausal),
            "na-temp" => Some(Self::NaTemp),
            "na-safety" => Some(Self::NaSafety),
            "na-consist" => Some(Self::NaConsist),
            _ => None,
        }
    }

    /// Sprint **D33** — true if this template is part of the MVP
    /// (D1-D9) set, false if it's a D33 GA addition. Used by some
    /// dashboards to filter "MVP-era" metrics.
    #[must_use]
    pub const fn is_mvp(&self) -> bool {
        matches!(self, Self::NaSem | Self::NaLog | Self::NaCrit)
    }
}

// =====================================================================
// Latent dimension (validated)
// =====================================================================

/// Validated dimension of the latent space (1 <= d <= 65536).
///
/// # Deserialization
///
/// Deserializing a raw integer routes through [`Self::new`], so
/// out-of-range values produce an error instead of constructing an
/// invalid instance.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct LatentDim(NonZeroU32);

impl LatentDim {
    /// Maximum allowed dimension (hard cap for MVP safety).
    pub const MAX: u32 = 65_536;

    /// Construct a validated dimension.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] if `d == 0` or `d > MAX`.
    pub fn new(d: u32) -> Result<Self> {
        let nz = NonZeroU32::new(d)
            .ok_or_else(|| EfeError::InvalidConfig("dim_latent must be > 0".into()))?;
        if nz.get() > Self::MAX {
            return Err(EfeError::InvalidConfig(format!(
                "dim_latent must be <= {}, got {}",
                Self::MAX,
                d
            )));
        }
        Ok(Self(nz))
    }

    /// Get the raw value.
    #[must_use]
    pub fn get(&self) -> u32 {
        self.0.get()
    }
}

impl<'de> Deserialize<'de> for LatentDim {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u32::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

// =====================================================================
// Population mix (validated: non-empty, total > 0)
// =====================================================================

/// Distribution of nano-agents across templates.
///
/// Invariants (enforced by [`Self::new`] AND by [`Deserialize`]):
/// - non-empty entries
/// - total count > 0
///
/// Deserialization reads a sequence of `(TemplateCode, u32)` pairs and
/// reconstructs the `total` via [`Self::new`] to guarantee the invariant.
#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct PopulationMix {
    #[serde(serialize_with = "serialize_mix_entries")]
    entries: Vec<(TemplateCode, u32)>,
    #[serde(skip)]
    total: u32,
}

fn serialize_mix_entries<S>(
    entries: &[(TemplateCode, u32)],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    entries.serialize(serializer)
}

impl<'de> Deserialize<'de> for PopulationMix {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let entries: Vec<(TemplateCode, u32)> = Vec::deserialize(deserializer)?;
        Self::new(entries).map_err(serde::de::Error::custom)
    }
}

impl PopulationMix {
    /// Construct a validated mix.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] if empty or zero total.
    pub fn new(entries: Vec<(TemplateCode, u32)>) -> Result<Self> {
        if entries.is_empty() {
            return Err(EfeError::InvalidConfig(
                "population must not be empty".into(),
            ));
        }
        let total: u32 = entries.iter().map(|(_, c)| *c).sum();
        if total == 0 {
            return Err(EfeError::InvalidConfig(
                "population total must be > 0".into(),
            ));
        }
        Ok(Self { entries, total })
    }

    /// Total nano-agent count across all templates.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.total
    }

    /// Iterate over `(template, count)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = &(TemplateCode, u32)> {
        self.entries.iter()
    }

    /// Number of distinct templates in the mix.
    #[must_use]
    pub fn template_count(&self) -> usize {
        self.entries.len()
    }
}

// =====================================================================
// Resource budget
// =====================================================================

/// Hard resource budget enforced by the arbiter for each request.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ResourceBudget {
    /// Max wall-clock time, in milliseconds.
    pub max_wall_clock_ms: u32,
    /// Max energy consumption, in Watt-hours.
    pub max_energy_wh: f32,
    /// Max HBM bytes allocated for this request.
    pub max_hbm_bytes: u64,
    /// Max recursion depth for nested swarms (P5+).
    pub max_recursion_depth: u8,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            max_wall_clock_ms: 500,
            max_energy_wh: 5.0,
            max_hbm_bytes: 6 * 1024 * 1024 * 1024, // 6 GiB
            max_recursion_depth: 0,
        }
    }
}

// =====================================================================
// Convergence thresholds
// =====================================================================

/// Thresholds used by the consensus detector.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ConvergenceThresholds {
    /// Variance must fall below this to converge.
    pub epsilon_variance: f32,
    /// Gradient norm must fall below this to converge.
    pub epsilon_gradient: f32,
    /// Centroid movement must be below this for stability.
    pub epsilon_stability: f32,
    /// Number of stable rounds required before declaring convergence.
    pub stability_window: NonZeroU32,
}

impl Default for ConvergenceThresholds {
    fn default() -> Self {
        Self {
            epsilon_variance: 1e-3,
            epsilon_gradient: 1e-4,
            epsilon_stability: 1e-3,
            stability_window: NonZeroU32::new(3).expect("3 is non-zero"),
        }
    }
}

// =====================================================================
// Alignment cone
// =====================================================================

/// Alignment cone in the latent space — projection target for the centroid.
///
/// Deserialization routes through [`Self::new`] so that externally
/// supplied payloads cannot construct a cone with empty anchors or a
/// non-positive / non-finite distance threshold.
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct AlignmentCone {
    /// Anchor vectors (constitutional priors).
    pub anchors: Vec<Vec<f32>>,
    /// Maximum allowed projection distance before declaring a breach.
    pub max_projection_distance: f32,
}

impl AlignmentCone {
    /// Construct a cone, ensuring non-empty anchors and positive delta.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] when anchors are empty or
    /// `max_projection_distance` is not strictly positive.
    pub fn new(anchors: Vec<Vec<f32>>, max_projection_distance: f32) -> Result<Self> {
        if anchors.is_empty() {
            return Err(EfeError::InvalidConfig(
                "alignment cone needs anchors".into(),
            ));
        }
        if !(max_projection_distance > 0.0 && max_projection_distance.is_finite()) {
            return Err(EfeError::InvalidConfig(
                "max_projection_distance must be positive and finite".into(),
            ));
        }
        Ok(Self {
            anchors,
            max_projection_distance,
        })
    }
}

impl<'de> Deserialize<'de> for AlignmentCone {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            anchors: Vec<Vec<f32>>,
            max_projection_distance: f32,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.anchors, raw.max_projection_distance).map_err(serde::de::Error::custom)
    }
}

// =====================================================================
// Swarm spec
// =====================================================================

/// Complete specification of a swarm execution.
///
/// Deserialization additionally calls [`Self::validate`] so that foreign
/// payloads cannot produce a spec with an embedding length mismatching
/// `dim_latent`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "ValidatedSwarmSpec")]
#[non_exhaustive]
pub struct SwarmSpec {
    /// Identifier (generated by the controller, do not set manually).
    pub id: SwarmId,
    /// Problem embedding (length must match `dim_latent`).
    pub problem_embedding: Vec<f32>,
    /// Latent dimension.
    pub dim_latent: LatentDim,
    /// Population mix across templates.
    pub population: PopulationMix,
    /// Maximum number of fusion rounds.
    pub max_rounds: NonZeroU32,
    /// Convergence thresholds.
    pub convergence: ConvergenceThresholds,
    /// Alignment cone.
    pub alignment_cone: AlignmentCone,
    /// Resource budget.
    pub budget: ResourceBudget,
}

impl SwarmSpec {
    /// Start building a new spec.
    #[must_use]
    pub fn builder() -> SwarmSpecBuilder {
        SwarmSpecBuilder::default()
    }

    /// Re-validate structural invariants (useful when deserialized).
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] when any cross-field invariant
    /// is violated.
    pub fn validate(&self) -> Result<()> {
        let expected = self.dim_latent.get() as usize;
        if self.problem_embedding.len() != expected {
            return Err(EfeError::InvalidConfig(format!(
                "problem_embedding length {} does not match dim_latent {}",
                self.problem_embedding.len(),
                expected
            )));
        }
        if self.population.total() == 0 {
            return Err(EfeError::InvalidConfig("population total is zero".into()));
        }
        if !self.problem_embedding.iter().all(|x| x.is_finite()) {
            return Err(EfeError::InvalidConfig(
                "problem_embedding contains NaN or Inf".into(),
            ));
        }
        Ok(())
    }
}

/// Intermediate representation used to validate on [`Deserialize`].
#[derive(Deserialize)]
struct ValidatedSwarmSpec {
    id: SwarmId,
    problem_embedding: Vec<f32>,
    dim_latent: LatentDim,
    population: PopulationMix,
    max_rounds: NonZeroU32,
    #[serde(default)]
    convergence: ConvergenceThresholds,
    alignment_cone: AlignmentCone,
    #[serde(default)]
    budget: ResourceBudget,
}

impl TryFrom<ValidatedSwarmSpec> for SwarmSpec {
    type Error = EfeError;

    fn try_from(v: ValidatedSwarmSpec) -> Result<Self> {
        let spec = Self {
            id: v.id,
            problem_embedding: v.problem_embedding,
            dim_latent: v.dim_latent,
            population: v.population,
            max_rounds: v.max_rounds,
            convergence: v.convergence,
            alignment_cone: v.alignment_cone,
            budget: v.budget,
        };
        spec.validate()?;
        Ok(spec)
    }
}

/// Builder for [`SwarmSpec`] that accumulates optional fields then
/// validates them atomically at `build()`.
#[derive(Default)]
pub struct SwarmSpecBuilder {
    id: Option<SwarmId>,
    problem_embedding: Option<Vec<f32>>,
    dim_latent: Option<LatentDim>,
    population: Option<PopulationMix>,
    max_rounds: Option<NonZeroU32>,
    convergence: Option<ConvergenceThresholds>,
    alignment_cone: Option<AlignmentCone>,
    budget: Option<ResourceBudget>,
}

impl SwarmSpecBuilder {
    /// Override the swarm id (otherwise generated on build).
    #[must_use]
    pub fn id(mut self, id: SwarmId) -> Self {
        self.id = Some(id);
        self
    }

    /// Set the problem embedding.
    #[must_use]
    pub fn problem_embedding(mut self, emb: Vec<f32>) -> Self {
        self.problem_embedding = Some(emb);
        self
    }

    /// Set the latent dimension.
    #[must_use]
    pub fn dim_latent(mut self, d: LatentDim) -> Self {
        self.dim_latent = Some(d);
        self
    }

    /// Set the population mix.
    #[must_use]
    pub fn population(mut self, p: PopulationMix) -> Self {
        self.population = Some(p);
        self
    }

    /// Set `max_rounds`.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] if `r == 0`.
    pub fn max_rounds(mut self, r: u32) -> Result<Self> {
        self.max_rounds = Some(
            NonZeroU32::new(r)
                .ok_or_else(|| EfeError::InvalidConfig("max_rounds must be >= 1".into()))?,
        );
        Ok(self)
    }

    /// Set convergence thresholds.
    #[must_use]
    pub fn convergence(mut self, c: ConvergenceThresholds) -> Self {
        self.convergence = Some(c);
        self
    }

    /// Set the alignment cone.
    #[must_use]
    pub fn alignment_cone(mut self, cone: AlignmentCone) -> Self {
        self.alignment_cone = Some(cone);
        self
    }

    /// Set the resource budget.
    #[must_use]
    pub fn budget(mut self, b: ResourceBudget) -> Self {
        self.budget = Some(b);
        self
    }

    /// Build the spec, validating all invariants.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] when required fields are missing
    /// or cross-field constraints are violated.
    pub fn build(self) -> Result<SwarmSpec> {
        let problem_embedding = self
            .problem_embedding
            .ok_or_else(|| EfeError::InvalidConfig("problem_embedding required".into()))?;
        let dim_latent = self
            .dim_latent
            .ok_or_else(|| EfeError::InvalidConfig("dim_latent required".into()))?;
        if problem_embedding.len() != dim_latent.get() as usize {
            return Err(EfeError::InvalidConfig(format!(
                "problem_embedding length {} does not match dim_latent {}",
                problem_embedding.len(),
                dim_latent.get()
            )));
        }
        let population = self
            .population
            .ok_or_else(|| EfeError::InvalidConfig("population required".into()))?;
        let max_rounds = self
            .max_rounds
            .unwrap_or_else(|| NonZeroU32::new(8).expect("8 is non-zero"));
        let alignment_cone = self
            .alignment_cone
            .ok_or_else(|| EfeError::InvalidConfig("alignment_cone required".into()))?;

        Ok(SwarmSpec {
            id: self.id.unwrap_or_default(),
            problem_embedding,
            dim_latent,
            population,
            max_rounds,
            convergence: self.convergence.unwrap_or_default(),
            alignment_cone,
            budget: self.budget.unwrap_or_default(),
        })
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_cone() -> AlignmentCone {
        AlignmentCone::new(vec![vec![0.1; 4]], 0.15).expect("valid cone")
    }

    // --- Identifiers ---

    #[test]
    fn swarm_id_is_unique() {
        let a = SwarmId::new();
        let b = SwarmId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn request_id_roundtrip() {
        let r = RequestId::from_raw("req-42");
        assert_eq!(r.as_str(), "req-42");
        let json = serde_json::to_string(&r).expect("serialize");
        let back: RequestId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(r, back);
    }

    // --- Template code ---

    #[test]
    fn template_code_strings_stable() {
        assert_eq!(TemplateCode::NaSem.as_str(), "na-sem");
        assert_eq!(TemplateCode::NaLog.as_str(), "na-log");
        assert_eq!(TemplateCode::NaCrit.as_str(), "na-crit");
    }

    #[test]
    fn template_code_from_str_roundtrip() {
        for tc in TemplateCode::MVP_SET {
            let s = tc.as_str();
            assert_eq!(TemplateCode::from_str(s), Some(*tc));
        }
        assert_eq!(TemplateCode::from_str("unknown"), None);
    }

    #[test]
    fn template_code_u8_stable() {
        assert_eq!(TemplateCode::NaSem.as_u8(), 0);
        assert_eq!(TemplateCode::NaLog.as_u8(), 1);
        assert_eq!(TemplateCode::NaCrit.as_u8(), 2);
    }

    // --- LatentDim ---

    #[test]
    fn latent_dim_accepts_valid() {
        assert!(LatentDim::new(1).is_ok());
        assert!(LatentDim::new(2048).is_ok());
        assert!(LatentDim::new(LatentDim::MAX).is_ok());
    }

    #[test]
    fn latent_dim_rejects_zero() {
        assert!(LatentDim::new(0).is_err());
    }

    #[test]
    fn latent_dim_rejects_above_max() {
        assert!(LatentDim::new(LatentDim::MAX + 1).is_err());
    }

    // --- PopulationMix ---

    #[test]
    fn population_mix_rejects_empty() {
        assert!(PopulationMix::new(vec![]).is_err());
    }

    #[test]
    fn population_mix_rejects_zero_total() {
        assert!(PopulationMix::new(vec![(TemplateCode::NaSem, 0)]).is_err());
    }

    #[test]
    fn population_mix_accepts_valid() {
        let mix = PopulationMix::new(vec![(TemplateCode::NaSem, 100), (TemplateCode::NaLog, 50)])
            .expect("valid");
        assert_eq!(mix.total(), 150);
        assert_eq!(mix.template_count(), 2);
    }

    // --- Defaults ---

    #[test]
    fn resource_budget_default_sane() {
        let b = ResourceBudget::default();
        assert!(b.max_wall_clock_ms > 0);
        assert!(b.max_energy_wh > 0.0);
        assert!(b.max_hbm_bytes > 0);
    }

    #[test]
    fn convergence_thresholds_default_monotonic() {
        let c = ConvergenceThresholds::default();
        assert!(c.epsilon_variance > 0.0);
        assert!(c.epsilon_gradient > 0.0);
        assert!(c.stability_window.get() >= 1);
    }

    // --- AlignmentCone ---

    #[test]
    fn alignment_cone_rejects_empty_anchors() {
        assert!(AlignmentCone::new(vec![], 0.15).is_err());
    }

    #[test]
    fn alignment_cone_rejects_non_positive_delta() {
        assert!(AlignmentCone::new(vec![vec![0.1]], 0.0).is_err());
        assert!(AlignmentCone::new(vec![vec![0.1]], -0.1).is_err());
        assert!(AlignmentCone::new(vec![vec![0.1]], f32::NAN).is_err());
    }

    // --- SwarmSpec builder ---

    #[test]
    fn spec_builder_full_ok() {
        let spec = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 4])
            .dim_latent(LatentDim::new(4).expect("valid"))
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 10)]).expect("valid"))
            .max_rounds(5)
            .expect("valid rounds")
            .alignment_cone(sample_cone())
            .build()
            .expect("valid spec");
        spec.validate().expect("spec is self-consistent");
        assert_eq!(spec.max_rounds.get(), 5);
        assert_eq!(spec.population.total(), 10);
    }

    #[test]
    fn spec_builder_rejects_mismatched_embedding() {
        let err = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 3])
            .dim_latent(LatentDim::new(4).expect("valid"))
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 10)]).expect("valid"))
            .alignment_cone(sample_cone())
            .build();
        assert!(err.is_err());
    }

    #[test]
    fn spec_builder_rejects_missing_population() {
        let err = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 4])
            .dim_latent(LatentDim::new(4).expect("valid"))
            .alignment_cone(sample_cone())
            .build();
        assert!(err.is_err());
    }

    #[test]
    fn spec_builder_rejects_zero_rounds() {
        assert!(SwarmSpec::builder().max_rounds(0).is_err());
    }

    // --- Property-based ---

    proptest! {
        #[test]
        fn population_mix_total_matches_sum(
            counts in prop::collection::vec(1u32..1000, 1..10)
        ) {
            let entries: Vec<_> = counts
                .iter()
                .map(|&c| (TemplateCode::NaSem, c))
                .collect();
            let expected: u32 = counts.iter().sum();
            let mix = PopulationMix::new(entries).expect("valid");
            prop_assert_eq!(mix.total(), expected);
        }

        #[test]
        fn latent_dim_roundtrip(d in 1u32..=LatentDim::MAX) {
            let ld = LatentDim::new(d).expect("valid");
            prop_assert_eq!(ld.get(), d);
        }

        #[test]
        fn template_code_u8_distinct(
            codes in prop::collection::vec(
                prop::sample::select(TemplateCode::MVP_SET.to_vec()),
                TemplateCode::MVP_SET.len(),
            )
        ) {
            let values: std::collections::HashSet<u8> =
                TemplateCode::MVP_SET.iter().map(TemplateCode::as_u8).collect();
            prop_assert_eq!(values.len(), TemplateCode::MVP_SET.len());
            let _ = codes;
        }
    }
}

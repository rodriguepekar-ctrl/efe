//! YAML-deserialisable configuration types.
//!
//! Used by `kovant_procurement.meta-swarm.yaml`-style declarative
//! definitions of a meta-swarm.  In Phase 1 we only parse the schema ;
//! a full programmatic builder is provided in [`crate::builder`].

use serde::{Deserialize, Serialize};

/// Top-level YAML schema for a meta-swarm definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaSwarmConfig {
    /// Unique identifier.
    pub id: String,

    /// Optional version stamp.
    #[serde(default)]
    pub version: Option<String>,

    /// Meta-space description.
    pub meta_space: MetaSpaceConfig,

    /// Coupling hyperparameters.
    #[serde(default)]
    pub coupling: CouplingConfig,

    /// Each participant (lift, pull, safety, bundle reference, alpha).
    pub participants: Vec<ParticipantConfig>,

    /// Choice of meta-operator.
    #[serde(default)]
    pub meta_operator: MetaOperatorConfig,

    /// Forward-compat — Phase-3 tenant binding.  Always None in Phase 1.
    #[serde(default)]
    pub tenant_id: Option<String>,
}

/// Meta-space description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaSpaceConfig {
    /// Dimension of the meta-space.
    pub dim: usize,
}

/// Coupling hyperparameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouplingConfig {
    /// Coupling coefficient $\gamma \in [0, 1)$.
    pub gamma: f64,
    /// Default $\alpha$ per participant ; can be overridden in
    /// `ParticipantConfig`.
    pub alpha_default: f64,
    /// Max iterations.
    pub max_iters: usize,
    /// Convergence tolerance.
    pub tolerance: f64,
}

impl Default for CouplingConfig {
    fn default() -> Self {
        Self {
            gamma: 0.2,
            alpha_default: 0.5,
            max_iters: 1_000,
            tolerance: 1e-4,
        }
    }
}

/// Per-participant configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantConfig {
    /// Unique id within the meta-swarm.
    pub id: String,
    /// Participant's latent dimension.
    pub dim: usize,
    /// Optional override of the default `alpha`.
    #[serde(default)]
    pub alpha: Option<f64>,
    /// Reference to the participant's bundle (Phase 3+ semantics).
    #[serde(default)]
    pub bundle: Option<String>,
    /// (Phase 2) gRPC endpoint of the participant.  Empty in Phase 1.
    #[serde(default)]
    pub endpoint: Option<String>,
}

/// Meta-operator choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetaOperatorConfig {
    /// Uniform 1/M weighting.
    UniformWeighted,
    /// Attention-weighted (requires `attention-weighted` feature).
    AttentionSoftmax {
        /// Temperature.
        temperature: f64,
    },
}

impl Default for MetaOperatorConfig {
    fn default() -> Self { Self::UniformWeighted }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialise_minimal_yaml() {
        let yaml = r#"
id: "kovant-procurement-v1"
meta_space:
  dim: 256
coupling:
  gamma: 0.2
  alpha_default: 0.5
  max_iters: 1000
  tolerance: 0.0001
participants:
  - id: "supplier-scoring"
    dim: 384
  - id: "contract-risk"
    dim: 512
  - id: "spend-categorization"
    dim: 384
meta_operator:
  kind: uniform_weighted
"#;
        let cfg: MetaSwarmConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.id, "kovant-procurement-v1");
        assert_eq!(cfg.meta_space.dim, 256);
        assert_eq!(cfg.participants.len(), 3);
        assert!(matches!(cfg.meta_operator, MetaOperatorConfig::UniformWeighted));
        assert!(cfg.tenant_id.is_none());
    }

    #[test]
    fn deserialise_attention_variant() {
        let yaml = r#"
id: "test"
meta_space:
  dim: 64
participants:
  - id: "a"
    dim: 64
  - id: "b"
    dim: 64
meta_operator:
  kind: attention_softmax
  temperature: 100.0
"#;
        let cfg: MetaSwarmConfig = serde_yaml::from_str(yaml).unwrap();
        if let MetaOperatorConfig::AttentionSoftmax { temperature } = cfg.meta_operator {
            assert!((temperature - 100.0).abs() < 1e-9);
        } else {
            panic!("expected attention_softmax");
        }
    }

    // =========================================================================
    // ===  Sprint A1 P1 — PR #3 — Category 13 : Config YAML proptest (5) ====
    // =========================================================================

    use proptest::prelude::*;

    fn pt_cfg_yaml() -> ProptestConfig {
        ProptestConfig { cases: 32, ..ProptestConfig::default() }
    }

    proptest! {
        #![proptest_config(pt_cfg_yaml())]

        /// Cat 13 / Test 1 — A minimal-valid YAML parses without error.
        #[test]
        fn config_minimal_yaml_parses_pt(
            dim in 1usize..=2048,
            n_participants in 1usize..=10,
        ) {
            use std::fmt::Write as _;
            let mut participants = String::new();
            for i in 0..n_participants {
                writeln!(participants, "  - id: \"p{i}\"\n    dim: 64").unwrap();
            }
            let yaml = format!(r#"
id: "pt13-1"
meta_space:
  dim: {dim}
participants:
{participants}"#);
            let cfg: MetaSwarmConfig = serde_yaml::from_str(&yaml).unwrap();
            prop_assert_eq!(cfg.meta_space.dim, dim);
            prop_assert_eq!(cfg.participants.len(), n_participants);
        }

        /// Cat 13 / Test 2 — Coupling defaults apply when section omitted.
        #[test]
        fn config_coupling_defaults_pt(dim in 8usize..=256) {
            let yaml = format!(r#"
id: "pt13-2"
meta_space:
  dim: {dim}
participants:
  - id: "p1"
    dim: 64
"#);
            let cfg: MetaSwarmConfig = serde_yaml::from_str(&yaml).unwrap();
            prop_assert!(cfg.coupling.max_iters > 0);
            prop_assert!(cfg.coupling.gamma >= 0.0);
            prop_assert!(cfg.coupling.alpha_default >= 0.0);
        }

        /// Cat 13 / Test 3 — Tenant ID parses as Some when present, None
        /// when absent.
        #[test]
        fn config_tenant_id_optional_pt(tenant in "[a-z][a-z0-9-]{2,16}") {
            let yaml_with = format!(r#"
id: "pt13-3"
meta_space:
  dim: 64
participants:
  - id: "p"
    dim: 64
tenant_id: "{tenant}"
"#);
            let cfg: MetaSwarmConfig = serde_yaml::from_str(&yaml_with).unwrap();
            prop_assert_eq!(cfg.tenant_id, Some(tenant));

            let yaml_without = r#"
id: "pt13-3-no"
meta_space:
  dim: 64
participants:
  - id: "p"
    dim: 64
"#;
            let cfg: MetaSwarmConfig = serde_yaml::from_str(yaml_without).unwrap();
            prop_assert_eq!(cfg.tenant_id, None);
        }

        /// Cat 13 / Test 4 — Attention variant parses with positive
        /// temperature.
        #[test]
        fn config_attention_variant_pt(temperature in 0.01_f64..1000.0) {
            let yaml = format!(r#"
id: "pt13-4"
meta_space:
  dim: 64
participants:
  - id: "p"
    dim: 64
meta_operator:
  kind: attention_softmax
  temperature: {temperature}
"#);
            let cfg: MetaSwarmConfig = serde_yaml::from_str(&yaml).unwrap();
            if let MetaOperatorConfig::AttentionSoftmax { temperature: t } = cfg.meta_operator {
                prop_assert!((t - temperature).abs() < 1e-9);
            } else {
                panic!("expected AttentionSoftmax variant");
            }
        }

        /// Cat 13 / Test 5 — Participant alpha override is recovered.
        #[test]
        fn config_participant_alpha_override_pt(alpha in 0.01_f64..0.9) {
            let yaml = format!(r#"
id: "pt13-5"
meta_space:
  dim: 64
participants:
  - id: "p"
    dim: 64
    alpha: {alpha}
"#);
            let cfg: MetaSwarmConfig = serde_yaml::from_str(&yaml).unwrap();
            prop_assert!(cfg.participants[0].alpha.is_some());
            prop_assert!((cfg.participants[0].alpha.unwrap() - alpha).abs() < 1e-9);
        }
    }
}

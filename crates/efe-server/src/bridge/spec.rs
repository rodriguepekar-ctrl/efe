//! `SwarmSpec` ↔ proto conversion.

use std::num::NonZeroU32;

use crate::bridge::{budget, cone};
use crate::proto::v1 as proto;
use efe_core::{
    ConvergenceThresholds, EfeError, LatentDim, PopulationMix, Result, SwarmSpec, TemplateCode,
};

/// Convert a proto `SwarmSpec` into its domain form.
///
/// The `cone_source` `oneof` supports:
/// - [`proto::swarm_spec::ConeSource::AlignmentCone`] — inline cone
///   materialised into the spec directly.
/// - [`proto::swarm_spec::ConeSource::AlignmentConeRef`] — registry
///   reference. **Unsupported in the unary path** at MVP; the client
///   is expected to resolve via `SwarmSpecBuilder::from_registry`
///   before calling Execute.
pub fn from_proto(p: proto::SwarmSpec) -> Result<SwarmSpec> {
    let mut builder = SwarmSpec::builder()
        .problem_embedding(p.problem_embedding)
        .dim_latent(LatentDim::new(p.dim_latent)?);

    // Population.
    let entries: Vec<(TemplateCode, u32)> = p
        .population
        .into_iter()
        .map(|e| {
            let code = TemplateCode::from_str(&e.template_code).ok_or_else(|| {
                EfeError::InvalidConfig(format!("unknown template_code {:?}", e.template_code))
            })?;
            Ok((code, e.count))
        })
        .collect::<Result<Vec<_>>>()?;
    builder = builder.population(PopulationMix::new(entries)?);

    // Rounds.
    if p.max_rounds > 0 {
        builder = builder.max_rounds(p.max_rounds)?;
    }

    // Convergence thresholds — struct is `#[non_exhaustive]` across
    // crates, so we derive from `Default` and mutate in place.
    if let Some(c) = p.convergence {
        let stability_window =
            NonZeroU32::new(c.stability_window.max(1)).expect("max(1, _) is non-zero");
        let mut thresholds = ConvergenceThresholds::default();
        thresholds.epsilon_variance = c.epsilon_variance;
        thresholds.epsilon_gradient = c.epsilon_gradient;
        thresholds.epsilon_stability = c.epsilon_stability;
        thresholds.stability_window = stability_window;
        builder = builder.convergence(thresholds);
    }

    // Cone.
    let cone_value = match p.cone_source {
        Some(proto::swarm_spec::ConeSource::AlignmentCone(c)) => cone::from_proto(c)?,
        Some(proto::swarm_spec::ConeSource::AlignmentConeRef(_)) => {
            return Err(EfeError::InvalidConfig(
                "server-side cone resolution from registry is not yet supported in Execute; \
                 resolve the cone client-side and submit it inline"
                    .into(),
            ));
        }
        None => {
            return Err(EfeError::InvalidConfig(
                "SwarmSpec.cone_source is required".into(),
            ));
        }
    };
    builder = builder.alignment_cone(cone_value);

    if let Some(b) = p.budget {
        builder = builder.budget(budget::from_proto(b)?);
    }

    builder.build()
}

/// Convert a domain [`SwarmSpec`] into its proto form.
///
/// Used by `DescribeSpec` echoes and tests. The `swarm_id` is embedded
/// as a stringified UUID.
#[must_use]
pub fn to_proto(s: &SwarmSpec) -> proto::SwarmSpec {
    let population: Vec<proto::PopulationEntry> = s
        .population
        .iter()
        .map(|(tpl, count)| proto::PopulationEntry {
            template_code: tpl.as_str().to_string(),
            count: *count,
        })
        .collect();
    // `proto::SwarmSpec` comes from `tonic-build` (not marked
    // non_exhaustive) so struct-expression construction is allowed.
    proto::SwarmSpec {
        swarm_id: s.id.to_string(),
        problem_embedding: s.problem_embedding.clone(),
        dim_latent: s.dim_latent.get(),
        population,
        max_rounds: s.max_rounds.get(),
        convergence: Some(proto::ConvergenceThresholds {
            epsilon_variance: s.convergence.epsilon_variance,
            epsilon_gradient: s.convergence.epsilon_gradient,
            epsilon_stability: s.convergence.epsilon_stability,
            stability_window: s.convergence.stability_window.get(),
        }),
        cone_source: Some(proto::swarm_spec::ConeSource::AlignmentCone(
            cone::to_proto(&s.alignment_cone),
        )),
        budget: Some(budget::to_proto(s.budget)),
    }
}

/// Estimate HBM usage for `DescribeSpec`.
#[must_use]
pub fn estimate_hbm_bytes(s: &SwarmSpec) -> u64 {
    // Rough estimate: per-agent latent (dim bf16) + mask byte + working buffers.
    let n_agents = u64::from(s.population.total());
    let dim = u64::from(s.dim_latent.get());
    // bf16 = 2 bytes ; account for primary + scratch ; +10% overhead.
    let latent = 2 * 2 * n_agents * dim;
    let mask = n_agents;
    (latent + mask) * 11 / 10
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proto_spec() -> proto::SwarmSpec {
        proto::SwarmSpec {
            swarm_id: String::new(),
            problem_embedding: vec![0.0; 4],
            dim_latent: 4,
            population: vec![proto::PopulationEntry {
                template_code: "na-sem".into(),
                count: 4,
            }],
            max_rounds: 4,
            convergence: None,
            cone_source: Some(proto::swarm_spec::ConeSource::AlignmentCone(
                proto::AlignmentCone {
                    anchors: vec![proto::Anchor {
                        values: vec![0.1, 0.1, 0.1, 0.1],
                    }],
                    max_projection_distance: 0.5,
                },
            )),
            budget: None,
        }
    }

    #[test]
    fn spec_roundtrip_minimal() {
        let domain = from_proto(proto_spec()).unwrap();
        assert_eq!(domain.dim_latent.get(), 4);
        assert_eq!(domain.population.total(), 4);
    }

    #[test]
    fn spec_rejects_unknown_template() {
        let mut p = proto_spec();
        p.population[0].template_code = "no-such".into();
        assert!(from_proto(p).is_err());
    }

    #[test]
    fn spec_rejects_zero_dim() {
        let mut p = proto_spec();
        p.dim_latent = 0;
        assert!(from_proto(p).is_err());
    }

    #[test]
    fn spec_rejects_cone_ref_in_unary_path() {
        let mut p = proto_spec();
        p.cone_source = Some(proto::swarm_spec::ConeSource::AlignmentConeRef(
            proto::ConeRef {
                tenant: "t".into(),
                task: "k".into(),
                version: 1,
            },
        ));
        let err = from_proto(p).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn spec_rejects_missing_cone_source() {
        let mut p = proto_spec();
        p.cone_source = None;
        assert!(from_proto(p).is_err());
    }

    #[test]
    fn spec_uses_default_convergence_when_unset() {
        let domain = from_proto(proto_spec()).unwrap();
        assert!(domain.convergence.epsilon_variance > 0.0);
    }

    #[test]
    fn spec_respects_custom_convergence() {
        let mut p = proto_spec();
        p.convergence = Some(proto::ConvergenceThresholds {
            epsilon_variance: 0.5,
            epsilon_gradient: 0.1,
            epsilon_stability: 0.2,
            stability_window: 5,
        });
        let domain = from_proto(p).unwrap();
        assert!((domain.convergence.epsilon_variance - 0.5).abs() < 1e-6);
        assert_eq!(domain.convergence.stability_window.get(), 5);
    }

    #[test]
    fn estimate_hbm_is_positive() {
        let domain = from_proto(proto_spec()).unwrap();
        assert!(estimate_hbm_bytes(&domain) > 0);
    }

    #[test]
    fn spec_to_proto_roundtrip() {
        let domain = from_proto(proto_spec()).unwrap();
        let back = to_proto(&domain);
        let domain2 = from_proto(back).unwrap();
        assert_eq!(domain.dim_latent.get(), domain2.dim_latent.get());
        assert_eq!(domain.population.total(), domain2.population.total());
    }
}

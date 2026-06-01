//! Security / soundness regression tests.
//!
//! These tests encode the audit findings fixed in the first round of
//! review. Any regression on one of these invariants must block the PR.

mod mocks;

use std::sync::Arc;

use efe_core::fusion::FusionStats;
use efe_core::swarm::{DecodedOutput, Request, SwarmController, Verdict};
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};
use efe_core::EfeError;

use mocks::{MockAligner, MockDissolver, MockFusion, MockGenesis, ScriptedDetector};

// =====================================================================
// F1 - Alignment delta NaN / Inf must be treated as a breach
// =====================================================================

#[tokio::test]
async fn alignment_nan_delta_is_treated_as_breach() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 4));
    let fusion = Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(
        1,
        dim as usize,
    )]));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "irrelevant"));
    // Aligner that maliciously returns NaN for delta.
    let aligner = Arc::new(MockAligner::new(f32::NAN, 1.0));

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.9,
            }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(minimal_spec(dim)))
        .await
        .unwrap_err();
    assert!(
        matches!(err, EfeError::AlignmentBreach { delta, .. } if delta.is_nan()),
        "NaN delta must be converted to AlignmentBreach, got {err:?}"
    );
}

#[tokio::test]
async fn alignment_inf_delta_is_treated_as_breach() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 4));
    let fusion = Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(
        1,
        dim as usize,
    )]));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "irrelevant"));
    let aligner = Arc::new(MockAligner::new(f32::INFINITY, 1.0));

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.9,
            }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(minimal_spec(dim)))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::AlignmentBreach { .. }));
}

// =====================================================================
// F2 - Deserialization MUST enforce invariants
// =====================================================================

#[test]
fn deserialize_latent_dim_rejects_zero() {
    let json = "0";
    let res: std::result::Result<LatentDim, _> = serde_json::from_str(json);
    assert!(res.is_err(), "LatentDim::0 must not deserialize");
}

#[test]
fn deserialize_latent_dim_rejects_above_max() {
    let json = format!("{}", LatentDim::MAX + 1);
    let res: std::result::Result<LatentDim, _> = serde_json::from_str(&json);
    assert!(
        res.is_err(),
        "LatentDim > MAX must not deserialize, got {res:?}"
    );
}

#[test]
fn deserialize_latent_dim_accepts_valid() {
    let json = "2048";
    let ld: LatentDim = serde_json::from_str(json).expect("valid");
    assert_eq!(ld.get(), 2048);
}

#[test]
fn deserialize_population_mix_rejects_empty_list() {
    let json = "[]";
    let res: std::result::Result<PopulationMix, _> = serde_json::from_str(json);
    assert!(res.is_err(), "empty population must not deserialize");
}

#[test]
fn deserialize_population_mix_rejects_zero_total() {
    // All templates with count 0.
    let json = r#"[["NaSem", 0], ["NaLog", 0]]"#;
    let res: std::result::Result<PopulationMix, _> = serde_json::from_str(json);
    assert!(res.is_err(), "zero total must not deserialize");
}

#[test]
fn deserialize_population_mix_accepts_valid() {
    let json = r#"[["NaSem", 10], ["NaLog", 5]]"#;
    let mix: PopulationMix = serde_json::from_str(json).expect("valid");
    assert_eq!(mix.total(), 15);
}

#[test]
fn deserialize_alignment_cone_rejects_empty_anchors() {
    let json = r#"{"anchors":[],"max_projection_distance":0.15}"#;
    let res: std::result::Result<AlignmentCone, _> = serde_json::from_str(json);
    assert!(res.is_err(), "empty anchors must not deserialize");
}

#[test]
fn deserialize_alignment_cone_rejects_non_positive_delta() {
    for delta in ["0.0", "-0.1", "NaN"] {
        let json = format!(r#"{{"anchors":[[0.1]],"max_projection_distance":{delta}}}"#);
        let res: std::result::Result<AlignmentCone, _> = serde_json::from_str(&json);
        assert!(res.is_err(), "delta={delta} must not deserialize");
    }
}

#[test]
fn deserialize_swarm_spec_rejects_mismatched_embedding() {
    // dim_latent = 4 but problem_embedding is length 2 → must reject.
    let json = serde_json::json!({
        "id": "00000000-0000-0000-0000-000000000000",
        "problem_embedding": [0.0, 1.0],
        "dim_latent": 4,
        "population": [["NaSem", 10]],
        "max_rounds": 5,
        "convergence": {
            "epsilon_variance": 0.001,
            "epsilon_gradient": 0.0001,
            "epsilon_stability": 0.001,
            "stability_window": 3
        },
        "alignment_cone": {
            "anchors": [[0.1, 0.2, 0.3, 0.4]],
            "max_projection_distance": 0.15
        },
        "budget": {
            "max_wall_clock_ms": 500,
            "max_energy_wh": 5.0,
            "max_hbm_bytes": 1000000,
            "max_recursion_depth": 0
        }
    });
    let res: std::result::Result<SwarmSpec, _> = serde_json::from_value(json);
    assert!(res.is_err(), "mismatched embedding must not deserialize");
}

#[test]
fn deserialize_swarm_spec_rejects_nan_embedding() {
    let json = serde_json::json!({
        "id": "00000000-0000-0000-0000-000000000000",
        "problem_embedding": [0.0, f32::NAN, 1.0, 2.0],
        "dim_latent": 4,
        "population": [["NaSem", 10]],
        "max_rounds": 5,
        "alignment_cone": {
            "anchors": [[0.1, 0.2, 0.3, 0.4]],
            "max_projection_distance": 0.15
        }
    });
    let res: std::result::Result<SwarmSpec, _> = serde_json::from_value(json);
    assert!(res.is_err(), "NaN in embedding must not deserialize");
}

#[test]
fn population_mix_roundtrip_preserves_total() {
    let mix = PopulationMix::new(vec![(TemplateCode::NaSem, 100), (TemplateCode::NaLog, 50)])
        .expect("valid");
    let json = serde_json::to_string(&mix).expect("serialize");
    let back: PopulationMix = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.total(), 150);
}

// =====================================================================
// F3 - ConvergenceTimeout must carry the actual last variance
// =====================================================================

#[tokio::test]
async fn convergence_timeout_carries_last_variance() {
    let dim = 4;
    // Create 3 stats with decreasing but never-converging variance.
    let stats = vec![
        FusionStats::synthetic_with(1, dim as usize, 0.9, 0.1),
        FusionStats::synthetic_with(2, dim as usize, 0.6, 0.1),
        FusionStats::synthetic_with(3, dim as usize, 0.42, 0.1),
    ];
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("valid"), 4));
    let fusion = Arc::new(MockFusion::new(stats));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, ""));
    let aligner = Arc::new(MockAligner::safe());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .consensus_factory(|| Box::new(ScriptedDetector::new(vec![Verdict::Continue; 10])))
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(minimal_spec(dim)))
        .await
        .unwrap_err();
    match err {
        EfeError::ConvergenceTimeout { rounds, sigma2 } => {
            assert_eq!(rounds, 3);
            assert!(
                (sigma2 - 0.42).abs() < 1e-6,
                "expected last variance 0.42, got {sigma2}"
            );
        }
        other => panic!("expected ConvergenceTimeout, got {other:?}"),
    }
}

// =====================================================================
// Helper
// =====================================================================

fn minimal_spec(dim: u32) -> SwarmSpec {
    SwarmSpec::builder()
        .problem_embedding(vec![0.0; dim as usize])
        .dim_latent(LatentDim::new(dim).expect("valid dim"))
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).expect("valid mix"))
        .max_rounds(3)
        .expect("valid rounds")
        .alignment_cone(
            AlignmentCone::new(vec![vec![0.1; dim as usize]], 0.15).expect("valid cone"),
        )
        .build()
        .expect("valid spec")
}

// =====================================================================
// Additional soundness checks
// =====================================================================

#[test]
fn decoded_output_equality_text_vs_raw() {
    let a = DecodedOutput::Text("x".into());
    let b = DecodedOutput::Raw(vec![0.0]);
    assert_ne!(a, b);
}

#[test]
fn latent_dim_bounds_are_exact() {
    assert!(LatentDim::new(1).is_ok());
    assert!(LatentDim::new(LatentDim::MAX).is_ok());
    assert!(LatentDim::new(0).is_err());
    assert!(LatentDim::new(LatentDim::MAX + 1).is_err());
}

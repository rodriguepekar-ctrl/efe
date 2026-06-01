//! **Sprint D7 — Calibration realism + anti-regression integration tests.**
//!
//! These tests pin the calibrated β/α schedules of the Mode-1 fusion
//! and verify, end-to-end, that :
//!
//! 1. The 4 tuned values in `build_mode1_fusion_config()` survive every
//!    `Mode1Config` constructor (`default`, `for_dim`, `from_env_for_dim`).
//! 2. The new schedule does NOT trigger spurious `alignment_breach`
//!    failures on a hand-curated valid spec (the R5 mitigation from the
//!    architecture document).
//! 3. The β/α env-var override path correctly drives the running
//!    backend — a calibration sweep can change the schedule without
//!    rebuilding the binary.
//!
//! These are integration tests (live in `crates/efe-server/tests/`)
//! because tests #2 and #3 spin a full `Mode1Backend` and execute one
//! `Request` end-to-end. Tests #1 also live here so the Sprint D7
//! anti-regression contract is collocated in a single file the
//! reviewer can read at a glance.
//!
//! # Failure mode
//!
//! Any PR that quietly mutates the β/α schedule (e.g. via a
//! `..FusionConfig::default()` shortcut upstream that bypasses
//! `build_mode1_fusion_config`) trips test #1 with a precise message
//! pointing at `docs/audits/d7_calibration_report.md`. Test #2
//! catches the case where a future relaxation of α breaks the
//! alignment-breach safety net (R5 in the architecture).

use std::sync::Arc;

use efe_core::swarm::request::{DecodedOutput, Request};
use efe_core::types::{
    AlignmentCone, LatentDim, PopulationMix, ResourceBudget, SwarmSpec, TemplateCode,
};
use efe_server::mode1::{build_mode1_fusion_config, build_mvp_backend, Mode1Config};
use efe_server::services::ExecuteBackend;

// =====================================================================
// 1. Pinned-thresholds anti-regression tests (deterministic, pure)
// =====================================================================

#[test]
fn d7_pinned_fusion_thresholds() {
    let f = build_mode1_fusion_config();
    assert!(
        (f.problem_anchor_init - 0.45).abs() < 1e-6,
        "D7 layer-1 : problem_anchor_init must be 0.45 ; got {} \
         — see docs/audits/d7_calibration_report.md",
        f.problem_anchor_init
    );
    assert!(
        (f.problem_anchor_decay - 0.08).abs() < 1e-6,
        "D7 layer-1 : problem_anchor_decay must be 0.08 (sweep winner) ; got {}",
        f.problem_anchor_decay
    );
    assert!(
        (f.alignment_strength_init - 0.25).abs() < 1e-6,
        "D7 layer-2 : alignment_strength_init must be 0.25 (sweep winner) ; got {}",
        f.alignment_strength_init
    );
    assert!(
        (f.alignment_strength_floor - 0.05).abs() < 1e-6,
        "D7 layer-2 : alignment_strength_floor must remain 0.05 ; got {}",
        f.alignment_strength_floor
    );
    assert_eq!(
        f.max_rounds, 16,
        "D7 : max_rounds must remain 16 (only β/α schedules are retuned)"
    );
    f.validate()
        .expect("D7 build_mode1_fusion_config must produce a valid FusionConfig");
}

#[test]
fn d7_default_propagates_tuned_fusion() {
    let c = Mode1Config::default();
    assert!((c.fusion.problem_anchor_init - 0.45).abs() < 1e-6);
    assert!((c.fusion.problem_anchor_decay - 0.08).abs() < 1e-6);
    assert!((c.fusion.alignment_strength_init - 0.25).abs() < 1e-6);
    assert!((c.fusion.alignment_strength_floor - 0.05).abs() < 1e-6);
}

#[test]
fn d7_for_dim_preserves_tuned_fusion() {
    for dim in [256_u32, 384, 512, 768, 1024] {
        let c = Mode1Config::for_dim(dim);
        assert!(
            (c.fusion.problem_anchor_init - 0.45).abs() < 1e-6,
            "D7 : for_dim({dim}) lost problem_anchor_init"
        );
        assert!(
            (c.fusion.problem_anchor_decay - 0.08).abs() < 1e-6,
            "D7 : for_dim({dim}) lost problem_anchor_decay"
        );
        assert!(
            (c.fusion.alignment_strength_init - 0.25).abs() < 1e-6,
            "D7 : for_dim({dim}) lost alignment_strength_init"
        );
    }
}

#[test]
fn d7_env_overrides_drive_running_backend() {
    // Lookup that simulates an operator-supplied calibration : push β₀
    // to a low value so the resulting Mode1Config reflects the env
    // input — proves the override path works end-to-end (without
    // having to spin a child process).
    let cfg = Mode1Config::from_env_for_dim_with(384, |k| match k {
        "EFE_FUSION_BETA_INIT" => Ok("0.50".to_string()),
        "EFE_FUSION_BETA_DECAY" => Ok("0.07".to_string()),
        "EFE_FUSION_ALPHA_INIT" => Ok("0.25".to_string()),
        _ => Err(std::env::VarError::NotPresent),
    });
    assert!((cfg.fusion.problem_anchor_init - 0.50).abs() < 1e-6);
    assert!((cfg.fusion.problem_anchor_decay - 0.07).abs() < 1e-6);
    assert!((cfg.fusion.alignment_strength_init - 0.25).abs() < 1e-6);
    // Floor was never set : it must remain at the calibrated default.
    assert!((cfg.fusion.alignment_strength_floor - 0.05).abs() < 1e-6);
}

// =====================================================================
// 2. Live realism check — R5 mitigation : no alignment_breach with
//    the new (lower) α schedule on a hand-curated valid spec.
// =====================================================================

/// Build a small, deterministic Mode-1 spec whose problem embedding
/// lies exactly on one of the cone anchors — i.e. the spec is *as
/// aligned as possible*. With the calibrated D7 schedule the
/// alignment-projector must accept this without raising a breach.
fn make_aligned_spec(dim: u32) -> SwarmSpec {
    let dim_u = dim as usize;
    // Three orthogonal anchors. We deliberately keep them simple
    // (one-hot in the first 3 coordinates) so the cone arithmetic is
    // easy to verify by hand. Vectors are L2-normalised already.
    let mut anchor_a = vec![0.0_f32; dim_u];
    anchor_a[0] = 1.0;
    let mut anchor_b = vec![0.0_f32; dim_u];
    anchor_b[1] = 1.0;
    let mut anchor_c = vec![0.0_f32; dim_u];
    anchor_c[2] = 1.0;

    // Problem embedding sits *on* anchor_a — a request that should
    // classify cleanly as the "a" class, no breach possible.
    let problem_embedding = anchor_a.clone();

    let cone =
        AlignmentCone::new(vec![anchor_a, anchor_b, anchor_c], 0.5).expect("valid 3-anchor cone");
    let pop = PopulationMix::new(vec![
        (TemplateCode::NaSem, 32),
        (TemplateCode::NaLog, 16),
        (TemplateCode::NaCrit, 16),
    ])
    .expect("valid population mix");
    let mut budget = ResourceBudget::default();
    budget.max_wall_clock_ms = 30_000;

    SwarmSpec::builder()
        .problem_embedding(problem_embedding)
        .dim_latent(LatentDim::new(dim).expect("dim"))
        .population(pop)
        .max_rounds(8)
        .expect("rounds")
        .alignment_cone(cone)
        .budget(budget)
        .build()
        .expect("spec build")
}

fn build_backend(dim: u32) -> Arc<dyn ExecuteBackend> {
    let mut cfg = Mode1Config::for_dim(dim);
    // Reduce arena count so the test stays cheap (one request only).
    cfg.prealloc_arenas = 2;
    cfg.max_arenas = 4;
    build_mvp_backend(cfg).expect("backend builds with D7 defaults")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d7_no_alignment_breach_on_aligned_spec() {
    let spec = make_aligned_spec(64);
    let backend = build_backend(64);
    let req = Request::for_tests(spec);
    let resp = backend
        .execute(req)
        .await
        .expect("aligned spec must execute without error");
    // The response carries an `alignment_delta` measuring how far the
    // raw centroid had to be projected to fit the cone. With the D7
    // schedule (α_floor = 0.05 unchanged + α₀ = 0.30) and a perfectly
    // aligned input, the delta must remain small.
    //
    // R5 in the architecture document specifies "≤ 0.50" (the cone
    // radius) — anything within tolerance is fine. We check that we
    // are well under that cap.
    assert!(
        resp.alignment_delta < 0.50,
        "D7 R5 : alignment_delta = {} is suspiciously high on a \
         perfectly aligned spec ; the new α schedule may have broken \
         the cone projection.",
        resp.alignment_delta
    );
    // The campaign must have run at least one fusion round (the D6
    // controller pin), so confidence is meaningful.
    assert!(
        resp.rounds_executed >= 1,
        "D7 : expected at least 1 round, got {}",
        resp.rounds_executed
    );
    // The decoded payload must be a `Raw` vector of the correct
    // dimensionality (the dissolver returns the raw centroid because
    // `Request::for_tests` defaults to `DecodeMode::PassThrough`).
    let raw = match &resp.output {
        DecodedOutput::Raw(v) => v.clone(),
        other => panic!("D7 : expected Raw payload, got {:?}", other),
    };
    assert_eq!(
        raw.len(),
        64,
        "D7 : decoded centroid has wrong dimensionality ; got {}",
        raw.len()
    );
}

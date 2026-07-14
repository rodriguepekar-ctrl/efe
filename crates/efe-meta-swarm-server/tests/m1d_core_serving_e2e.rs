// SPDX-License-Identifier: Apache-2.0
//! **Couche 3 · M1.d — un VRAI `SwarmController` efe-core tourne une ronde
//! SERVIE (Step-sur-gRPC) — le DoD #1 du jalon M1.**
//!
//! Gated by `core-integration` : pulls efe-core + efe-server into the test
//! build.  Run with :
//!
//! ```text
//! cargo test -p efe-meta-swarm-server --features core-integration --test m1d_core_serving_e2e
//! ```
//!
//! Proves, over the REAL gRPC stack (`ParticipantService` bound on a real
//! socket, tonic client) :
//!
//! 1. an efe-core manifest row boots a served participant whose engine is the
//!    real bridge (`build_mvp_controller` → deterministic M1.b Φ) ;
//! 2. N `Step` RPCs run the coupled iteration : states stay FINITE, CONTAINED
//!    in the manifest safety box, and the successive-state deltas CONTRACT
//!    (κ_composite = (1−α−γ) + α·L̂ ≈ 0.3 + 0.5·0.69 ≈ 0.65 < 1) ;
//! 3. `Capabilities` advertises the HONEST wire : `engine:efe-core-mode1`
//!    token + 64-hex `config_hash`, kappa/seed sentinels 0.0/0 ;
//! 4. the H1 boot gate runs the real M1.c canary and passes on this box
//!    (property-tested, NOT certified) — and `mode: off` skips it ;
//! 5. `config_hash` is stable across rebuilds and sensitive to block edits.
//!
//! ## Honesty (never over-sell)
//!
//! The engine is efe-core's **numerical Mode-1 fusion on stub templates** —
//! real efe-core latent swarms, NOT SLMs.  The contraction asserted here is
//! EMPIRICAL (measured on this box), not a proof.

#![cfg(feature = "core-integration")]

mod common;

use std::time::Duration;

use efe_meta_swarm_proto::meta_swarm::{CapabilitiesRequest, StepRequest, TensorF32};
use efe_meta_swarm_proto::participant::MetaParticipantServiceClient;
use efe_meta_swarm_server::manifest::{
    AlignmentConeManifest, ControllerManifest, H1CanaryManifest, ParticipantManifest,
    PopulationManifest, SafetyBoxManifest, TopologyManifest,
};
use efe_meta_swarm_server::participant::{IterationParams, LocalParticipant};
use tonic::transport::Endpoint;
use tonic::Request;

const DIM: usize = 16;
const BOX_LO: f32 = 0.05;
const BOX_HI: f32 = 0.30;

fn engine_block(h1_mode: &str) -> ControllerManifest {
    engine_block_with_topology(h1_mode, TopologyManifest::Fixed)
}

fn engine_block_with_topology(h1_mode: &str, topology: TopologyManifest) -> ControllerManifest {
    ControllerManifest {
        schema: 1,
        population: vec![PopulationManifest { template: "na-sem".into(), count: 4 }],
        max_rounds: 3,
        alignment_cone: AlignmentConeManifest {
            anchors: vec![vec![0.1; DIM]],
            max_projection_distance: 0.5,
        },
        safety_box: SafetyBoxManifest { lo: BOX_LO, hi: BOX_HI },
        topology,
        tenant_id: "tenant-m1d".into(),
        h1_canary: H1CanaryManifest {
            mode: h1_mode.into(),
            // A TINY boot probe (~18 Φ evals ≈ 15-60 s) — the dedicated boot
            // test budget ;  production uses the larger schema defaults.
            n_seeds: 6,
            n_pairs: 8,
            n_directions: 1,
            n_bisect_pairs: 1,
            bisect_depth: 6,
            max_evals: 24,
            margin: 0.05,
        },
    }
}

fn engine_row(id: &str, h1_mode: &str) -> ParticipantManifest {
    ParticipantManifest {
        id: id.into(),
        dim: DIM as u32,
        kappa: None,
        seed: None,
        blas_backend: "none".into(),
        controller: Some(engine_block(h1_mode)),
    }
}

async fn connect(
    addr: std::net::SocketAddr,
) -> MetaParticipantServiceClient<tonic::transport::Channel> {
    let channel = Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        // A real Φ costs ~0.5-3 s per Step — far above the mock's µs budget.
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .expect("connect");
    MetaParticipantServiceClient::new(channel)
}

/// **DoD #1** — the real controller runs a SERVED round over gRPC : N Steps,
/// finite, contained in the safety box, contracting.  Plus the honest wire.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_swarm_controller_serves_steps_over_grpc() {
    // Canary off here — the boot gate has its own dedicated test below ;
    // this one is about the SERVED round (DoD #1).
    let (addr, _h) = common::spawn_participant_service(
        engine_row("core-p", "off"),
        IterationParams { alpha_default: 0.5, coupling_gamma: 0.2 },
    )
    .await;
    let mut client = connect(addr).await;

    // ── Honest wire (Capabilities) ────────────────────────────────────
    let caps = client
        .capabilities(Request::new(CapabilitiesRequest {}))
        .await
        .expect("capabilities")
        .into_inner();
    let d = caps.descriptor.expect("descriptor present");
    assert_eq!(d.dim as usize, DIM);
    assert!(
        d.features.iter().any(|f| f == "engine:efe-core-mode1"),
        "the engine token must be advertised, got {:?}",
        d.features,
    );
    let hash = d.config_hash.expect("an efe-core participant advertises its config_hash");
    assert_eq!(hash.len(), 64, "hex SHA-256");
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert!((d.kappa - 0.0).abs() < f32::EPSILON, "kappa sentinel for efe-core rows");
    assert_eq!(d.seed, 0, "seed sentinel for efe-core rows");
    assert!(d.features.iter().any(|f| f == "determinism"), "M1.b Φ is deterministic");

    // ── The served round : N coupled Steps over gRPC ──────────────────
    // pull inside the box → the coupled map contracts (empirically ~0.65).
    let pull = vec![0.1_f32; DIM];
    let mut prev: Option<Vec<f32>> = None;
    let mut deltas = Vec::new();
    for step_index in 0..6_u32 {
        let resp = client
            .step(Request::new(StepRequest {
                pulled_from_meta: Some(TensorF32 { dim: DIM as u32, data: pull.clone() }),
                step_index,
            }))
            .await
            .expect("a REAL served Step succeeds")
            .into_inner();
        let state = resp.new_state.expect("new_state present");
        assert_eq!(state.dim as usize, DIM);
        assert!(
            state.data.iter().all(|v| v.is_finite()),
            "step {step_index}: the real fusion must not emit NaN/Inf",
        );
        assert!(
            state.data.iter().all(|v| (BOX_LO - 1e-3..=BOX_HI + 1e-3).contains(v)),
            "step {step_index}: state must stay inside the manifest safety box \
             [{BOX_LO}, {BOX_HI}], got {:?}",
            state.data,
        );
        if let Some(p) = &prev {
            let delta: f64 = state
                .data
                .iter()
                .zip(p)
                .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                .sum::<f64>()
                .sqrt();
            deltas.push(delta);
        }
        prev = Some(state.data);
    }
    // Contraction : the last successive delta must be well below the first
    // (empirical composite κ ≈ 0.65 per step ⇒ ×0.65⁴ ≈ 0.18 over 4 gaps).
    let (first, last) = (deltas[0], *deltas.last().unwrap());
    assert!(
        last <= first * 0.5 + 1e-9,
        "the served coupled iteration must contract : deltas = {deltas:?}",
    );

    // ── State / Reset round-trip ───────────────────────────────────────
    let st = client
        .state(Request::new(efe_meta_swarm_proto::meta_swarm::StateRequest {}))
        .await
        .expect("state")
        .into_inner();
    assert_eq!(st.step_index_last, 5, "the last successful step index is reported");
    let rg = client
        .reset(Request::new(efe_meta_swarm_proto::meta_swarm::ResetParticipantRequest {
            initial: None,
        }))
        .await
        .expect("reset")
        .into_inner();
    assert_eq!(rg.reset_gen, 1, "reset bumps the generation");
}

/// **The H1 boot gate over the REAL canary** : `mode: boot` runs the M1.c
/// adversarial Lipschitz canary on the box and PASSES here (L̂ ≈ 0.69 < 0.95)
/// — property-tested, not certified ;  `mode: off` skips it instantly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_boot_gate_runs_the_real_canary() {
    // mode: off — skip, immediate.
    let p_off = LocalParticipant::from_manifest_row(&engine_row("core-off", "off"))
        .expect("build (canary off)");
    p_off.h1_boot_gate().await.expect("mode=off skips the canary");

    // mode: boot — the real canary runs (~18 Φ evals) and must pass on this
    // within-cone box (M1.c measured L̂ = 0.691 with a larger probe).
    let p_boot = LocalParticipant::from_manifest_row(&engine_row("core-boot", "boot"))
        .expect("build (canary boot)");
    p_boot
        .h1_boot_gate()
        .await
        .expect("the real Φ is non-expansive on this box — the canary must pass");
}

/// **M1 Palier-2 — the explicit DYNAMIC opt-out still serves.**  Coverage of
/// the legacy (x-dependent genesis) profile : the participant boots, serves a
/// Step, and its wire descriptor HONESTLY carries `topology:dynamic`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dynamic_topology_opt_out_serves_and_is_advertised() {
    let mut row = engine_row("core-dyn", "off");
    row.controller = Some(engine_block_with_topology("off", TopologyManifest::Dynamic));
    let (addr, _h) = common::spawn_participant_service(
        row,
        IterationParams { alpha_default: 0.5, coupling_gamma: 0.2 },
    )
    .await;
    let mut client = connect(addr).await;

    let caps = client
        .capabilities(Request::new(CapabilitiesRequest {}))
        .await
        .expect("capabilities")
        .into_inner();
    let d = caps.descriptor.expect("descriptor");
    assert!(
        d.features.iter().any(|f| f == "topology:dynamic"),
        "the dynamic opt-out must be visible on the wire, got {:?}",
        d.features,
    );

    let resp = client
        .step(Request::new(StepRequest {
            pulled_from_meta: Some(TensorF32 { dim: DIM as u32, data: vec![0.1; DIM] }),
            step_index: 0,
        }))
        .await
        .expect("a dynamic-topology Step still serves")
        .into_inner();
    assert!(resp.new_state.expect("state").data.iter().all(|v| v.is_finite()));
}

/// **The Palier-2 measurement — the experiment that arbitrates the doc's
/// β₁ claim.**  M1.c measured L̂ ≈ 0.69 on the DYNAMIC operator ;  the
/// architecture's closed-form analysis (§1.1) predicts ≤ β₁ ≈ 0.17
/// *intra-cell*.  Under FIXED topology there are no routing cells at all :
/// if the 0.69 came from boundary jumps, L̂_fixed should collapse toward β₁ ;
/// if it persists, the closed-form analysis is too optimistic and must never
/// be minted as a certificate.  Either way the HONEST claim stays the
/// measured one.  The test asserts only H1 (< 1 − margin) for the fixed
/// operator and PRINTS both bounds for the record.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn palier2_measurement_fixed_vs_dynamic_lipschitz() {
    use efe_meta_swarm_server::participant::LocalParticipant;
    let cert_of = |topology: TopologyManifest| async move {
        let mut row = engine_row("core-measure", "boot");
        row.controller = Some(engine_block_with_topology("boot", topology));
        let p = LocalParticipant::from_manifest_row(&row).expect("build");
        p.h1_certificate()
            .await
            .expect("canary ran")
            .expect("efe-core row yields a certificate")
    };

    let fixed = cert_of(TopologyManifest::Fixed).await;
    let dynamic = cert_of(TopologyManifest::Dynamic).await;

    use efe_meta_swarm::participants::NonExpansiveCertificate as C;
    let (C::Empirical { l_hat: l_fixed, inconclusive: inc_f, .. },
         C::Empirical { l_hat: l_dyn, inconclusive: inc_d, .. }) = (&fixed, &dynamic)
    else {
        panic!("expected Empirical certificates, got {fixed:?} / {dynamic:?}");
    };
    eprintln!(
        "PALIER-2 MEASUREMENT  L̂_fixed={l_fixed:.5} (inconclusive={inc_f})  \
         L̂_dynamic={l_dyn:.5} (inconclusive={inc_d})  — doc §1.1 predicted ≤ β₁ ≈ 0.17 ; \
         the HONEST claim is the measured bound (property-tested, NOT certified)"
    );
    assert!(!inc_f, "the fixed-topology canary must be conclusive on this box");
    assert!(
        fixed.accepts(false),
        "H1 must hold (L̂ < 1 − margin) for the fixed-topology operator, got L̂ = {l_fixed}",
    );
}

/// `config_hash` is a stable function of the block (same block ⇒ same hash ;
/// any edit ⇒ different hash) — the reload-drift and wire identity anchor.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_hash_is_stable_and_edit_sensitive() {
    let a = LocalParticipant::from_manifest_row(&engine_row("p", "off")).expect("build a");
    let b = LocalParticipant::from_manifest_row(&engine_row("p", "off")).expect("build b");
    assert_eq!(
        a.config_hash().unwrap(),
        b.config_hash().unwrap(),
        "identical blocks must hash identically",
    );

    let mut row = engine_row("p", "off");
    row.controller.as_mut().unwrap().max_rounds = 5;
    let c = LocalParticipant::from_manifest_row(&row).expect("build c");
    assert_ne!(
        a.config_hash().unwrap(),
        c.config_hash().unwrap(),
        "an edited block must hash differently",
    );

    // An unknown template code fails construction closed.
    let mut row = engine_row("p", "off");
    row.controller.as_mut().unwrap().population[0].template = "na-unknown".into();
    let err = LocalParticipant::from_manifest_row(&row).unwrap_err();
    assert!(format!("{err}").contains("unknown template code"), "{err}");
}

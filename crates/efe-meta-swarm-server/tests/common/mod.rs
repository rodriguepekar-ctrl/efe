//! Shared helpers for integration tests.
//!
//! Spawning a *real* `ParticipantService` is the production path —
//! the test stub `TestParticipantSvc` (introduced in PR #8) is now
//! deleted and replaced by `spawn_participant_service` here.

#![allow(dead_code)] // each integration-test binary picks what it needs

#[cfg(unix)]
pub mod multiprocess;

use std::net::SocketAddr;
use std::time::Duration;

use efe_meta_swarm_proto::participant::MetaParticipantServiceServer;
use efe_meta_swarm_server::config::participant::ParticipantConfig;
use efe_meta_swarm_server::manifest::ParticipantManifest;
use efe_meta_swarm_server::participant::{
    IterationParams, LocalParticipant, ParticipantService, ParticipantState,
};
use tonic::transport::Server;

/// Bind + immediately release a localhost port.  Tiny TOCTOU window
/// before the caller re-binds ; on `127.0.0.1` in test mode it's
/// empirically negligible (cf. PR #8's smoke test).
pub async fn allocate_port() -> SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0");
    let addr = l.local_addr().expect("local_addr");
    drop(l);
    addr
}

/// Spawn a real `ParticipantService` on a fresh localhost port.
/// Returns `(addr, JoinHandle)`.  The JoinHandle keeps the task alive
/// until the test's runtime drops.
pub async fn spawn_participant_service(
    row: ParticipantManifest,
    iteration: IterationParams,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let id = row.id.clone();
    let local =
        LocalParticipant::from_manifest_row(&row).expect("LocalParticipant::from_manifest_row");
    let config = ParticipantConfig {
        id: id.clone(),
        grpc_bind: "127.0.0.1:0".parse().unwrap(),
        metrics_bind: "127.0.0.1:0".parse().unwrap(),
        worker_threads: 1,
        tls_required: false,
        tls_cert_path: None,
        tls_key_path: None,
        tls_ca_path: None,
        // PR #16 — defaults : TLS off, no mTLS, TLS 1.3 minimum (only
        // consulted when `tls_required: true`).
        tls_mutual: false,
        tls_min_protocol: efe_meta_swarm_server::tls::ProtocolVersion::Tls13,
        tls_allowed_peer_sans: Vec::new(),
        tls_peer_san: None,
    };
    let state = ParticipantState::new(config, local, iteration);
    let svc = ParticipantService::new(state);

    let addr = allocate_port().await;
    let handle = tokio::spawn(async move {
        if let Err(e) = Server::builder()
            .add_service(MetaParticipantServiceServer::new(svc))
            .serve(addr)
            .await
        {
            eprintln!("test participant {id} server exited: {e}");
        }
    });

    // Wait up to 1 s for the bind to land.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return (addr, handle);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("test participant did not come up at {addr}");
}

/// Convenience : spawn a participant from `(id, dim, kappa, seed)`
/// with the default Phase-2 iteration knobs (α = 0.5, γ = 0.2).
pub async fn spawn_participant_simple(
    id: &str,
    dim: u32,
    kappa: f32,
    seed: u64,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_participant_service(
        ParticipantManifest {
            id: id.into(),
            dim,
            kappa: Some(kappa),
            seed: Some(seed),
            blas_backend: "none".into(),
            controller: None,
        },
        IterationParams {
            alpha_default: 0.5,
            coupling_gamma: 0.2,
        },
    )
    .await
}

/// Build a `ControllerState` pointing at the given participants.
/// Each tuple is `(id, dim, kappa, seed, SocketAddr)`.
///
/// Used by `integration_e2e_iterate.rs` and (after PR #9)
/// `integration_iterate_inproc.rs`.
pub fn build_controller_state_for(
    statics: &[(String, usize, f32, u64, SocketAddr)],
    max_inflight: usize,
    meta_dim: usize,
) -> efe_meta_swarm_server::controller::ControllerState {
    build_controller_state_with_iteration(
        statics,
        max_inflight,
        meta_dim,
        efe_meta_swarm_server::manifest::IterationManifest {
            max_iters: 100,
            tolerance: 1.0e-4,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 15_000,
            require_convergence: None, // non stipulé (participants mock ⇒ résout à false)
        },
    )
}

/// Couche 3 M1.d — like [`build_controller_state_for`] but with explicit
/// iteration knobs (e.g. `require_convergence: true` for the fail-closed
/// non-convergence tests).
pub fn build_controller_state_with_iteration(
    statics: &[(String, usize, f32, u64, SocketAddr)],
    max_inflight: usize,
    meta_dim: usize,
    iteration: efe_meta_swarm_server::manifest::IterationManifest,
) -> efe_meta_swarm_server::controller::ControllerState {
    use efe_meta_swarm_server::config::controller::{
        ControllerConfig, DiscoveryMode, StaticParticipant,
    };
    use efe_meta_swarm_server::manifest::{Manifest, MetaSpaceManifest, SafetyManifest};

    let static_participants: Vec<StaticParticipant> = statics
        .iter()
        .map(|(id, _, _, _, addr)| StaticParticipant {
            id: id.clone(),
            addr: addr.to_string(),
        })
        .collect();
    let config = ControllerConfig {
        grpc_bind: "127.0.0.1:0".parse().unwrap(),
        metrics_bind: "127.0.0.1:0".parse().unwrap(),
        discovery: DiscoveryMode::StaticLocalhost,
        static_participants,
        manifest_path: "/dev/null".into(),
        max_inflight,
        default_max_wall_ms: 15_000,
        worker_threads: 1,
        reflection_enabled: false,
        tls_required: false,
        tls_cert_path: None,
        tls_key_path: None,
        tls_ca_path: None,
        // PR #16 — TLS knobs default to off ; integration tests
        // don't exercise the mTLS path.
        tls_mutual: false,
        tls_min_protocol: efe_meta_swarm_server::tls::ProtocolVersion::Tls13,
        tls_allowed_peer_sans: Vec::new(),
        tls_peer_san: None,
        drain_timeout: Duration::from_secs(30),
    };

    let participants = statics
        .iter()
        .map(|(id, dim, kappa, seed, _)| ParticipantManifest {
            id: id.clone(),
            dim: *dim as u32,
            kappa: Some(*kappa),
            seed: Some(*seed),
            blas_backend: "none".into(),
            controller: None,
        })
        .collect();
    let manifest = Manifest {
        version: "v0.2.0-smoke".into(),
        meta: MetaSpaceManifest {
            dim: meta_dim as u32,
            safety: SafetyManifest::Box { radius: 50.0 },
        },
        participants,
        iteration,
    };

    efe_meta_swarm_server::controller::ControllerState::new(config, manifest)
}

/// Build a well-formed `IterateRequest` for the integration tests.
pub fn iterate_request(
    meta_dim: u32,
    max_iters: u32,
    tolerance: f64,
) -> efe_meta_swarm_proto::meta_swarm::IterateRequest {
    use efe_meta_swarm_proto::meta_swarm::{
        meta_space_config::SafetyGeometry, BoxSafety as ProtoBoxSafety, IterateRequest,
        IterationConfig, MetaSpaceConfig,
    };
    IterateRequest {
        expected_participants: vec![],
        meta: Some(MetaSpaceConfig {
            dim: meta_dim,
            safety_geometry: Some(SafetyGeometry::Box(ProtoBoxSafety { radius: 50.0 })),
        }),
        iteration: Some(IterationConfig {
            max_iters,
            tolerance,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 10_000,
        }),
        initial_states: vec![],
        traceparent: String::new(),
        // Couche 3 M2 — par défaut on ne demande PAS le reçu complet : le root suffit à la
        // plupart des tests, et le bundle coûte O(itérations × dim).
        include_provenance_bundle: false,
    }
}

/// Couche 3 M2 — la même requête, mais qui RÉCLAME le reçu re-prouvable complet.
#[allow(dead_code)]
pub fn iterate_request_with_receipt(
    meta_dim: u32,
    max_iters: u32,
    tolerance: f64,
) -> efe_meta_swarm_proto::meta_swarm::IterateRequest {
    efe_meta_swarm_proto::meta_swarm::IterateRequest {
        include_provenance_bundle: true,
        ..iterate_request(meta_dim, max_iters, tolerance)
    }
}

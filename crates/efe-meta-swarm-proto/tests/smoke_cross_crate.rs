//! Smoke test : verifies that the crate compiles AND that its public
//! surface (constants, modules, types) matches the documented
//! contract.
//!
//! This test is the **load-bearing pin** for the proto versioning
//! policy : any change that breaks one of these assertions is a v2
//! break and requires the full RFC process (cf. README "Breaking-
//! change protocol").
//!
//! Run with :
//!
//! ```bash
//! cargo test -p efe-meta-swarm-proto --test smoke_cross_crate
//! cargo test -p efe-meta-swarm-proto --test smoke_cross_crate --features full
//! ```

use efe_meta_swarm_proto::{
    conv,
    meta_swarm::{
        BoxSafety, CapabilitiesResponse, HealthResponse, IterateChunk, IterateRequest,
        IterateResponse, IterationConfig, MetaSpaceConfig, MetaSwarmErrorCode,
        ParticipantDescriptor, StepRequest, StepResponse, StopReason, TensorF32,
    },
    ACTUAL_RPC_NAMES, PROTO_PACKAGE, PROTO_VERSION, RPC_NAMES,
};

// Client traits are only available off the `server-only` feature.
// Gated imports keep the smoke test runnable on every feature combo.
#[cfg(not(feature = "server-only"))]
use efe_meta_swarm_proto::{
    controller::MetaSwarmServiceClient, participant::MetaParticipantServiceClient,
};

// ---------------------------------------------------------------------------
//  Pin tests — break iff someone violates the versioning policy.
// ---------------------------------------------------------------------------

/// The package name as it appears on the wire is `efe.v1.meta_swarm`.
/// Changing it is a v2 break.
#[test]
fn proto_package_pinned() {
    assert_eq!(PROTO_PACKAGE, "efe.v1.meta_swarm");
}

/// The version constant is `v1` for the entire v1 surface (the same
/// value the server returns in `InfoResponse.proto_version`).
#[test]
fn proto_version_pinned() {
    assert_eq!(PROTO_VERSION, "v1");
}

/// The RPC catalogue has exactly 13 entries (7 in MetaSwarmService +
/// 6 in MetaParticipantService).  Adding one without updating
/// `RPC_NAMES` in lib.rs will fail this test.
#[test]
fn rpc_count_pinned() {
    assert_eq!(
        RPC_NAMES.len(),
        13,
        "RPC count drift : check RPC_NAMES in lib.rs (got {} names)",
        RPC_NAMES.len(),
    );
}

/// Drift detector : the hand-maintained `RPC_NAMES` constant in
/// `lib.rs` MUST match the RPCs actually declared in the .proto
/// files (parsed at build time by `build.rs::scan_rpc_names`).
///
/// This catches the mutation where someone adds an `rpc Foo (...)`
/// line to a proto but forgets to update `RPC_NAMES` (a regression
/// the earlier `rpc_count_pinned` test alone cannot detect).
#[test]
fn rpc_names_match_protos() {
    let manual: Vec<&str> = RPC_NAMES.to_vec();
    let actual: Vec<&str> = ACTUAL_RPC_NAMES.to_vec();
    assert_eq!(
        manual, actual,
        "RPC_NAMES drift : the hand-maintained list in lib.rs does \
         NOT match the rpc declarations in protos/efe/v1/meta_swarm/*.\
         proto.\n  RPC_NAMES (lib.rs)        = {manual:?}\n  \
         ACTUAL_RPC_NAMES (protos)  = {actual:?}\nUpdate lib.rs to match.",
    );
}

/// The 7 RPCs of `MetaSwarmService` are present in `RPC_NAMES`.
#[test]
fn meta_swarm_service_rpcs_present() {
    let expected: &[&str] = &[
        "Iterate",
        "IterateStream",
        "StepOnce",
        "Reset",
        "ReloadConfig",
        "Health",
        "Info",
    ];
    for r in expected {
        assert!(
            RPC_NAMES.contains(r),
            "MetaSwarmService RPC missing from RPC_NAMES : {r}",
        );
    }
}

/// The 6 RPCs of `MetaParticipantService` are present in `RPC_NAMES`.
#[test]
fn meta_participant_service_rpcs_present() {
    let expected: &[&str] = &[
        "Step",
        "Reset", // duplicated with MetaSwarmService — intentional
        "State",
        "StepStream",
        "Capabilities",
        "Health", // duplicated — intentional
    ];
    for r in expected {
        assert!(
            RPC_NAMES.contains(r),
            "MetaParticipantService RPC missing from RPC_NAMES : {r}",
        );
    }
}

// ---------------------------------------------------------------------------
//  Default constructibility — tonic-build regression catcher.
// ---------------------------------------------------------------------------

#[test]
fn messages_default_construct() {
    let _ = IterateRequest::default();
    let _ = IterateResponse::default();
    let _ = IterateChunk::default();
    let _ = StepRequest::default();
    let _ = StepResponse::default();
    let _ = TensorF32::default();
    let _ = ParticipantDescriptor::default();
    let _ = MetaSpaceConfig::default();
    let _ = BoxSafety::default();
    let _ = IterationConfig::default();
    let _ = HealthResponse::default();
    let _ = CapabilitiesResponse::default();
}

/// Enum default values are the `UNSPECIFIED` variant (= 0).  This
/// matches proto3 semantics.
#[test]
fn enum_defaults_are_unspecified() {
    let s = StopReason::default();
    assert_eq!(s as i32, 0);
    let e = MetaSwarmErrorCode::default();
    assert_eq!(e as i32, 0);
    let h = HealthResponse::default();
    assert_eq!(h.status, 0);
}

// ---------------------------------------------------------------------------
//  Wire-format round trip — prost encode/decode regression catcher.
// ---------------------------------------------------------------------------

#[test]
fn iterate_request_roundtrip() {
    use efe_meta_swarm_proto::meta_swarm::meta_space_config::SafetyGeometry;
    use prost::Message;

    let original = IterateRequest {
        expected_participants: vec![],
        meta: Some(MetaSpaceConfig {
            dim: 256,
            safety_geometry: Some(SafetyGeometry::Box(BoxSafety { radius: 10.0 })),
        }),
        iteration: Some(IterationConfig {
            max_iters: 100,
            tolerance: 1e-4,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 15_000,
        }),
        initial_states: vec![],
        traceparent: String::new(),
    };

    let mut bytes = Vec::new();
    original.encode(&mut bytes).expect("encode");
    let decoded = IterateRequest::decode(&*bytes).expect("decode");

    let it = decoded.iteration.as_ref().expect("iteration round-tripped");
    assert_eq!(it.max_iters, 100);
    assert!((it.tolerance - 1.0e-4).abs() < 1.0e-12);
    assert_eq!(it.max_wall_ms, 15_000);

    let meta = decoded.meta.as_ref().expect("meta round-tripped");
    assert_eq!(meta.dim, 256);
    match meta.safety_geometry.as_ref().expect("oneof present") {
        SafetyGeometry::Box(b) => assert!((b.radius - 10.0).abs() < f32::EPSILON),
        _ => panic!("expected BoxSafety oneof"),
    }
}

// ---------------------------------------------------------------------------
//  conv helpers regression.
// ---------------------------------------------------------------------------

#[test]
fn conv_tensor_from_slice() {
    let data: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
    let t = conv::tensor_from_slice(&data);
    assert_eq!(t.dim, 4);
    assert_eq!(t.data, vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn conv_tensor_validate_rejects_mismatch() {
    let bad = TensorF32 {
        dim: 5,
        data: vec![1.0, 2.0],
    };
    assert!(conv::tensor_validate(&bad).is_err());

    let good = TensorF32 {
        dim: 2,
        data: vec![1.0, 2.0],
    };
    let slice = conv::tensor_validate(&good).expect("valid tensor");
    assert_eq!(slice.len(), 2);
    assert_eq!(slice[0], 1.0);
    assert_eq!(slice[1], 2.0);
}

// ---------------------------------------------------------------------------
//  Compile-time : client traits accept a Channel.
// ---------------------------------------------------------------------------

/// Client traits exist and accept a transport `Channel`.  Skipped in
/// `server-only` builds (clients are not generated there).
#[cfg(not(feature = "server-only"))]
#[test]
fn clients_compile_with_channel() {
    fn _accept_meta_swarm_client(_c: MetaSwarmServiceClient<tonic::transport::Channel>) {}
    fn _accept_meta_participant_client(
        _c: MetaParticipantServiceClient<tonic::transport::Channel>,
    ) {
    }
    // No body — the type signatures alone are the test.  If
    // tonic-build moves the client structs, this fails to compile
    // with a clear error.
}

// ---------------------------------------------------------------------------
//  serde feature — gated.
// ---------------------------------------------------------------------------

#[cfg(feature = "serde-derives")]
#[test]
fn serde_round_trip_json() {
    let t = TensorF32 {
        dim: 3,
        data: vec![1.0, 2.0, 3.0],
    };
    let json = serde_json::to_string(&t).expect("serialize");
    assert!(json.contains("\"dim\":3"));

    let back: TensorF32 = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.dim, 3);
    assert_eq!(back.data, vec![1.0, 2.0, 3.0]);
}

/// The `oneof safety_geometry` enum (`SafetyGeometry`, generated
/// inside `meta_space_config`) is the trickiest serde target because
/// prost-build emits it as a `pub enum` without explicit tag
/// hints — serde derives must apply via the `type_attribute(".")`
/// glob in `build.rs`.  This test pins that the round-trip works
/// end-to-end for every oneof variant.
#[cfg(feature = "serde-derives")]
#[test]
fn serde_round_trip_oneof_safety_geometry() {
    use efe_meta_swarm_proto::meta_swarm::meta_space_config::SafetyGeometry;
    use efe_meta_swarm_proto::meta_swarm::{LearnedHull, Polyhedron};

    // Variant 1 : Box.
    let cfg_box = MetaSpaceConfig {
        dim: 256,
        safety_geometry: Some(SafetyGeometry::Box(BoxSafety { radius: 10.0 })),
    };
    let json = serde_json::to_string(&cfg_box).expect("serialize Box");
    let back: MetaSpaceConfig = serde_json::from_str(&json).expect("deserialize Box");
    assert_eq!(back.dim, 256);
    match back.safety_geometry.as_ref().expect("oneof present") {
        SafetyGeometry::Box(b) => assert!((b.radius - 10.0).abs() < f32::EPSILON),
        _ => panic!("expected SafetyGeometry::Box variant after round-trip"),
    }

    // Variant 2 : Poly.
    let cfg_poly = MetaSpaceConfig {
        dim: 4,
        safety_geometry: Some(SafetyGeometry::Poly(Polyhedron {
            a_flat: vec![1.0, 0.0, 0.0, 1.0],
            b: vec![1.0, 1.0],
            dim_a: 2,
        })),
    };
    let json = serde_json::to_string(&cfg_poly).expect("serialize Poly");
    let back: MetaSpaceConfig = serde_json::from_str(&json).expect("deserialize Poly");
    match back.safety_geometry.as_ref().expect("oneof present") {
        SafetyGeometry::Poly(p) => {
            assert_eq!(p.dim_a, 2);
            assert_eq!(p.a_flat.len(), 4);
        }
        _ => panic!("expected SafetyGeometry::Poly variant after round-trip"),
    }

    // Variant 3 : Hull (placeholder).
    let cfg_hull = MetaSpaceConfig {
        dim: 32,
        safety_geometry: Some(SafetyGeometry::Hull(LearnedHull {
            serialized_model: vec![0xde, 0xad, 0xbe, 0xef],
            version: "v0.1.0".into(),
        })),
    };
    let json = serde_json::to_string(&cfg_hull).expect("serialize Hull");
    let back: MetaSpaceConfig = serde_json::from_str(&json).expect("deserialize Hull");
    match back.safety_geometry.as_ref().expect("oneof present") {
        SafetyGeometry::Hull(h) => {
            assert_eq!(h.serialized_model, vec![0xde, 0xad, 0xbe, 0xef]);
            assert_eq!(h.version, "v0.1.0");
        }
        _ => panic!("expected SafetyGeometry::Hull variant after round-trip"),
    }
}

// ---------------------------------------------------------------------------
//  Edge cases : empty, large, UTF-8, NaN.
// ---------------------------------------------------------------------------

/// Empty TensorF32 (dim=0) round-trips correctly.  This is a valid
/// edge case — `MetaSwarm::reset()` may emit zero-dim tensors when a
/// participant is being torn down.
#[test]
fn tensor_empty_roundtrip() {
    use prost::Message;
    let empty = TensorF32 {
        dim: 0,
        data: vec![],
    };
    let mut bytes = Vec::new();
    empty.encode(&mut bytes).expect("encode");
    let back = TensorF32::decode(&*bytes).expect("decode");
    assert_eq!(back.dim, 0);
    assert!(back.data.is_empty());
}

/// Large TensorF32 (dim = 100k) round-trips correctly.  Tests prost's
/// varint encoding of large dim + packed `repeated float` payload.
#[test]
fn tensor_large_roundtrip() {
    use prost::Message;
    let big = TensorF32 {
        dim: 100_000,
        data: (0..100_000).map(|i| i as f32 * 0.001).collect(),
    };
    let mut bytes = Vec::with_capacity(420_000);
    big.encode(&mut bytes).expect("encode");
    // Wire size : ~ 4 bytes (tag + varint dim) + 4 × 100000 bytes (packed f32) + tag overhead.
    assert!(
        bytes.len() >= 400_000,
        "wire size too small : {}",
        bytes.len()
    );
    assert!(
        bytes.len() <= 410_000,
        "wire size too large : {}",
        bytes.len()
    );
    let back = TensorF32::decode(&*bytes).expect("decode");
    assert_eq!(back.dim, 100_000);
    assert_eq!(back.data.len(), 100_000);
    assert!((back.data[42] - 0.042).abs() < 1e-6);
}

/// UTF-8 strings in `ParticipantDescriptor.id` round-trip correctly.
/// (Phase-3 may have tenant ids with non-ASCII chars.)
#[test]
fn participant_descriptor_utf8_roundtrip() {
    use prost::Message;
    let pd = ParticipantDescriptor {
        id: "supplier-スコアリング".into(),
        dim: 384,
        kappa: 0.6,
        seed: 11,
        build_sha: "abc123".into(),
        blas_backend: "none".into(),
        features: vec!["determinism".into(), "métriques".into()],
    };
    let mut bytes = Vec::new();
    pd.encode(&mut bytes).expect("encode");
    let back = ParticipantDescriptor::decode(&*bytes).expect("decode");
    assert_eq!(back.id, "supplier-スコアリング");
    assert_eq!(back.features, vec!["determinism", "métriques"]);
}

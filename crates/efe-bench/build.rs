//! Build script: compile the `.proto` subset consumed by the bench
//! harness with `tonic-build`. Mirrors `efe-server/build.rs` so the
//! generated stubs land at `crate::proto::v1::{efe, catalog, mode3, ...}`
//! and the wire types stay byte-compatible.
//!
//! Sprint **D29 (Φ-D)** — gated on the `tonic` cargo feature. With
//! the feature OFF (default), the build is a no-op and the bench
//! harness compiles without `protoc`. With the feature ON, we
//! compile the same `efe.v1` proto family that `efe-server` exposes
//! so `TonicBenchClient` can dispatch RPCs.

#[cfg(feature = "tonic")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::{Path, PathBuf};

    // Hard-wire `protoc` to the vendored binary so the build is
    // reproducible without a system-wide `protoc` install. Respect
    // `PROTOC` if already set (CI / nix / custom toolchains).
    if std::env::var_os("PROTOC").is_none() {
        let vendored = protoc_bin_vendored::protoc_bin_path()?;
        std::env::set_var("PROTOC", vendored);
    }

    // Resolve repo-root `protos/` deterministically relative to this crate.
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let protos_root: PathBuf = manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(|p| p.join("protos"))
        .ok_or("protos/ dir not found two levels up")?;

    let efe_v1 = protos_root.join("efe/v1");
    let proto_files = [
        // Order matters for prost-build — leaf types first.
        efe_v1.join("types.proto"),
        efe_v1.join("efe.proto"),
        efe_v1.join("catalog.proto"),
        // Sprint **D38** — Mode-3 service stub for `mode3-k5` /
        // `mode3-k10` workloads. Without this proto the bench
        // harness can't dispatch `Mode3Service.Execute`.
        efe_v1.join("mode3.proto"),
    ];

    tonic_build::configure()
        // Bench is a CLIENT only — never builds server stubs.
        .build_server(false)
        .build_client(true)
        .compile_protos(&proto_files, &[protos_root.clone()])?;

    // Re-run when any .proto file changes.
    for f in &proto_files {
        println!("cargo:rerun-if-changed={}", f.display());
    }
    println!("cargo:rerun-if-changed={}", protos_root.display());

    Ok(())
}

#[cfg(not(feature = "tonic"))]
fn main() {
    // No-op build when the `tonic` feature is OFF. The bench
    // harness ships only `MockBenchClient` and the proto-derived
    // stubs are never referenced.
    println!("cargo:rerun-if-changed=build.rs");
}

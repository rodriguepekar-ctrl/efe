//! Build script: compile `.proto` files with `tonic-build` and emit
//! generated Rust modules + a file descriptor set for server reflection.

use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
        .expect("protos/ dir exists two levels up");

    let efe_v1 = protos_root.join("efe/v1");
    let proto_files = [
        efe_v1.join("types.proto"),
        efe_v1.join("efe.proto"),
        efe_v1.join("admin.proto"),
        // Sprint D12-bis : add the catalog admin service proto so
        // tonic-build generates the Rust stubs under
        // `crate::proto::v1::catalog`. Pre-D12-bis the catalog
        // protos were Python-only — Rust handler was missing,
        // tests/dashboards/alerts dependent on it were dormant.
        efe_v1.join("catalog.proto"),
        // ─────────────────────────────────────────────────────────
        // **Sprint D34** (M4 § 13) — Mode-2 service.
        // **Sprint D37** (M4 § 14) — Mode-3 service.
        //
        // Both protos add a service in their own package
        // (`efe.v1.mode2`, `efe.v1.mode3`) so they can grow new
        // RPCs without disturbing the wire-stable `efe.v1` package.
        // The Rust stubs land at `crate::proto::v1::mode2` /
        // `mode3` per the manual `tonic::include_proto!` mapping
        // in `lib.rs` (no codegen-side aliasing — a handler that
        // forgets to import the right module triggers a compile
        // error).
        // ─────────────────────────────────────────────────────────
        efe_v1.join("mode2.proto"),
        efe_v1.join("mode3.proto"),
    ];

    // Descriptor set written into OUT_DIR; embedded by the server for
    // `tonic-reflection`.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor_path = out_dir.join("efe_v1_descriptor.bin");

    tonic_build::configure()
        .build_server(true)
        .build_client(true) // required for integration tests + future embedded client use.
        .file_descriptor_set_path(&descriptor_path)
        .compile_protos(&proto_files, &[protos_root.clone()])?;

    // Re-run when any .proto file changes.
    for f in &proto_files {
        println!("cargo:rerun-if-changed={}", f.display());
    }
    println!("cargo:rerun-if-changed={}", protos_root.display());

    Ok(())
}

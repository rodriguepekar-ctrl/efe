//! Build script for `efe-meta-swarm-proto`.
//!
//! Compiles 4 `.proto` files into Rust client + server stubs via
//! `tonic-build` (which delegates to `prost-build`).  All
//! configuration is explicit — we do not rely on
//! `tonic_build::compile_protos()` defaults — because the resulting
//! code is shipped to production and we want every choice auditable.
//!
//! ## Outputs
//!
//! `OUT_DIR/efe.v1.meta_swarm.rs` — the single generated file
//! containing all proto messages + service traits.  This is included
//! in `src/lib.rs` via `tonic::include_proto!("efe.v1.meta_swarm")`.
//!
//! ## protoc
//!
//! The build script hard-wires `protoc` to the binary shipped by the
//! `protoc-bin-vendored` crate.  This matches `efe-server/build.rs`
//! and ensures the build is reproducible without a system-wide
//! `protoc` install.  Respect the `PROTOC` env var if already set
//! (CI, nix, custom toolchains).
//!
//! ## Re-run conditions
//!
//! We emit explicit `rerun-if-changed` for each `.proto` file so a
//! touch to a proto triggers a regeneration ; other crate-internal
//! changes (Cargo.toml updates, etc.) don't.

use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Step 1 : protoc resolution.
    if std::env::var_os("PROTOC").is_none() {
        let vendored = protoc_bin_vendored::protoc_bin_path()?;
        std::env::set_var("PROTOC", vendored);
    }

    // Step 2 : locate the workspace protos root deterministically.
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let protos_root: PathBuf = manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(|p| p.join("protos"))
        .ok_or("protos/ dir not found two levels up")?;

    // Step 3 : enumerate the 4 .proto files.  Order matters for
    // prost-build : leaf-types first (messages.proto + errors.proto),
    // then services (which import them).
    let meta_swarm_root = protos_root.join("efe/v1/meta_swarm");
    let proto_files: Vec<PathBuf> = [
        "messages.proto",
        "errors.proto",
        "service_meta_swarm.proto",
        "service_meta_participant.proto",
    ]
    .iter()
    .map(|p| meta_swarm_root.join(p))
    .collect();

    // Step 4 : cargo re-run directives.
    for p in &proto_files {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-changed=build.rs");

    // Step 5 : configure tonic-build.
    let mut builder = tonic_build::configure()
        // Enable / disable client + server based on cargo features.
        // (`client-only` → no server ; `server-only` → no client ;
        // default → both.)
        .build_client(!cfg!(feature = "server-only"))
        .build_server(!cfg!(feature = "client-only"))
        // We do NOT use any well-known types (`google.protobuf.*`),
        // so skip pulling them in.  Saves ~5s of build time.
        .compile_well_known_types(false);

    // Step 6 : optional serde derive injection.
    //
    // We apply serde derives to every generated message + every
    // generated enum (including nested `oneof` enums, which
    // prost-build generates inside the parent message's module).
    //
    // Glob-pattern rationale : a per-type allow-list would miss the
    // implicit `oneof` enums (e.g. `MetaSpaceConfig.SafetyGeometry`)
    // and break compilation.  The `"."` selector matches the proto
    // root and recursively covers everything.
    //
    // Cost : adds the two derives to ~ 30 types ; ~ 50 kB binary
    // size when the feature is on (acceptable for the bench HTML
    // emitter + CLI use cases that opt in).
    if cfg!(feature = "serde-derives") {
        builder =
            builder.type_attribute(".", "#[derive(::serde::Serialize, ::serde::Deserialize)]");
    }

    // Step 7 : compile.  This is the single fallible step ; failure
    // here aborts the build with a clear protoc diagnostic.
    builder.compile_protos(&proto_files, &[protos_root])?;

    // Step 8 : emit `rpc_names_actual.rs` with the ground-truth list
    // of RPC names parsed from the .proto files.  Consumed by the
    // smoke test to detect drift between the hand-maintained
    // `RPC_NAMES` constant in lib.rs and the actual proto contents.
    //
    // Catches the mutation where a developer adds an `rpc Foo (...)`
    // line to a service proto but forgets to update `RPC_NAMES`.
    let actual_rpc_names = scan_rpc_names(&proto_files)?;
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let rpc_names_path = out_dir.join("rpc_names_actual.rs");
    let rpc_names_src = render_rpc_names_module(&actual_rpc_names);
    std::fs::write(&rpc_names_path, rpc_names_src)?;

    Ok(())
}

/// Scan each `.proto` file for `rpc <Name> (...)` lines.  Order is
/// preserved : RPC names are emitted in file order, then in the order
/// they appear inside each file.  This matches the order in which
/// `RPC_NAMES` is declared in `src/lib.rs`.
fn scan_rpc_names(proto_files: &[PathBuf]) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for path in proto_files {
        let content = std::fs::read_to_string(path)?;
        for line in content.lines() {
            let trimmed = line.trim_start();
            // Match lines of the form `rpc <Name> (` and skip
            // commented-out lines (`// rpc Foo ...`).
            if let Some(rest) = trimmed.strip_prefix("rpc ") {
                // Ignore lines inside a // comment.
                if trimmed.starts_with("//") {
                    continue;
                }
                if let Some(open_paren) = rest.find('(') {
                    let name = rest[..open_paren].trim();
                    if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }
    Ok(names)
}

/// Render the generated module that declares `ACTUAL_RPC_NAMES`.
fn render_rpc_names_module(names: &[String]) -> String {
    let mut s = String::with_capacity(256 + names.len() * 32);
    s.push_str("// AUTO-GENERATED by build.rs — do not edit.\n");
    s.push_str("// Scans `protos/efe/v1/meta_swarm/*.proto` for `rpc <Name>` lines.\n");
    s.push_str("// Consumed by `tests/smoke_cross_crate.rs::rpc_names_match_protos`.\n");
    s.push_str("pub const ACTUAL_RPC_NAMES: &[&str] = &[\n");
    for n in names {
        s.push_str("    \"");
        s.push_str(n);
        s.push_str("\",\n");
    }
    s.push_str("];\n");
    s
}

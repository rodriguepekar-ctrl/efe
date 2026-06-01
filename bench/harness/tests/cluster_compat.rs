//! PR #23 § 21 + AC6 — compat shim regression test.
//!
//! After extracting `bench/harness/src/cluster/{kind,k3s,external}.rs`
//! into the workspace crate `efe-cluster-provisioner`, PR #21 callers
//! must continue to compile + behave byte-identically via the compat
//! shim at `bench/harness/src/cluster/mod.rs`.
//!
//! This 4-test file pins :
//!   1. The historical `from_cli(ProvisionerKind, …)` builder API.
//!   2. The 3-impl taxonomy : kind, k3s, external.
//!   3. The `BenchError::Cluster` / `Kubectl` exit-code mapping via
//!      `From<ProvisionError>`.
//!   4. The trait surface (`provision` / `teardown` / `name`).

use std::path::PathBuf;

use efe_bench_harness::cli::ProvisionerKind;
use efe_bench_harness::cluster::from_cli;
use efe_bench_harness::error::{BenchError, ExitCode};

// ─────────────────────────────────────────────────────────────────────
// AC6.1 — `from_cli` builds a provisioner of the expected kind.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ac6_from_cli_builds_kind_provisioner() {
    let p = from_cli(ProvisionerKind::Kind, None, "bench-test".into());
    assert_eq!(p.name(), "kind");
}

#[test]
fn ac6_from_cli_builds_external_provisioner() {
    let p = from_cli(
        ProvisionerKind::External,
        Some(PathBuf::from("/tmp/kc")),
        "bench-test".into(),
    );
    assert_eq!(p.name(), "external");
}

// ─────────────────────────────────────────────────────────────────────
// AC6.2 — `BenchError::Cluster` is reachable via the
// `From<ProvisionError>` adapter (extracted-crate error → bench error).
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ac6_provision_error_maps_to_bench_cluster_exit() {
    let pe = efe_cluster_provisioner::ProvisionError::Cluster {
        provisioner: "kind".into(),
        reason: "kind binary not on PATH".into(),
    };
    let be: BenchError = pe.into();
    assert_eq!(be.exit_code(), ExitCode::Cluster);
}

// ─────────────────────────────────────────────────────────────────────
// AC6.3 — `external` provisioner still rejects a missing kubeconfig
// via the historic `BenchError::Cluster` exit-2 path (i.e. PR #21
// scenario YAMLs that point at an external cluster with a bad
// kubeconfig continue to fail with the same exit code).
// ─────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────
// D-BF — coverage gap : the `k3s` provisioner shim was untested.
// Three provisioners are documented (kind, k3s, external) but only
// two had compat tests.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_bf_from_cli_builds_k3s_provisioner() {
    let p = from_cli(ProvisionerKind::K3s, None, "bench-test".into());
    assert_eq!(p.name(), "k3s");
}

#[test]
fn d_bf_provision_error_kubectl_maps_to_cluster_exit() {
    // PR #23 AC6 sub-invariant : `ProvisionError::Kubectl` maps to
    // `BenchError::Kubectl` → exit Cluster (2).  Untested previously.
    let pe = efe_cluster_provisioner::ProvisionError::Kubectl {
        command: "apply -f x.yaml".into(),
        stderr: "no such file".into(),
    };
    let be: BenchError = pe.into();
    assert_eq!(be.exit_code(), ExitCode::Cluster);
}

#[test]
fn d_bf_provision_error_io_maps_to_io_internal() {
    // The third ProvisionError variant : Io → BenchError::Io → Internal (5).
    let pe = efe_cluster_provisioner::ProvisionError::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "x",
    ));
    let be: BenchError = pe.into();
    assert_eq!(be.exit_code(), ExitCode::Internal);
}

#[tokio::test]
async fn ac6_external_missing_kubeconfig_still_exits_cluster() {
    let p = from_cli(
        ProvisionerKind::External,
        Some(PathBuf::from("/nonexistent/kc-1234567")),
        "bench-test".into(),
    );
    let pe = p.provision().await.unwrap_err();
    let be: BenchError = pe.into();
    assert_eq!(be.exit_code(), ExitCode::Cluster);
}

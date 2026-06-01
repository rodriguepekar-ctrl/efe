//! PR #23 § 10 + § 21 — provisioner golden integration tests.
//!
//! 12 tests pin the trait surface and the 3-impl taxonomy.  Locked by
//! STABILITY_CONTRACT v1.5 § 23.6.

use std::path::PathBuf;

use efe_cluster_provisioner::{
    from_kind, ClusterProvisioner, ExternalProvisioner, K3sProvisioner, KindProvisioner,
    ProvisionError, ProvisionerKind,
};
use tempfile::NamedTempFile;

// ─────────────────────────────────────────────────────────────────────
// 1.  Discriminator round-trip.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn provisioner_kind_string_form_frozen() {
    assert_eq!(ProvisionerKind::Kind.as_str(), "kind");
    assert_eq!(ProvisionerKind::K3s.as_str(), "k3s");
    assert_eq!(ProvisionerKind::External.as_str(), "external");
}

#[test]
fn provisioner_kind_parse_roundtrips() {
    for k in [ProvisionerKind::Kind, ProvisionerKind::K3s, ProvisionerKind::External] {
        assert_eq!(ProvisionerKind::parse(k.as_str()), Some(k));
    }
}

#[test]
fn provisioner_kind_parse_rejects_unknown() {
    assert!(ProvisionerKind::parse("microk8s").is_none());
    assert!(ProvisionerKind::parse("KIND").is_none()); // case-sensitive
    assert!(ProvisionerKind::parse("").is_none());
}

// ─────────────────────────────────────────────────────────────────────
// 2.  `from_kind` builds the right impl.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn from_kind_builds_kind_impl() {
    let p = from_kind(ProvisionerKind::Kind, None, "test".into());
    assert_eq!(p.name(), "kind");
    assert_eq!(p.kind(), ProvisionerKind::Kind);
}

#[test]
fn from_kind_builds_k3s_impl() {
    let p = from_kind(ProvisionerKind::K3s, None, "test".into());
    assert_eq!(p.name(), "k3s");
}

#[test]
fn from_kind_builds_external_impl_with_kubeconfig() {
    let p = from_kind(ProvisionerKind::External, Some(PathBuf::from("/tmp/kc")), "t".into());
    assert_eq!(p.name(), "external");
}

// ─────────────────────────────────────────────────────────────────────
// 3.  ExternalProvisioner contract.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn external_provisioner_returns_existing_kubeconfig_path() {
    let f = NamedTempFile::new().unwrap();
    let p = ExternalProvisioner::new(f.path().to_path_buf());
    let kc = p.provision().await.unwrap();
    assert_eq!(kc, f.path());
}

#[tokio::test]
async fn external_provisioner_rejects_missing_kubeconfig() {
    let p = ExternalProvisioner::new(PathBuf::from("/nonexistent/kc-987654"));
    let e = p.provision().await.unwrap_err();
    match e {
        ProvisionError::Cluster { provisioner, .. } => assert_eq!(provisioner, "external"),
        other => panic!("expected ProvisionError::Cluster, got {other:?}"),
    }
}

#[tokio::test]
async fn external_provisioner_teardown_is_noop() {
    let f = NamedTempFile::new().unwrap();
    let p = ExternalProvisioner::new(f.path().to_path_buf());
    // External-provisioner teardown is always Ok.
    p.teardown().await.unwrap();
}

// ─────────────────────────────────────────────────────────────────────
// 4.  K3sProvisioner is a Phase-3 stub.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn k3s_provisioner_is_phase3_stub() {
    let p = K3sProvisioner::new("x".into());
    let e = p.provision().await.unwrap_err();
    match e {
        ProvisionError::Cluster { provisioner, .. } => assert_eq!(provisioner, "k3s"),
        other => panic!("expected ProvisionError::Cluster, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
// 5.  KindProvisioner image override + cluster_name inspection.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn kind_default_image_v1_30_plus() {
    let p = KindProvisioner::new("test".into());
    assert!(p.image().contains("v1.30"));
    assert_eq!(p.cluster_name(), "test");
}

#[test]
fn kind_with_image_override() {
    let p = KindProvisioner::new("test".into()).with_image("kindest/node:v1.29.0".into());
    assert!(p.image().contains("v1.29"));
}

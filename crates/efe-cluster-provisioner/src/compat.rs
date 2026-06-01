//! PR #23 § 2.4 + § 10 — bench/harness compatibility module.
//!
//! Phase-2 ships a thin layer that documents the historical names used
//! by PR #21 `bench/harness/src/cluster/{kind,k3s,external}.rs` for
//! reviewers performing the AC6 byte-identical audit.
//!
//! **There is no runtime code here** — the actual conversion happens
//! via `From<ProvisionError> for BenchError` in PR #21's `error.rs`.
//! This module exists to anchor the historical type-aliases + provide
//! a single place to add additional compat shims in the future
//! (Phase-3 G.2 — Phase-1 binary compat layer).
//!
//! # Migration map (PR #21 → PR #23 extraction)
//!
//! | PR #21 path | PR #23 path |
//! |---|---|
//! | `bench::harness::cluster::ClusterProvisioner` | [`crate::ClusterProvisioner`] |
//! | `bench::harness::cluster::kind::KindProvisioner` | [`crate::KindProvisioner`] |
//! | `bench::harness::cluster::k3s::K3sProvisioner` | [`crate::K3sProvisioner`] |
//! | `bench::harness::cluster::external::ExternalProvisioner` | [`crate::ExternalProvisioner`] |
//! | `bench::harness::cluster::from_cli(_, _, _)` | [`crate::from_kind(_, _, _)`] |
//! | `bench::harness::cluster::ensure_kubectl_available()` | [`crate::ensure_kubectl_available`] |
//!
//! # Frozen surface (STABILITY_CONTRACT v1.5 § 23.6)
//!
//! See [`crate::traits::ClusterProvisioner`].

/// Pinned to the v1.5 surface for forward audits.
pub const STABILITY_TIER: &str = "v1.5 § 23.6";

/// Historical alias for [`crate::ProvisionerKind`].  Kept so PR #21
/// reviewers can `cargo doc` and find the original symbol.
pub type LegacyProvisionerKind = crate::ProvisionerKind;

/// Historical alias for [`crate::ProvisionError`].
pub type LegacyClusterError = crate::ProvisionError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stability_tier_anchor_frozen() {
        // STABILITY_CONTRACT v1.5 § 23.6 — locked.
        assert_eq!(STABILITY_TIER, "v1.5 § 23.6");
    }

    #[test]
    fn legacy_aliases_compile() {
        // Compile-only assertion that the aliases resolve to the
        // current types.  Used by the AC6 reviewer checklist.
        let _ = LegacyProvisionerKind::Kind;
        let _: Option<LegacyClusterError> = None;
    }
}

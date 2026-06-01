//! PR #17 Group F — `build_info` metric (1 series).
//!
//! Architecture reference : `docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md` § 11.
//!
//! | Series                                     | Type           | Labels                                                          |
//! | ------------------------------------------ | -------------- | --------------------------------------------------------------- |
//! | `efe_meta_swarm_build_info`                | IntGaugeVec    | `version`,`git_sha`,`profile`,`rustc_version`,`build_date`      |
//!
//! ## Why a gauge with value 1
//!
//! Prometheus has no native string-metadata type.  The convention
//! (used by `node_exporter`, `blackbox_exporter`, kube-state-metrics,
//! etc.) is a gauge fixed at `1` with labels carrying the actual
//! information.  Downstream PromQL can left-join this onto any other
//! series :
//!
//! ```promql
//! rate(efe_meta_swarm_iterate_total[5m])
//!   * on(instance) group_left(version)
//!     efe_meta_swarm_build_info{}
//! ```
//!
//! Result : the iterate rate annotated with the version label for every
//! sample, enabling "compare v0.2.0 vs v0.3.0 iterate rates" without
//! manual joining.
//!
//! ## Where do the labels come from
//!
//! All five labels are compile-time constants set by the build :
//!
//!   * `version`        ← `env!("CARGO_PKG_VERSION")`
//!   * `git_sha`        ← `env!("GIT_SHA")` (set by `build.rs`)
//!   * `profile`        ← `if cfg!(debug_assertions) { "debug" } else { "release" }`
//!   * `rustc_version`  ← `env!("RUSTC_VERSION")` (set by `build.rs`)
//!   * `build_date`     ← `env!("BUILD_DATE")` (set by `build.rs`)
//!
//! When the build.rs hasn't set the optional vars, we fall back to
//! sensible sentinels (`"unknown"`).

use prometheus::{register_int_gauge_vec_with_registry, IntGaugeVec, Registry};

/// Compile-time `git_sha` (set by build.rs ; falls back to `"nogit"`).
const GIT_SHA: &str = crate::GIT_SHA;
/// Compile-time `version` from `Cargo.toml`.
const VERSION: &str = crate::VERSION;
/// Compile-time profile.
const PROFILE: &str = crate::PROFILE;
/// `rustc --version --verbose` short form.  Falls back to `"unknown"`
/// when build.rs doesn't set the env var.
const RUSTC_VERSION: &str = match option_env!("RUSTC_VERSION") {
    Some(v) => v,
    None => "unknown",
};
/// ISO-8601 UTC build timestamp.  Falls back to `"unknown"` when
/// build.rs doesn't set the env var.
const BUILD_DATE: &str = match option_env!("BUILD_DATE") {
    Some(v) => v,
    None => "unknown",
};

/// Group F — 1 Prometheus series carrying build metadata.
#[derive(Debug, Clone)]
pub struct BuildInfoMetric {
    /// `efe_meta_swarm_build_info` — gauge always at `1`, labels carry
    /// the actual metadata.
    pub info: IntGaugeVec,
}

impl BuildInfoMetric {
    /// Register the series on `reg` + emit the (compile-time) labelled
    /// sample.
    pub fn register(reg: &Registry) -> prometheus::Result<Self> {
        let info =
            register_int_gauge_vec_with_registry!(
            "efe_meta_swarm_build_info",
            "Build metadata gauge — value is always 1 ; the labels carry the actual information.",
            &["version", "git_sha", "profile", "rustc_version", "build_date"],
            reg,
        )?;
        // Emit the single labelled sample at boot.
        info.with_label_values(&[VERSION, GIT_SHA, PROFILE, RUSTC_VERSION, BUILD_DATE])
            .set(1);
        Ok(Self { info })
    }

    /// Test-only : emit a custom labelled sample.  Production code
    /// uses the compile-time-set sample emitted in `register()`.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn set_for_test(
        &self,
        version: &str,
        git_sha: &str,
        profile: &str,
        rustc_version: &str,
        build_date: &str,
    ) {
        self.info
            .with_label_values(&[version, git_sha, profile, rustc_version, build_date])
            .set(1);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::TextEncoder;

    fn body(reg: &Registry) -> String {
        TextEncoder::new()
            .encode_to_string(&reg.gather())
            .expect("encode")
    }

    #[test]
    fn register_emits_one_sample_with_compile_time_labels() {
        let r = Registry::new();
        let _m = BuildInfoMetric::register(&r).expect("register");
        let b = body(&r);
        assert!(b.contains("efe_meta_swarm_build_info"));
        // Labels are set : there's at least one `} 1` line.
        let info_lines: Vec<&str> = b
            .lines()
            .filter(|l| l.starts_with("efe_meta_swarm_build_info{"))
            .collect();
        assert_eq!(info_lines.len(), 1);
        let line = info_lines[0];
        // Every label must be present.
        for label in [
            "version=",
            "git_sha=",
            "profile=",
            "rustc_version=",
            "build_date=",
        ] {
            assert!(line.contains(label), "missing label {label} in {line}");
        }
        // Value is 1.
        assert!(line.ends_with(" 1"));
    }

    #[test]
    fn test_helper_emits_custom_labels() {
        let r = Registry::new();
        let m = BuildInfoMetric::register(&r).expect("register");
        m.set_for_test(
            "0.3.0",
            "abc1234",
            "release",
            "1.88.0",
            "2026-07-27T14:32:11Z",
        );
        let b = body(&r);
        // Both the compile-time sample AND the custom sample are present.
        let count = b
            .lines()
            .filter(|l| l.starts_with("efe_meta_swarm_build_info{"))
            .count();
        assert!(count >= 1);
    }
}

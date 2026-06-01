//! PR #24 § 17 + AD29-AD32 — gh-pages deploy + ratchet sanity check.

pub mod ratchet;

use std::path::Path;

use tracing::info;

use crate::error::{BenchError, BenchResult};

/// Verify the output directory matches the v1.6 § 24 layout BEFORE
/// the operator triggers the gh-pages push (AD32 — T123 mitigation).
///
/// Used by both `efe-bench-harness publish` and the CI workflow's
/// pre-deploy step.
pub fn pre_deploy_ratchet(output_root: &Path) -> BenchResult<DeployManifest> {
    ratchet::run(output_root)
}

/// Report of the ratchet check.
#[derive(Debug, Clone)]
pub struct DeployManifest {
    pub badge_count: usize,
    pub api_endpoint_count: usize,
    pub scenario_page_count: usize,
    pub has_index: bool,
    pub has_methodology: bool,
    pub has_chaos: bool,
    pub has_quarterly_archive: bool,
}

impl DeployManifest {
    /// Returns Ok(()) iff the manifest matches the frozen v1.6 layout.
    pub fn validate(&self) -> BenchResult<()> {
        if self.badge_count != 11 {
            return Err(BenchError::Internal(format!(
                "expected 11 badges (v1.6 § 24.1), got {}",
                self.badge_count
            )));
        }
        if self.api_endpoint_count != 5 {
            return Err(BenchError::Internal(format!(
                "expected 5 JSON API endpoints (v1.6 § 24.3), got {}",
                self.api_endpoint_count
            )));
        }
        if self.scenario_page_count != 10 {
            return Err(BenchError::Internal(format!(
                "expected 10 scenario pages (v1.6 § 24.2), got {}",
                self.scenario_page_count
            )));
        }
        for (name, present) in [
            ("index.html", self.has_index),
            ("methodology.html", self.has_methodology),
            ("chaos.html", self.has_chaos),
            ("quarterly-archive.html", self.has_quarterly_archive),
        ] {
            if !present {
                return Err(BenchError::Internal(format!(
                    "missing required page : {name}"
                )));
            }
        }
        info!(?self, "pre-deploy ratchet validated");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_with_correct_counts_validates() {
        let m = DeployManifest {
            badge_count: 11,
            api_endpoint_count: 5,
            scenario_page_count: 10,
            has_index: true,
            has_methodology: true,
            has_chaos: true,
            has_quarterly_archive: true,
        };
        m.validate().unwrap();
    }

    #[test]
    fn missing_badge_rejected() {
        let m = DeployManifest {
            badge_count: 10,
            api_endpoint_count: 5,
            scenario_page_count: 10,
            has_index: true,
            has_methodology: true,
            has_chaos: true,
            has_quarterly_archive: true,
        };
        assert!(m.validate().is_err());
    }

    #[test]
    fn missing_api_endpoint_rejected() {
        let m = DeployManifest {
            badge_count: 11,
            api_endpoint_count: 4,
            scenario_page_count: 10,
            has_index: true,
            has_methodology: true,
            has_chaos: true,
            has_quarterly_archive: true,
        };
        assert!(m.validate().is_err());
    }

    #[test]
    fn missing_scenario_page_rejected() {
        let m = DeployManifest {
            badge_count: 11,
            api_endpoint_count: 5,
            scenario_page_count: 9,
            has_index: true,
            has_methodology: true,
            has_chaos: true,
            has_quarterly_archive: true,
        };
        assert!(m.validate().is_err());
    }

    #[test]
    fn missing_index_rejected() {
        let m = DeployManifest {
            badge_count: 11,
            api_endpoint_count: 5,
            scenario_page_count: 10,
            has_index: false,
            has_methodology: true,
            has_chaos: true,
            has_quarterly_archive: true,
        };
        assert!(m.validate().is_err());
    }
}

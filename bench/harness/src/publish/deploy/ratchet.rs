//! PR #24 AD32 — ratchet sanity check.
//!
//! Walks the output directory and confirms the frozen v1.6 § 24
//! layout invariants : 11 badges + 5 JSON endpoints + 10 scenario
//! pages + 4 required top-level pages.
//!
//! **D-AS fix** — the badge + scenario-page + API-endpoint filename
//! lists are now sourced from the canonical tables :
//!   * `crate::publish::badge::BADGE_FILENAMES`           (11 entries)
//!   * `crate::publish::SCENARIO_PAGE_FILENAMES`          (10 entries)
//!   * `crate::publish::api::API_ENDPOINT_FILENAMES`      (5 entries)
//!
//! Previously a rename in any of those canonical tables without a
//! mirror update in the ratchet would have caused the ratchet to
//! silently under-count valid output (or over-count nonexistent
//! files) — defeating the whole point of the ratchet.
//!
//! **PR #25 day 4** — final D-AS cleanup : the local `REQUIRED_API`
//! const (a parallel-list duplication carried since PR #24 § 15) is
//! now removed in favour of the canonical
//! `crate::publish::api::API_ENDPOINT_FILENAMES`.  Enforced by
//! `tests/process_invariants.rs::i_2_no_parallel_const_lists_in_workspace`.

use std::path::Path;

use crate::error::BenchResult;
use crate::publish::api::API_ENDPOINT_FILENAMES;
use crate::publish::badge::BADGE_FILENAMES;
use crate::publish::deploy::DeployManifest;
use crate::publish::SCENARIO_PAGE_FILENAMES;

/// Inspect `output_root` and return a [`DeployManifest`].
pub fn run(output_root: &Path) -> BenchResult<DeployManifest> {
    let badges_dir = output_root.join("badges");
    let api_dir = output_root.join("api");
    let scenarios_dir = output_root.join("scenarios");

    // Three canonical SOT iterations — no parallel-list duplication.
    let badge_count = BADGE_FILENAMES
        .iter()
        .filter(|f| badges_dir.join(f).exists())
        .count();
    let api_endpoint_count = API_ENDPOINT_FILENAMES
        .iter()
        .filter(|f| api_dir.join(f).exists())
        .count();
    let scenario_page_count = SCENARIO_PAGE_FILENAMES
        .iter()
        .filter(|(_, name)| scenarios_dir.join(name).exists())
        .count();

    let m = DeployManifest {
        badge_count,
        api_endpoint_count,
        scenario_page_count,
        has_index: output_root.join("index.html").exists(),
        has_methodology: output_root.join("methodology.html").exists(),
        has_chaos: output_root.join("chaos.html").exists(),
        has_quarterly_archive: output_root.join("quarterly-archive.html").exists(),
    };
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, "").unwrap();
    }

    #[test]
    fn empty_dir_returns_zeros() {
        let tmp = tempfile::tempdir().unwrap();
        let m = run(tmp.path()).unwrap();
        assert_eq!(m.badge_count, 0);
        assert_eq!(m.api_endpoint_count, 0);
        assert_eq!(m.scenario_page_count, 0);
        assert!(!m.has_index);
    }

    #[test]
    fn fully_populated_returns_full_counts() {
        let tmp = tempfile::tempdir().unwrap();
        for f in BADGE_FILENAMES {
            touch(&tmp.path().join("badges").join(f));
        }
        for f in API_ENDPOINT_FILENAMES {
            touch(&tmp.path().join("api").join(f));
        }
        for (_, f) in SCENARIO_PAGE_FILENAMES {
            touch(&tmp.path().join("scenarios").join(f));
        }
        for f in ["index.html", "methodology.html", "chaos.html", "quarterly-archive.html"] {
            touch(&tmp.path().join(f));
        }
        let m = run(tmp.path()).unwrap();
        assert_eq!(m.badge_count, 11);
        assert_eq!(m.api_endpoint_count, 5);
        assert_eq!(m.scenario_page_count, 10);
        m.validate().unwrap();
    }

    #[test]
    fn frozen_taxonomies_have_correct_size() {
        assert_eq!(BADGE_FILENAMES.len(), 11);
        assert_eq!(API_ENDPOINT_FILENAMES.len(), 5);
        assert_eq!(SCENARIO_PAGE_FILENAMES.len(), 10);
    }

    /// **D-AS** (final cleanup, PR #25 day 4) — the ratchet's API
    /// endpoint taxonomy MUST mirror the JSON files written by
    /// `Publisher::write_api_with_agg`.  A drift would cause the
    /// ratchet to undercount or over-require.
    ///
    /// Phase-2 read this from a local `REQUIRED_API` const ;
    /// Phase-3 PR #25 hoists the canonical to
    /// `crate::publish::api::API_ENDPOINT_FILENAMES`.
    #[test]
    fn d_as_api_endpoint_filenames_mirrors_writer_outputs() {
        // The five JSON files written by write_api_with_agg :
        //   current.json, history.json, scenarios.json, baselines.json, version.json
        // — locked here as the structural reference for the ratchet.
        let expected = [
            "current.json",
            "history.json",
            "scenarios.json",
            "baselines.json",
            "version.json",
        ];
        let actual: std::collections::HashSet<_> =
            API_ENDPOINT_FILENAMES.iter().copied().collect();
        let want: std::collections::HashSet<_> = expected.iter().copied().collect();
        assert_eq!(
            actual, want,
            "D-AS : API_ENDPOINT_FILENAMES drifted from the writer output set"
        );
    }
}

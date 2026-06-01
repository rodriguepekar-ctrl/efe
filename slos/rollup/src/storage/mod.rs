//! PR #22 § 10 — storage layer.
//!
//! 3 backends :
//!   * [`parquet`] — daily (Phase-2 ships a JSON-envelope ; Phase-3
//!     G.9 swaps to true Parquet without API change).
//!   * [`json`] — monthly canonical-form.
//!   * [`pdf`] — quarterly signed PDF + JSON sidecar.
//!
//! Plus :
//!   * [`object_store`] — file:// / s3:// / gs:// abstraction.
//!   * [`retention`] — daily 90d / monthly 7y / quarterly forever.

pub mod json;
pub mod object_store;
pub mod parquet;
pub mod pdf;
pub mod retention;

use std::path::{Path, PathBuf};

/// Canonical path for a daily file.
///
/// `<root>/v{chart}/<yyyy-mm>/daily/<yyyy-mm-dd>.parquet`
#[must_use]
pub fn daily_path(root: &Path, chart_version: &str, yyyy_mm_dd: &str) -> PathBuf {
    let yyyy_mm = &yyyy_mm_dd[..7];
    root.join(format!("v{chart_version}"))
        .join(yyyy_mm)
        .join("daily")
        .join(format!("{yyyy_mm_dd}.parquet"))
}

/// Canonical path for a monthly file.
#[must_use]
pub fn monthly_path(root: &Path, chart_version: &str, yyyy_mm: &str) -> PathBuf {
    root.join(format!("v{chart_version}"))
        .join("monthly")
        .join(format!("{yyyy_mm}.json"))
}

/// Canonical path for a quarterly file.
///
/// `<root>/v{chart}/quarterly/Q{1..4}-{yyyy}.{json,pdf}`
#[must_use]
pub fn quarterly_path(root: &Path, chart_version: &str, quarter: &str, ext: &str) -> PathBuf {
    root.join(format!("v{chart_version}"))
        .join("quarterly")
        .join(format!("{quarter}.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_path_layout_is_v1_4() {
        // PR #22 § 10.1 AD25.
        let p = daily_path(Path::new("/x"), "0.6.0", "2026-05-15");
        assert_eq!(p, PathBuf::from("/x/v0.6.0/2026-05/daily/2026-05-15.parquet"));
    }

    #[test]
    fn monthly_path_layout() {
        let p = monthly_path(Path::new("/x"), "0.6.0", "2026-05");
        assert_eq!(p, PathBuf::from("/x/v0.6.0/monthly/2026-05.json"));
    }

    #[test]
    fn quarterly_path_layout() {
        let pdf = quarterly_path(Path::new("/x"), "0.6.0", "Q2-2026", "pdf");
        assert_eq!(pdf, PathBuf::from("/x/v0.6.0/quarterly/Q2-2026.pdf"));
        let json = quarterly_path(Path::new("/x"), "0.6.0", "Q2-2026", "json");
        assert_eq!(json, PathBuf::from("/x/v0.6.0/quarterly/Q2-2026.json"));
    }
}

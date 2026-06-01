//! PR #23 § 14 — build metadata exposed via `--version`.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_SHA: &str = match option_env!("EFE_GIT_SHA") {
    Some(s) => s,
    None => "unknown",
};
pub const RUSTC_VERSION: &str = match option_env!("EFE_RUSTC_VERSION") {
    Some(s) => s,
    None => "unknown",
};
pub const BUILD_DATE: &str = match option_env!("EFE_BUILD_DATE") {
    Some(s) => s,
    None => "unknown",
};

#[must_use]
pub fn long() -> String {
    format!(
        "efe-smoke {VERSION}\n  git_sha       : {GIT_SHA}\n  rustc_version : {RUSTC_VERSION}\n  build_date    : {BUILD_DATE}\n  stability     : v1.5 (PR #23 § 23)"
    )
}

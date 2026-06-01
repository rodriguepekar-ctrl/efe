//! Build script for `efe-meta-swarm-server`.
//!
//! Emits one env var :
//!
//!     GIT_SHA — short (8-char) SHA of the current commit, or
//!               "nogit" if not in a git repo.
//!
//! Used at compile time by [`crate::GIT_SHA`] (via `env!("GIT_SHA")`)
//! to populate `InfoResponse.build_sha`.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");

    let sha = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "nogit".to_string());

    // Sanitize : alphanumeric only, length ≤ 16.  Defends against a
    // hostile `git` binary returning trash on PATH (sec-team req).
    let safe_sha = if sha.len() <= 16 && sha.chars().all(|c| c.is_ascii_alphanumeric()) {
        sha
    } else {
        "invalid-sha".to_string()
    };

    println!("cargo:rustc-env=GIT_SHA={safe_sha}");
}

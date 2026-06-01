//! PR #24 § 11 — mdbook orchestration.
//!
//! Shells out to `mdbook build` when the binary is installed (CI path) ;
//! otherwise produces the structured `book/` directory directly so
//! operators can preview locally without mdbook.

use std::path::Path;
use std::process::Command;

use tracing::{info, warn};

use crate::error::{BenchError, BenchResult};

/// Hard cap for `mdbook build` so a hung mdbook can't freeze the
/// whole publish pipeline.  Generous for a 10-page bench book ; the
/// real worst-case observed on CI is ~3 seconds.
pub const MDBOOK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run `mdbook build` on `source` directory and put the output at
/// `--dest-dir` (which we set to `output` so files land directly).
/// Returns `Ok(false)` when mdbook is not installed or fails.
///
/// **D-AY fix** — wraps the child invocation in a watchdog thread
/// that kills mdbook after [`MDBOOK_TIMEOUT`].  The original
/// implementation called `.output()` which blocks forever ; a hung
/// mdbook (rare, but possible if a network-fetched theme stalls)
/// would have frozen the publish pipeline indefinitely.  CI's
/// `timeout-minutes: 30` was the only safeguard.
pub fn build_mdbook(source: &Path, output: &Path) -> BenchResult<bool> {
    use std::sync::mpsc;
    let child = Command::new("mdbook")
        .arg("build")
        .arg(source)
        .arg("--dest-dir")
        .arg(output)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "mdbook not installed — skipping HTML generation");
            return Ok(false);
        }
    };
    // Watchdog : sleep MDBOOK_TIMEOUT then kill if still alive.
    let (tx, rx) = mpsc::channel::<()>();
    let timeout = MDBOOK_TIMEOUT;
    std::thread::spawn(move || {
        if rx.recv_timeout(timeout).is_err() {
            // Timeout fired — the child is still running.  Caller
            // signals "done" via tx.send() so we don't kill late.
            // No reliable cross-thread access to `child` here — see
            // the kill loop below using try_wait() polling instead.
        }
    });
    // Poll the child completion ourselves with a deadline.
    let start = std::time::Instant::now();
    let result;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                result = Some(status);
                break;
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    // Hard kill.
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = tx.send(()); // tell watchdog to stop
                    warn!(
                        timeout_secs = timeout.as_secs(),
                        "mdbook build timed out — killed (D-AY)"
                    );
                    return Ok(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                let _ = tx.send(());
                warn!(error = %e, "mdbook wait failed");
                return Ok(false);
            }
        }
    }
    let _ = tx.send(()); // stop watchdog
    let status = result.expect("loop exit implies status set");
    // Collect output.
    let mut stderr = Vec::new();
    if let Some(mut e) = child.stderr.take() {
        use std::io::Read;
        let _ = e.read_to_end(&mut stderr);
    }
    if status.success() {
        info!(source = %source.display(), output = %output.display(), "mdbook build ok");
        Ok(true)
    } else {
        warn!(
            stderr = %String::from_utf8_lossy(&stderr),
            "mdbook build failed (best-effort)"
        );
        Ok(false)
    }
}

/// Sentinel marker placed in the mdbook source markdown (overview.md)
/// that this function substitutes after `mdbook build` runs.  Picked
/// for : (a) idempotency (mdbook regenerates from source each run, so
/// the sentinel always re-appears), (b) HTML legality (a comment node
/// is invisible if not substituted), (c) grep-ability for operators.
pub const SMOKE_BADGE_SENTINEL: &str = "<!-- EFE_SMOKE_BADGE_SLOT -->";

/// **D-N fix** — substitute the PR #23 smoke-badge sentinel inside
/// mdbook-produced HTML files.  Without this post-process, the
/// `--embed-smoke` flag is a no-op for the mdbook code path (the
/// placeholder code path is handled separately by D-J).
///
/// `embed_smoke = true`  → sentinel replaced with the smoke fragment.
/// `embed_smoke = false` → sentinel replaced with an empty string
///                         (no visual residue, no broken layout).
///
/// Safe on absent files (no-op).  Idempotent : second call finds no
/// sentinel and is a no-op.
pub fn post_process_smoke_slot(output: &Path, embed_smoke: bool) -> BenchResult<()> {
    let index = output.join("index.html");
    if !index.exists() {
        return Ok(()); // placeholder path will have its own injection
    }
    let body = std::fs::read_to_string(&index).map_err(|source| BenchError::Result {
        path: index.clone(),
        source,
    })?;
    if !body.contains(SMOKE_BADGE_SENTINEL) {
        return Ok(()); // no sentinel = nothing to do (idempotent on 2nd run)
    }
    let replacement = if embed_smoke {
        crate::publish::crosslink::smoke::render_header_html()
    } else {
        String::new()
    };
    let patched = body.replace(SMOKE_BADGE_SENTINEL, &replacement);
    std::fs::write(&index, patched).map_err(|source| BenchError::Result {
        path: index,
        source,
    })
}

/// Convenience : check whether mdbook is on PATH (without actually
/// invoking a build).
#[must_use]
pub fn mdbook_available() -> bool {
    Command::new("mdbook")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Verify that a `book/` output directory contains the required
/// per-scenario pages (10) + overview + methodology + chaos +
/// quarterly-archive.  Used by the publisher's ratchet sanity check.
pub fn verify_layout(book_root: &Path) -> BenchResult<()> {
    let required = [
        "index.html",
        "methodology.html",
        "chaos.html",
        "quarterly-archive.html",
        "scenarios/s01-baseline.html",
        "scenarios/s02-canary.html",
        "scenarios/s03-ramp.html",
        "scenarios/s04-chaos-participant-loss.html",
        "scenarios/s05-chaos-network-partition.html",
        "scenarios/s06-chaos-cert-expiry.html",
        "scenarios/s07-chaos-reload-storm.html",
        "scenarios/s08-multi-region.html",
        "scenarios/s09-soak.html",
        "scenarios/s10-slo-validation.html",
    ];
    for rel in required {
        let path = book_root.join(rel);
        if !path.exists() {
            return Err(BenchError::Result {
                path: path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("required page missing : {rel}"),
                ),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_layout_returns_err_on_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let e = verify_layout(tmp.path()).unwrap_err();
        match e {
            BenchError::Result { .. } => {}
            other => panic!("expected Result error, got {other:?}"),
        }
    }

    #[test]
    fn d_n_smoke_slot_replaced_when_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let index = tmp.path().join("index.html");
        let body =
            format!("<html><body><h1>EFE</h1>{SMOKE_BADGE_SENTINEL} more</body></html>");
        std::fs::write(&index, body).unwrap();
        post_process_smoke_slot(tmp.path(), true).unwrap();
        let patched = std::fs::read_to_string(&index).unwrap();
        assert!(
            patched.contains("smoke/badge.svg"),
            "D-N : smoke fragment must be injected when embed_smoke=true"
        );
        assert!(
            !patched.contains(SMOKE_BADGE_SENTINEL),
            "D-N : sentinel must be consumed"
        );
    }

    #[test]
    fn d_n_smoke_slot_blanked_when_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let index = tmp.path().join("index.html");
        let body =
            format!("<html><body><h1>EFE</h1>{SMOKE_BADGE_SENTINEL} more</body></html>");
        std::fs::write(&index, body).unwrap();
        post_process_smoke_slot(tmp.path(), false).unwrap();
        let patched = std::fs::read_to_string(&index).unwrap();
        assert!(
            !patched.contains("smoke/badge.svg"),
            "D-N : no smoke fragment when embed_smoke=false"
        );
        assert!(
            !patched.contains(SMOKE_BADGE_SENTINEL),
            "D-N : sentinel must still be consumed (with empty replacement)"
        );
    }

    #[test]
    fn d_n_smoke_slot_idempotent_second_call_noop() {
        // Second call finds no sentinel → must not double-inject or
        // panic.  Critical for re-running publish multiple times.
        let tmp = tempfile::tempdir().unwrap();
        let index = tmp.path().join("index.html");
        let body =
            format!("<html><body>{SMOKE_BADGE_SENTINEL}</body></html>");
        std::fs::write(&index, body).unwrap();
        post_process_smoke_slot(tmp.path(), true).unwrap();
        let after_first = std::fs::read_to_string(&index).unwrap();
        post_process_smoke_slot(tmp.path(), true).unwrap();
        let after_second = std::fs::read_to_string(&index).unwrap();
        assert_eq!(after_first, after_second, "D-N : 2nd call must be a no-op");
    }

    #[test]
    fn d_n_smoke_slot_handles_missing_index_gracefully() {
        let tmp = tempfile::tempdir().unwrap();
        // index.html doesn't exist — must not error.
        post_process_smoke_slot(tmp.path(), true).expect("must not error on missing index");
    }

    #[test]
    fn verify_layout_passes_when_all_present() {
        let tmp = tempfile::tempdir().unwrap();
        for rel in [
            "index.html",
            "methodology.html",
            "chaos.html",
            "quarterly-archive.html",
        ] {
            std::fs::write(tmp.path().join(rel), "<html></html>").unwrap();
        }
        std::fs::create_dir_all(tmp.path().join("scenarios")).unwrap();
        for rel in [
            "s01-baseline.html",
            "s02-canary.html",
            "s03-ramp.html",
            "s04-chaos-participant-loss.html",
            "s05-chaos-network-partition.html",
            "s06-chaos-cert-expiry.html",
            "s07-chaos-reload-storm.html",
            "s08-multi-region.html",
            "s09-soak.html",
            "s10-slo-validation.html",
        ] {
            std::fs::write(tmp.path().join("scenarios").join(rel), "<html></html>").unwrap();
        }
        verify_layout(tmp.path()).unwrap();
    }

    #[test]
    fn mdbook_available_is_callable() {
        // Either true or false — both are valid (CI may not have mdbook).
        let _ = mdbook_available();
    }
}

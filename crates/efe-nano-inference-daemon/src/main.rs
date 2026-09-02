//! **Sprint 8 — N11 days 13-15** : guest-side inference daemon.
//!
//! Runs inside a Firecracker microVM.  Listens for vsock connections
//! on port 5000, dispatches `Infer` / `Health` / `Shutdown` RPCs.
//!
//! # Boot sequence
//!
//! 1. Parse `/proc/cmdline` for `efe.model_id=X efe.model_version=Y`
//!    and `efe.daemon_port=N`.
//! 2. Verify SHA-256 of `/opt/models/{X}/{Y}/model.onnx` against the
//!    expected hash baked into `/etc/efe/model.sha256` at build time.
//! 3. Load the model into memory (Sprint 8 days 13-14 : stub ; Sprint 9
//!    wires real ONNX runtime).
//! 4. Listen on `AF_VSOCK` (port from cmdline, default 5000).
//! 5. For each accepted connection : read length-prefixed frame, parse
//!    JSON, dispatch, write response, close.
//!
//! # Why one connection per RPC
//!
//! Simplifies guest state (no per-connection session).  Vsock setup
//! cost is ~ 100 µs which is negligible vs the inference cost
//! (10-150 ms typical).

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};

mod handler;
mod model;
mod transport;

use handler::Handler;
use model::ModelLoader;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    init_tracing();

    let config = BootConfig::from_cmdline().context("parse boot config from /proc/cmdline")?;
    tracing::info!(
        model_id = %config.model_id,
        model_version = %config.model_version,
        daemon_port = config.daemon_port,
        "guest inference daemon starting"
    );

    let started = Instant::now();
    let bundle = ModelLoader::load(&config.model_id, &config.model_version)
        .context("load model bundle")?;
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        memory_mb = bundle.estimated_memory_mb(),
        "model loaded"
    );

    let handler = Arc::new(Handler::new(
        config.model_id.clone(),
        config.model_version.clone(),
        bundle,
        Instant::now(),
    ));

    transport::serve(handler, config.daemon_port).await
}

#[derive(Debug, Clone)]
pub struct BootConfig {
    pub model_id: String,
    pub model_version: String,
    pub daemon_port: u32,
}

impl BootConfig {
    /// Parse a kernel command line for `efe.*` keys, falling back to the supplied
    /// environment lookup (`EFE_MODEL_ID`, `EFE_MODEL_VERSION`, `EFE_DAEMON_PORT`).
    ///
    /// Both sources are **passed in**. They used to be read here — `/proc/cmdline`
    /// straight off the filesystem, the rest straight off `std::env` — and that cost
    /// three things at once, all of them measured rather than suspected:
    ///
    /// 1. **The tests raced each other.** `std::env::set_var` mutates state owned by
    ///    the whole *process*, and cargo runs a binary's tests on parallel threads:
    ///    three cases here set and cleared the same three variables. Replaying the
    ///    same binary: 15/20 green in parallel, 20/20 with `--test-threads=1`. The
    ///    failure was `left: 9999` where 5000 was expected — one case reading what
    ///    its neighbour had just written.
    /// 2. **They depended on the host.** Their own comments admitted it ("on macOS
    ///    /proc/cmdline doesn't exist", "assuming ... on this dev machine"): the same
    ///    suite did not measure the same thing under Linux.
    /// 3. **The half that matters was never tested.** Command-line parsing is the
    ///    *priority* source — the one a real boot uses — and no test could reach it,
    ///    because no test can conjure `/proc/cmdline`.
    ///
    /// Taking the two sources as parameters removes all three, and removes them
    /// *structurally*: there is no longer any shared state left to share.
    pub fn depuis(cmdline: &str, var: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let mut model_id = None;
        let mut model_version = None;
        let mut daemon_port = None;
        for token in cmdline.split_whitespace() {
            if let Some(rest) = token.strip_prefix("efe.model_id=") {
                model_id = Some(rest.to_string());
            } else if let Some(rest) = token.strip_prefix("efe.model_version=") {
                model_version = Some(rest.to_string());
            } else if let Some(rest) = token.strip_prefix("efe.daemon_port=") {
                daemon_port = rest.parse().ok();
            }
        }
        let model_id = model_id
            .or_else(|| var("EFE_MODEL_ID"))
            .ok_or_else(|| anyhow!("efe.model_id not found in /proc/cmdline or EFE_MODEL_ID env"))?;
        let model_version = model_version
            .or_else(|| var("EFE_MODEL_VERSION"))
            .ok_or_else(|| {
                anyhow!("efe.model_version not found in /proc/cmdline or EFE_MODEL_VERSION env")
            })?;
        let daemon_port = daemon_port
            .or_else(|| var("EFE_DAEMON_PORT").and_then(|s| s.parse().ok()))
            .unwrap_or(5000);
        Ok(Self {
            model_id,
            model_version,
            daemon_port,
        })
    }

    /// The boot config of *this* process: the real `/proc/cmdline`, the real environment.
    ///
    /// The only place in the crate that touches either. An unreadable `/proc/cmdline` is
    /// an empty command line, not an error — that is what makes the env fallback work on
    /// a host that has no such file, which is how the daemon is developed.
    pub fn from_cmdline() -> Result<Self> {
        Self::depuis(
            &std::fs::read_to_string("/proc/cmdline").unwrap_or_default(),
            |n| std::env::var(n).ok(),
        )
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(false)
        .compact()
        .init();
}

// ─────────────────────────────────────────────────────────────────────
//  Helpers : path conventions
// ─────────────────────────────────────────────────────────────────────

/// Path to a model bundle inside the guest rootfs.
pub fn model_dir(model_id: &str, model_version: &str) -> PathBuf {
    PathBuf::from("/opt/models")
        .join(model_id)
        .join(model_version)
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// An environment that exists only inside one test case.
    ///
    /// This is the whole repair. The cases used to call `std::env::set_var`, which mutates
    /// state owned by the *process*; cargo runs a binary's tests on parallel threads, so
    /// they reached each other. Here nothing is shared, so nothing can be raced — the
    /// property holds by construction rather than by a rule someone has to remember.
    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_string())
    }

    /// Nothing on the kernel command line: the environment answers.
    #[test]
    fn boot_config_from_env_falls_back_when_cmdline_absent() {
        let cfg = BootConfig::depuis(
            "",
            env(&[
                ("EFE_MODEL_ID", "test_model"),
                ("EFE_MODEL_VERSION", "1.2.3"),
                ("EFE_DAEMON_PORT", "9999"),
            ]),
        )
        .expect("env alone is enough");
        assert_eq!(cfg.model_id, "test_model");
        assert_eq!(cfg.model_version, "1.2.3");
        assert_eq!(cfg.daemon_port, 9999);
    }

    /// The case `r3-check.sh` caught: a model, no port, so the default stands.
    #[test]
    fn boot_config_defaults_port_to_5000() {
        let cfg = BootConfig::depuis(
            "",
            env(&[("EFE_MODEL_ID", "foo"), ("EFE_MODEL_VERSION", "0.0.1")]),
        )
        .expect("a model with no port is legal");
        assert_eq!(cfg.daemon_port, 5000);
    }

    /// No model anywhere is an **error**, and this now asserts it.
    ///
    /// The previous version read `if res.is_err() { assert!(...) }` — so in the world where
    /// the config did *not* error, which is precisely the failure this name describes, the
    /// body was skipped and the test passed. It was green either way. It could not fall,
    /// because it could not know what `/proc/cmdline` held; now it supplies it.
    #[test]
    fn boot_config_errors_when_no_model_info() {
        let err = BootConfig::depuis("", env(&[]))
            .expect_err("no model id, from either source, must not boot")
            .to_string();
        assert!(err.contains("model_id"), "the missing key is named: {err}");
    }

    /// All three keys read straight off a kernel command line.
    ///
    /// This branch is the **priority** source — the one a real boot uses — and no test
    /// reached it before, because no test can conjure `/proc/cmdline`. Other tokens on the
    /// line are ignored, as a real one is full of them.
    #[test]
    fn all_three_keys_are_read_from_the_command_line() {
        let cfg = BootConfig::depuis(
            "console=ttyS0 efe.model_id=ner_clinical efe.model_version=2.0.1 \
             efe.daemon_port=7000 quiet",
            env(&[]),
        )
        .expect("the command line alone is enough");
        assert_eq!(cfg.model_id, "ner_clinical");
        assert_eq!(cfg.model_version, "2.0.1");
        assert_eq!(cfg.daemon_port, 7000);
    }

    /// The command line **wins**. The order was written but never exercised, and an
    /// inverted precedence would let a stale environment override what the operator
    /// actually booted with.
    #[test]
    fn the_command_line_wins_over_the_environment() {
        let cfg = BootConfig::depuis(
            "efe.model_id=from_cmdline efe.daemon_port=1234",
            env(&[
                ("EFE_MODEL_ID", "from_env"),
                ("EFE_MODEL_VERSION", "9.9.9"),
                ("EFE_DAEMON_PORT", "4321"),
            ]),
        )
        .expect("both sources present");
        assert_eq!(cfg.model_id, "from_cmdline");
        assert_eq!(cfg.daemon_port, 1234);
        // …and the key the command line did NOT carry still comes from the environment.
        assert_eq!(cfg.model_version, "9.9.9");
    }

    /// **A pinned defect, not an endorsed behaviour.** `efe.daemon_port=abc` does not
    /// refuse the boot: `parse().ok()` drops it and the next source answers — here the
    /// default. A typo on a kernel command line therefore binds 5000 while the operator
    /// believes they asked for something else.
    ///
    /// This is the silent substitution that `M0` exists to refuse, and it lives in another
    /// module's boot path; the test names it so that fixing it is a decision someone takes
    /// rather than a discovery someone makes.
    #[test]
    fn an_unparsable_port_on_the_command_line_is_silently_ignored() {
        let cfg = BootConfig::depuis("efe.model_id=x efe.model_version=1 efe.daemon_port=abc", env(&[]))
            .expect("the bad port does not stop the boot — that is the point");
        assert_eq!(cfg.daemon_port, 5000, "it falls through, without a word");
    }

    /// **The ratchet.** The race is gone; this is what stops it coming back. One
    /// `std::env::set_var` in this module and the neighbouring cases can reach it again —
    /// intermittently, which is the expensive way to find out.
    #[test]
    fn no_case_in_this_module_touches_process_wide_state() {
        let src = include_str!("main.rs");
        let module = &src[src.find("mod tests {").expect("the test module")..];
        // Comment lines are dropped: the property is about what the cases DO, and naming
        // the forbidden call in prose — as the helper above does, deliberately — is not a
        // race. Note the contrast with the `P3` guard, which had to keep comments because
        // the citations it hunted *were* comments. Same shape, opposite rule; copying one
        // into the other is how a guard ends up measuring the wrong thing.
        let code: String = module
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // Built by parts so that this guard does not match itself.
        for interdit in [concat!("set_", "var"), concat!("remove_", "var")] {
            assert!(
                !code.contains(interdit),
                "`{interdit}` is back in the test module: it mutates state owned by the whole \
                 process, and cargo runs these cases on parallel threads. Measured when it was \
                 last here: 15/20 green in parallel, 20/20 single-threaded. Pass the value to \
                 `BootConfig::depuis` instead."
            );
        }
    }

    #[test]
    fn model_dir_layout() {
        let p = model_dir("ner_clinical", "1.0.0");
        assert_eq!(p.to_str().unwrap(), "/opt/models/ner_clinical/1.0.0");
    }
}

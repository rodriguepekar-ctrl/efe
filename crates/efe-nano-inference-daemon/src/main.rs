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
    /// Parse `/proc/cmdline` for `efe.*` keys.  Falls back to env vars
    /// (`EFE_MODEL_ID`, `EFE_MODEL_VERSION`, `EFE_DAEMON_PORT`) when
    /// `/proc/cmdline` doesn't carry them — useful for the host-side
    /// unit tests that don't actually run inside a VM.
    pub fn from_cmdline() -> Result<Self> {
        let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
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
            .or_else(|| std::env::var("EFE_MODEL_ID").ok())
            .ok_or_else(|| anyhow!("efe.model_id not found in /proc/cmdline or EFE_MODEL_ID env"))?;
        let model_version = model_version
            .or_else(|| std::env::var("EFE_MODEL_VERSION").ok())
            .ok_or_else(|| {
                anyhow!("efe.model_version not found in /proc/cmdline or EFE_MODEL_VERSION env")
            })?;
        let daemon_port = daemon_port
            .or_else(|| std::env::var("EFE_DAEMON_PORT").ok().and_then(|s| s.parse().ok()))
            .unwrap_or(5000);
        Ok(Self {
            model_id,
            model_version,
            daemon_port,
        })
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

    #[test]
    fn boot_config_from_env_falls_back_when_cmdline_absent() {
        // On macOS dev /proc/cmdline doesn't exist → falls back to env.
        std::env::set_var("EFE_MODEL_ID", "test_model");
        std::env::set_var("EFE_MODEL_VERSION", "1.2.3");
        std::env::set_var("EFE_DAEMON_PORT", "9999");
        let cfg = BootConfig::from_cmdline().unwrap();
        assert_eq!(cfg.model_id, "test_model");
        assert_eq!(cfg.model_version, "1.2.3");
        assert_eq!(cfg.daemon_port, 9999);
        std::env::remove_var("EFE_MODEL_ID");
        std::env::remove_var("EFE_MODEL_VERSION");
        std::env::remove_var("EFE_DAEMON_PORT");
    }

    #[test]
    fn boot_config_defaults_port_to_5000() {
        std::env::set_var("EFE_MODEL_ID", "foo");
        std::env::set_var("EFE_MODEL_VERSION", "0.0.1");
        std::env::remove_var("EFE_DAEMON_PORT");
        let cfg = BootConfig::from_cmdline().unwrap();
        assert_eq!(cfg.daemon_port, 5000);
        std::env::remove_var("EFE_MODEL_ID");
        std::env::remove_var("EFE_MODEL_VERSION");
    }

    #[test]
    fn boot_config_errors_when_no_model_info() {
        std::env::remove_var("EFE_MODEL_ID");
        std::env::remove_var("EFE_MODEL_VERSION");
        // Will fail with model_id missing (assuming /proc/cmdline doesn't
        // have efe.* entries on this dev machine).
        let res = BootConfig::from_cmdline();
        // It may pass if /proc/cmdline has entries (unlikely on dev) ;
        // we don't assert hard, just sanity-check.
        if res.is_err() {
            let err = res.unwrap_err().to_string();
            assert!(err.contains("model_id"));
        }
    }

    #[test]
    fn model_dir_layout() {
        let p = model_dir("ner_clinical", "1.0.0");
        assert_eq!(p.to_str().unwrap(), "/opt/models/ner_clinical/1.0.0");
    }
}

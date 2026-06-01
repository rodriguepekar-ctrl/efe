//! `tracing-subscriber` wiring.
//!
//! JSON output by default ; pretty output when stderr is a TTY (dev
//! ergonomics).  Filter via `EFE_META_SWARM_LOG` env var (defaults
//! to `info`).
//!
//! PR #6 ships subscriber-only.  PR #18 adds the `OTel` exporter
//! that ships spans to the collector.

use std::io::IsTerminal;

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialise the global tracing subscriber.  Idempotent : calling
/// twice is a no-op (the second call's `try_init` returns `Err`).
pub fn init() {
    let filter =
        EnvFilter::try_from_env("EFE_META_SWARM_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let use_json = !std::io::stderr().is_terminal();

    if use_json {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json())
            .try_init();
    } else {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().pretty())
            .try_init();
    }
}

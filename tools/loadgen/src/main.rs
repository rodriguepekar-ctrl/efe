//! `efe-meta-swarm-loadgen` binary entry point.
//!
//! Parses CLI flags + optional profile YAML, builds the
//! [`LoadgenConfig`](efe_meta_swarm_loadgen::LoadgenConfig), runs the
//! engine, and exits with the appropriate
//! [`ExitCode`](efe_meta_swarm_loadgen::ExitCode) per architecture
//! § AD17.

use std::process::ExitCode as StdExitCode;

use efe_meta_swarm_loadgen::cli::{Cli, Command};
use efe_meta_swarm_loadgen::{engine, ExitCode, LoadgenError};
use tracing::error;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> StdExitCode {
    // ── 1. Parse CLI ──────────────────────────────────────────────
    let cli = match Cli::parse_args() {
        Ok(cli) => cli,
        Err(err) => {
            // Clap-driven errors print to stderr already ; we just exit.
            let code = err.exit_code();
            std::process::exit(code.as_i32());
        }
    };

    // ── 2. Initialise tracing (logging) ───────────────────────────
    init_logging(&cli);

    // ── 3. Dispatch by subcommand ─────────────────────────────────
    let result = match cli.command {
        Command::Run(args) => engine::dispatch_run(args).await,
        Command::Validate(args) => engine::dispatch_validate(args).await,
        Command::Replay(args) => engine::dispatch_replay(args).await,
        Command::Aggregate(args) => engine::dispatch_aggregate(args).await,
        Command::Compare(args) => engine::dispatch_compare(args).await,
        Command::Version => {
            println!("{}", efe_meta_swarm_loadgen::build::long());
            return StdExitCode::SUCCESS;
        }
    };

    // ── 4. Map result → exit code (AD17) ─────────────────────────
    let exit = match result {
        Ok(ec) => ec,
        Err(err) => {
            error!(error = %err, "loadgen aborted");
            err.exit_code()
        }
    };

    StdExitCode::from(u8::try_from(exit.as_i32()).unwrap_or(5))
}

/// Initialise `tracing_subscriber` from the CLI knobs.
///
/// Honors `--log-format` + `--log-level` + `RUST_LOG` env var (env var
/// wins when explicitly set, otherwise the CLI flag).
fn init_logging(cli: &Cli) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cli.log_level.as_str()));

    let registry = tracing_subscriber::registry().with(filter);

    match cli.log_format {
        efe_meta_swarm_loadgen::cli::LogFormat::Text => {
            registry.with(fmt::layer().with_target(true).compact()).init();
        }
        efe_meta_swarm_loadgen::cli::LogFormat::Json => {
            registry
                .with(fmt::layer().json().with_target(true).with_current_span(true))
                .init();
        }
    }
}

/// Helper to translate a [`LoadgenError`] to the process exit code.
/// Exposed so tests can assert on it without going through `main`.
#[must_use]
#[allow(dead_code)]
fn map_exit(result: Result<ExitCode, LoadgenError>) -> i32 {
    match result {
        Ok(ec) => ec.as_i32(),
        Err(e) => e.exit_code().as_i32(),
    }
}

//! `efe-bench-harness` binary entry.
//!
//! Parses CLI flags, dispatches to the engine, exits with the
//! appropriate [`ExitCode`](efe_bench_harness::ExitCode) per AD17/AC16.

use std::process::ExitCode as StdExitCode;

use efe_bench_harness::cli::{Cli, Command};
use efe_bench_harness::{engine, BenchError, ExitCode};
use tracing::error;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> StdExitCode {
    let cli = match Cli::parse_args() {
        Ok(c) => c,
        Err(err) => {
            let code = err.exit_code();
            std::process::exit(code.as_i32());
        }
    };
    init_logging(&cli);

    let result = match cli.command {
        Command::Run(args) => engine::dispatch_run(args).await,
        Command::List(args) => engine::dispatch_list(args).await,
        Command::Validate(args) => engine::dispatch_validate(args).await,
        Command::Compare(args) => engine::dispatch_compare(args).await,
        Command::BaselineRegen(args) => engine::dispatch_baseline_regen(args).await,
        Command::Publish(args) => engine::dispatch_publish(args).await,
        Command::PublishQuarterly(args) => engine::dispatch_publish_quarterly(args).await,
        Command::Evidence(args) => engine::dispatch_evidence(args).await,
        Command::Version => {
            println!("{}", efe_bench_harness::build::long());
            return StdExitCode::SUCCESS;
        }
    };

    let exit = match result {
        Ok(ec) => ec,
        Err(err) => {
            error!(error = %err, "bench-harness aborted");
            err.exit_code()
        }
    };

    StdExitCode::from(u8::try_from(exit.as_i32()).unwrap_or(5))
}

fn init_logging(cli: &Cli) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cli.log_level.as_str()));
    let r = tracing_subscriber::registry().with(filter);
    match cli.log_format {
        efe_bench_harness::cli::LogFormat::Text => {
            r.with(fmt::layer().with_target(true).compact()).init();
        }
        efe_bench_harness::cli::LogFormat::Json => {
            r.with(fmt::layer().json().with_target(true).with_current_span(true)).init();
        }
    }
}

#[must_use]
#[allow(dead_code)]
fn map_exit(result: Result<ExitCode, BenchError>) -> i32 {
    match result {
        Ok(ec) => ec.as_i32(),
        Err(e) => e.exit_code().as_i32(),
    }
}

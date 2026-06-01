//! `efe-slo-rollup` binary entry — see PR #22 § 14 / § 15.

use std::process::ExitCode as StdExitCode;

use efe_slo_rollup::cli::{Cli, Command};
use efe_slo_rollup::rollup;
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
        Command::Compute(args) => rollup::dispatch_compute(args).await,
        Command::Rollup(args) => rollup::dispatch_rollup(args).await,
        Command::Publish(args) => rollup::dispatch_publish(args).await,
        Command::Evidence(args) => rollup::dispatch_evidence(args).await,
        Command::List(args) => rollup::dispatch_list(args).await,
        Command::Validate(args) => rollup::dispatch_validate(args).await,
        Command::Version => {
            println!("{}", efe_slo_rollup::build::long());
            return StdExitCode::SUCCESS;
        }
    };

    let exit = match result {
        Ok(ec) => ec,
        Err(err) => {
            error!(error = %err, "efe-slo-rollup aborted");
            err.exit_code()
        }
    };
    StdExitCode::from(u8::try_from(exit.as_i32()).unwrap_or(7))
}

fn init_logging(cli: &Cli) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cli.log_level.as_str()));
    let r = tracing_subscriber::registry().with(filter);
    match cli.log_format {
        efe_slo_rollup::cli::LogFormat::Text => {
            r.with(fmt::layer().with_target(true).compact()).init();
        }
        efe_slo_rollup::cli::LogFormat::Json => {
            r.with(fmt::layer().json().with_target(true).with_current_span(true)).init();
        }
    }
}


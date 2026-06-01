//! `efe-admin` — EFE tenant administration CLI.
//!
//! Sprint GLOSS A (D8). Statically-compiled binary that wraps the
//! mcp-server m23_* tools and provides a UX-friendly surface for SE
//! (Solutions Engineers) to onboard tenants self-service.
//!
//! Cf. `docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md` § 3.

#![allow(clippy::needless_pass_by_value)] // clap derive idioms
#![allow(clippy::missing_const_for_fn)] // pedantic noise on async fns

use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use efe_admin::{
    cli::{Cli, GlobalArgs, Subcommand},
    commands,
    config::AdminConfig,
    context::AdminContext,
    error::AdminError,
    output::OutputFormat,
};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

#[tokio::main]
async fn main() -> ExitCode {
    // Parse CLI early — bad args ⇒ exit 64 (sysexits.h EX_USAGE).
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            // clap already printed; map exit codes per sysexits.
            return match e.kind() {
                clap::error::ErrorKind::DisplayHelp
                | clap::error::ErrorKind::DisplayVersion => ExitCode::SUCCESS,
                _ => {
                    e.print().ok();
                    ExitCode::from(64)
                }
            };
        }
    };

    init_tracing(&cli.global);

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(AdminError::UserAbort) => {
            eprintln!("aborted.");
            ExitCode::from(1)
        }
        Err(AdminError::AuthFailed(msg)) => {
            eprintln!("auth failure: {msg}");
            ExitCode::from(3)
        }
        Err(AdminError::InvalidInput(msg)) => {
            eprintln!("invalid input: {msg}");
            ExitCode::from(64)
        }
        Err(AdminError::Backend { code, message }) => {
            eprintln!("backend error [{code}]: {message}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> Result<(), AdminError> {
    // Special-case completion generation : no config / no network needed.
    if let Subcommand::Completion(args) = &cli.command {
        let mut cmd = Cli::command();
        let bin = cmd.get_name().to_string();
        clap_complete::generate(args.shell, &mut cmd, bin, &mut std::io::stdout());
        return Ok(());
    }

    // Load config (env > flags > file > defaults).
    let config = AdminConfig::load(&cli.global)?;
    let ctx = AdminContext::new(config, cli.global.clone()).await?;

    match cli.command {
        Subcommand::Tenant(args) => commands::tenant::dispatch(args, &ctx).await,
        Subcommand::Catalog(args) => commands::catalog::dispatch(args, &ctx).await,
        Subcommand::Compliance(args) => commands::compliance::dispatch(args, &ctx).await,
        Subcommand::Keys(args) => commands::keys::dispatch(args, &ctx).await,
        Subcommand::Quotas(args) => commands::quotas::dispatch(args, &ctx).await,
        Subcommand::Audit(args) => commands::audit::dispatch(args, &ctx).await,
        Subcommand::SmokeTest(args) => commands::smoke::run(args, &ctx).await,
        Subcommand::Health(args) => commands::health::run(args, &ctx).await,
        Subcommand::Version => commands::version::run(&ctx),
        Subcommand::Login(args) => commands::login::run(args, &ctx).await,
        Subcommand::Curation(args) => commands::curation::dispatch(args, &ctx).await,
        Subcommand::Learning(args) => commands::learning::dispatch(args, &ctx).await,
        Subcommand::Agent(args) => commands::agent::dispatch(args, &ctx).await,
        Subcommand::Ops(args) => commands::ops::dispatch(args, &ctx).await,
        Subcommand::Completion(_) => unreachable!("handled above"),
    }
}

fn init_tracing(global: &GlobalArgs) {
    let level = if global.verbose >= 2 {
        "trace".to_string()
    } else if global.verbose == 1 {
        "debug".to_string()
    } else if global.quiet {
        "error".to_string()
    } else {
        std::env::var("EFE_ADMIN_LOG").unwrap_or_else(|_| "warn".to_string())
    };

    let filter =
        EnvFilter::try_new(&level).unwrap_or_else(|_| EnvFilter::new("warn"));

    // The two formatter layers have different concrete types ; we
    // erase them to a `Box<dyn Layer<…>>` via the Layer trait's
    // `boxed()` method (provided by tracing-subscriber).
    let fmt_layer = if matches!(global.output_format, Some(OutputFormat::Json)) {
        fmt::layer().with_writer(std::io::stderr).json().boxed()
    } else {
        fmt::layer()
            .with_writer(std::io::stderr)
            .without_time()
            .with_target(false)
            .compact()
            .boxed()
    };

    tracing_subscriber::registry().with(filter).with(fmt_layer).init();
}

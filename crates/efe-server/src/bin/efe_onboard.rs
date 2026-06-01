//! Sprint **D15** — `efe-onboard` CLI.
//!
//! Source-of-truth tooling for the OIDC tenant onboarding workflow.
//!
//! # Synopsis
//!
//! ```text
//! efe-onboard preflight
//!     [--issuers-file <path>] [--pg-url <dsn>]
//!
//! efe-onboard register
//!     --tenant-id <id> --display-name <name>
//!     --idp-issuer <url> --idp-strategy <claim|issuer>
//!     [--idp-meta '{"json":...}'] [--pg-url <dsn>]
//!
//! efe-onboard activate <tenant_id> [--pg-url <dsn>]
//! efe-onboard suspend  <tenant_id> [--pg-url <dsn>]
//! efe-onboard list                  [--pg-url <dsn>] [--json]
//! efe-onboard get <tenant_id>      [--pg-url <dsn>] [--json]
//! ```
//!
//! All commands accept `--pg-url` ; if absent, falls back to
//! `$EFE_PG_URL` then `$EFE_PG_TEST_URL` (CI).
//!
//! `preflight` validates the issuers file *and* connects to Postgres
//! to check the `tenants` table is reachable. It is the recommended
//! gate before flipping a Helm release to `oidc.enabled=true`.
//!
//! # Exit codes
//!
//! * `0` — success.
//! * `1` — operation refused (state-machine violation, format error).
//! * `2` — operational error (DB unreachable, malformed input file).
//!
//! # Audit trail (Sprint D15-bis backlog)
//!
//! **Audit fix D15-C8** — the prior docstring claimed every state
//! transition lands a row in `audit_events` via a SQL trigger. **No
//! such trigger exists** in `migrations/0004_tenants.sql`. The
//! HMAC-chained audit sink (D14-bis) requires Rust-side computation
//! of the chained HMAC ; a SQL trigger cannot reproduce the
//! `prev_hmac → hmac` chain by itself.
//!
//! The honest current state :
//!
//!   * `tenants` lifecycle (register / activate / suspend) is **NOT**
//!     hash-chained in the audit log. Operators get a Postgres row
//!     in `tenants` with `created_at / activated_at / suspended_at`
//!     timestamps but no tamper-evident proof.
//!   * Catalog operations (D14-bis) ARE chained.
//!
//! The proper fix (deferred to D15-bis) is to extend the AuditEmitter
//! schema with a tenant-lifecycle action class, plumb it into
//! `TenantStore::register/activate/suspend`, and add a corresponding
//! verifier path in `efe-verify-audit`. Until then, treat the
//! `tenants` table as **append-mostly governance metadata**, not as
//! cryptographic audit evidence.

#![allow(clippy::missing_docs_in_private_items)]

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use efe_server::auth::{
    load_issuers_file, IdpStrategy, IssuerLoadError, LoadParams, NewTenant, TenantStatus,
    TenantStore, TenantStoreError,
};
use efe_server::config::OidcConfig;
use serde_json::Value as JsonValue;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "efe-onboard",
    version,
    about = "EFE tenant onboarding CLI (D15) — manage the `tenants` registry."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Validate the issuers file + verify the `tenants` table is
    /// reachable. Recommended before activating OIDC in Helm.
    Preflight {
        /// Path to `oidc-issuers.json`. Defaults to
        /// `/etc/efe/oidc/oidc-issuers.json` (the path used by the K8s
        /// Secret mount, post audit-fix D15-M7).
        #[arg(long, default_value = "/etc/efe/oidc/oidc-issuers.json")]
        issuers_file: PathBuf,
        /// Postgres DSN. Falls back to env `EFE_PG_URL` /
        /// `EFE_PG_TEST_URL`.
        #[arg(long)]
        pg_url: Option<String>,
    },

    /// Register a brand-new tenant in `pending` state.
    Register {
        /// Canonical tenant id (regex `^[a-z0-9_-]{1,63}$`).
        #[arg(long)]
        tenant_id: String,
        /// Human-friendly name.
        #[arg(long)]
        display_name: String,
        /// OIDC issuer URL (e.g. `https://acme.auth0.com/`).
        #[arg(long)]
        idp_issuer: String,
        /// IdP resolution strategy.
        #[arg(long, value_parser = parse_idp_strategy)]
        idp_strategy: IdpStrategy,
        /// Vendor-specific JSON blob (default `{}`).
        #[arg(long, default_value = "{}")]
        idp_meta: String,
        /// Postgres DSN. Falls back to env.
        #[arg(long)]
        pg_url: Option<String>,
    },

    /// Transition `pending → active` (or `suspended → active`).
    Activate {
        /// Tenant id.
        tenant_id: String,
        /// Postgres DSN.
        #[arg(long)]
        pg_url: Option<String>,
    },

    /// Transition `pending|active → suspended`.
    Suspend {
        /// Tenant id.
        tenant_id: String,
        /// Postgres DSN.
        #[arg(long)]
        pg_url: Option<String>,
    },

    /// List every tenant.
    List {
        /// Postgres DSN.
        #[arg(long)]
        pg_url: Option<String>,
        /// Emit JSON instead of a TTY table.
        #[arg(long)]
        json: bool,
    },

    /// Show one tenant.
    Get {
        /// Tenant id.
        tenant_id: String,
        /// Postgres DSN.
        #[arg(long)]
        pg_url: Option<String>,
        /// Emit JSON instead of TTY.
        #[arg(long)]
        json: bool,
    },
}

/// `clap` value parser for `IdpStrategy` (claim / issuer).
fn parse_idp_strategy(s: &str) -> Result<IdpStrategy, String> {
    match s {
        "claim" => Ok(IdpStrategy::Claim),
        "issuer" => Ok(IdpStrategy::Issuer),
        other => Err(format!(
            "unknown idp_strategy: {other:?} (expected `claim` or `issuer`)"
        )),
    }
}

#[tokio::main]
async fn main() {
    // Init basic env_logger-style tracing.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();

    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(e.exit_code());
        }
    }
}

/// CLI-level error type — distinguishes "operation refused" (exit 1)
/// from "operational failure" (exit 2) per D14-bis convention.
#[derive(Debug)]
enum CliError {
    /// State-machine / format violation.
    Refused(String),
    /// DB unreachable / file IO / parse / clap error.
    Operational(String),
}

impl CliError {
    fn exit_code(&self) -> i32 {
        match self {
            CliError::Refused(_) => 1,
            CliError::Operational(_) => 2,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Refused(s) | CliError::Operational(s) => f.write_str(s),
        }
    }
}

impl From<TenantStoreError> for CliError {
    fn from(e: TenantStoreError) -> Self {
        match e {
            TenantStoreError::AlreadyExists(_)
            | TenantStoreError::NotFound(_)
            | TenantStoreError::InvalidArgument(_)
            | TenantStoreError::InvalidTransition { .. } => CliError::Refused(e.to_string()),
            TenantStoreError::Inconsistent(_) | TenantStoreError::Database(_) => {
                CliError::Operational(e.to_string())
            }
        }
    }
}

impl From<IssuerLoadError> for CliError {
    fn from(e: IssuerLoadError) -> Self {
        // All variants are operational — bad file, bad DSN, etc.
        CliError::Operational(e.to_string())
    }
}

impl From<sqlx::Error> for CliError {
    fn from(e: sqlx::Error) -> Self {
        CliError::Operational(format!("postgres: {e}"))
    }
}

async fn run(cli: Cli) -> Result<(), CliError> {
    match cli.cmd {
        Cmd::Preflight {
            issuers_file,
            pg_url,
        } => {
            // 1. Issuers file load + validation.
            //
            // `OidcConfig` is `#[non_exhaustive]` (defensive against
            // future-field additions). From outside the defining crate
            // the struct expression with `..Default::default()` is
            // forbidden by E0639. We must use the
            // `let mut cfg = OidcConfig::default(); cfg.field = ...;`
            // pattern, locally allowing
            // `clippy::field_reassign_with_default`.
            #[allow(clippy::field_reassign_with_default)]
            let cfg = {
                let mut cfg = OidcConfig::default();
                cfg.enabled = true;
                cfg.issuers_file = issuers_file.to_string_lossy().to_string();
                cfg
            };
            let params = LoadParams::from_oidc_config(&cfg);
            let registry = load_issuers_file(&issuers_file, &params)?;
            println!("✓ issuers_file OK : {} issuers", registry.len());
            for entry in registry.iter() {
                println!(
                    "    - {}  audience={}  jwks={}  alg={:?}  strategy={:?}",
                    entry.config.issuer,
                    entry.config.audience,
                    entry.config.jwks_url,
                    entry.config.alg_whitelist,
                    entry.config.tenant_strategy
                );
            }
            // 2. Postgres reachability + tenants table check.
            //
            // **Audit fix D15-M3** — use a named lookup instead of
            // positional `counts[0].1, counts[1].1, counts[2].1`
            // which silently breaks if the order in
            // `count_by_status` ever changes.
            let store = connect_store(pg_url.as_deref()).await?;
            let counts = store.count_by_status().await?;
            let pick = |st| {
                counts
                    .iter()
                    .find(|(s, _)| *s == st)
                    .map(|(_, n)| *n)
                    .unwrap_or(0)
            };
            println!(
                "✓ postgres reachable : {} pending / {} active / {} suspended",
                pick(efe_server::auth::TenantStatus::Pending),
                pick(efe_server::auth::TenantStatus::Active),
                pick(efe_server::auth::TenantStatus::Suspended),
            );
            Ok(())
        }

        Cmd::Register {
            tenant_id,
            display_name,
            idp_issuer,
            idp_strategy,
            idp_meta,
            pg_url,
        } => {
            let meta: JsonValue = serde_json::from_str(&idp_meta)
                .map_err(|e| CliError::Refused(format!("--idp-meta is not valid JSON: {e}")))?;
            let store = connect_store(pg_url.as_deref()).await?;
            let new = NewTenant {
                tenant_id: tenant_id.clone(),
                display_name,
                idp_issuer,
                idp_strategy,
                idp_meta: meta,
            };
            let t = store.register(new).await?;
            println!(
                "✓ registered tenant {} (status={})",
                t.tenant_id,
                t.status.as_sql()
            );
            Ok(())
        }

        Cmd::Activate { tenant_id, pg_url } => {
            // **Audit fix D15-H10** — print RFC3339 instead of `Debug`
            // for human-readable ops output. `chrono::DateTime::Display`
            // emits RFC3339 with offset by default.
            let store = connect_store(pg_url.as_deref()).await?;
            let t = store.activate(&tenant_id).await?;
            let ts = t
                .activated_at
                .map_or_else(|| "<unset>".to_string(), |dt| dt.to_rfc3339());
            println!("✓ {} → active (activated_at={ts})", t.tenant_id);
            Ok(())
        }

        Cmd::Suspend { tenant_id, pg_url } => {
            let store = connect_store(pg_url.as_deref()).await?;
            let t = store.suspend(&tenant_id).await?;
            let ts = t
                .suspended_at
                .map_or_else(|| "<unset>".to_string(), |dt| dt.to_rfc3339());
            println!("✓ {} → suspended (suspended_at={ts})", t.tenant_id);
            Ok(())
        }

        Cmd::List { pg_url, json } => {
            let store = connect_store(pg_url.as_deref()).await?;
            let tenants = store.list().await?;
            if json {
                let s = serde_json::to_string_pretty(&tenants)
                    .map_err(|e| CliError::Operational(format!("json: {e}")))?;
                println!("{s}");
            } else {
                print_table(&tenants);
            }
            Ok(())
        }

        Cmd::Get {
            tenant_id,
            pg_url,
            json,
        } => {
            let store = connect_store(pg_url.as_deref()).await?;
            let t = store.get(&tenant_id).await?;
            if json {
                let s = serde_json::to_string_pretty(&t)
                    .map_err(|e| CliError::Operational(format!("json: {e}")))?;
                println!("{s}");
            } else {
                print_one(&t);
            }
            Ok(())
        }
    }
}

/// Resolve the Postgres DSN with the env fallback chain :
/// `--pg-url` > `$EFE_PG_URL` > `$EFE_PG_TEST_URL`.
fn resolve_dsn(cli_arg: Option<&str>) -> Result<String, CliError> {
    if let Some(s) = cli_arg {
        return Ok(s.to_string());
    }
    if let Ok(s) = std::env::var("EFE_PG_URL") {
        if !s.is_empty() {
            return Ok(s);
        }
    }
    if let Ok(s) = std::env::var("EFE_PG_TEST_URL") {
        if !s.is_empty() {
            return Ok(s);
        }
    }
    Err(CliError::Operational(
        "missing --pg-url ; no EFE_PG_URL / EFE_PG_TEST_URL in env".into(),
    ))
}

async fn connect_store(cli_arg: Option<&str>) -> Result<TenantStore, CliError> {
    let dsn = resolve_dsn(cli_arg)?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&dsn)
        .await?;
    Ok(TenantStore::from_pool(pool))
}

fn print_table(tenants: &[efe_server::auth::Tenant]) {
    println!(
        "{:<20} {:<10} {:<30} {:<10} {:<32}",
        "TENANT_ID", "STATUS", "ISSUER", "STRATEGY", "DISPLAY_NAME",
    );
    println!("{:-<104}", "");
    for t in tenants {
        let status_pretty = match t.status {
            TenantStatus::Pending => "pending",
            TenantStatus::Active => "active",
            TenantStatus::Suspended => "suspended",
        };
        println!(
            "{:<20} {:<10} {:<30} {:<10} {:<32}",
            truncate(&t.tenant_id, 20),
            status_pretty,
            truncate(&t.idp_issuer, 30),
            match t.idp_strategy {
                IdpStrategy::Claim => "claim",
                IdpStrategy::Issuer => "issuer",
            },
            truncate(&t.display_name, 32),
        );
    }
}

fn print_one(t: &efe_server::auth::Tenant) {
    // **Audit fix D15-H10** — RFC3339 timestamps everywhere.
    println!("tenant_id     : {}", t.tenant_id);
    println!("display_name  : {}", t.display_name);
    println!("status        : {}", t.status.as_sql());
    println!("idp_issuer    : {}", t.idp_issuer);
    println!("idp_strategy  : {}", t.idp_strategy.as_sql());
    println!("idp_meta      : {}", t.idp_meta);
    println!("created_at    : {}", t.created_at.to_rfc3339());
    if let Some(a) = t.activated_at {
        println!("activated_at  : {}", a.to_rfc3339());
    }
    if let Some(s) = t.suspended_at {
        println!("suspended_at  : {}", s.to_rfc3339());
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

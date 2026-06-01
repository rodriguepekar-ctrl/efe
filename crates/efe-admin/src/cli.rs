//! CLI surface — top-level parser + global args + subcommand enum.
//!
//! All flags / args / value types live here so that integration tests
//! can construct them programmatically.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand as ClapSubcommand};
use clap_complete::Shell;

use crate::output::OutputFormat;

/// `efe-admin` — EFE tenant administration CLI.
#[derive(Debug, Parser)]
#[command(
    name = "efe-admin",
    version = env!("CARGO_PKG_VERSION"),
    about = "EFE tenant administration CLI — Sprint GLOSS A",
    long_about = "Provision tenants, attest compliance, seed catalogs and run\nsmoke tests against an EFE deployment. Wraps the mcp-server m23_*\ntools. Designed for Solutions Engineers running 30-min POC onboardings."
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Subcommand,
}

/// Flags that apply to every subcommand.
#[derive(Debug, Args, Clone, Default)]
pub struct GlobalArgs {
    /// Cluster alias (selected from config). Overrides the default cluster.
    #[arg(long, short = 'C', env = "EFE_ADMIN_CLUSTER", global = true)]
    pub cluster: Option<String>,

    /// Custom config file path. Defaults to `~/.config/efe-admin/config.toml`.
    #[arg(long, env = "EFE_ADMIN_CONFIG", global = true)]
    pub config: Option<PathBuf>,

    /// Output format.
    #[arg(long = "format", value_enum, env = "EFE_ADMIN_FORMAT", global = true)]
    pub output_format: Option<OutputFormat>,

    /// Skip interactive confirmation on destructive operations.
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,

    /// Increase verbosity (-v debug, -vv trace).
    #[arg(short = 'v', long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Silence non-essential output.
    #[arg(short = 'q', long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
}

#[derive(Debug, ClapSubcommand)]
pub enum Subcommand {
    /// Manage tenants (create, list, suspend, delete).
    Tenant(TenantArgs),
    /// Manage catalogs (seed, list, promote).
    Catalog(CatalogArgs),
    /// Manage compliance attestations.
    Compliance(ComplianceArgs),
    /// Manage API keys (create, rotate, revoke).
    Keys(KeysArgs),
    /// Get / set per-tenant quotas.
    Quotas(QuotasArgs),
    /// Query / export audit logs.
    Audit(AuditArgs),
    /// Run a post-deploy smoke test for a tenant.
    SmokeTest(SmokeArgs),
    /// Cluster health snapshot.
    Health(HealthArgs),
    /// Show version + connectivity info.
    Version,
    /// (Experimental) OIDC device-flow login.
    Login(LoginArgs),
    /// Generate shell completion scripts.
    Completion(CompletionArgs),
    /// (M23) LLM-assisted taxonomy curation : propose splits/merges,
    /// review queue, promote, drift events, cost reporting.
    Curation(crate::commands::curation::CurationArgs),
    /// (M24) Active learning loop : queue, retune, promote, GDPR Art. 17.
    Learning(crate::commands::learning::LearningArgs),
    /// (M25) Agent runtime EXPERIMENTAL : runs, approvals, templates, DPIA, cost.
    Agent(crate::commands::agent::AgentArgs),
    /// (Φ-D-OPS) Operational unification : master gate, DR drill, chargeback, FSM audit.
    Ops(crate::commands::ops::OpsArgs),
}

// ─── Tenant subcommand ────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct TenantArgs {
    #[command(subcommand)]
    pub command: TenantCommand,
}

#[derive(Debug, ClapSubcommand)]
pub enum TenantCommand {
    /// Create a new tenant.
    Create(TenantCreateArgs),
    /// List tenants.
    List(TenantListArgs),
    /// Show details for one tenant.
    Get(TenantGetArgs),
    /// Update tenant (quotas, residency).
    Update(TenantUpdateArgs),
    /// Suspend a tenant (sets max_rps=0).
    Suspend(TenantSuspendArgs),
    /// Permanently delete a tenant. Irreversible.
    Delete(TenantDeleteArgs),
}

#[derive(Debug, Args)]
pub struct TenantCreateArgs {
    /// Tenant slug `[a-z0-9-]{3,32}`.
    pub name: String,

    #[arg(long, value_enum, default_value_t = TenantPlan::Trial)]
    pub plan: TenantPlan,

    #[arg(long, default_value_t = 100)]
    pub max_rps: u32,

    #[arg(long, default_value_t = 100_000)]
    pub max_tokens_per_day: u64,

    #[arg(long, value_enum, default_value_t = DataResidency::Any)]
    pub data_residency: DataResidency,

    #[arg(long)]
    pub contact_email: Option<String>,
}

#[derive(Debug, Args)]
pub struct TenantListArgs {
    #[arg(long)]
    pub status: Option<String>,
}

#[derive(Debug, Args)]
pub struct TenantGetArgs {
    pub name: String,
}

#[derive(Debug, Args)]
pub struct TenantUpdateArgs {
    pub name: String,
    #[arg(long)]
    pub max_rps: Option<u32>,
    #[arg(long)]
    pub max_tokens_per_day: Option<u64>,
    #[arg(long, value_enum)]
    pub data_residency: Option<DataResidency>,
    #[arg(long)]
    pub contact_email: Option<String>,
}

#[derive(Debug, Args)]
pub struct TenantSuspendArgs {
    pub name: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, Args)]
pub struct TenantDeleteArgs {
    pub name: String,
    #[arg(long, default_value_t = 2555)]
    pub purge_audit_after_days: u32,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TenantPlan {
    Trial,
    Growth,
    Enterprise,
}

impl std::fmt::Display for TenantPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Trial => "trial",
            Self::Growth => "growth",
            Self::Enterprise => "enterprise",
        })
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
pub enum DataResidency {
    /// EU-only residency (e.g. eu-west-1).
    #[serde(rename = "EU_ONLY")]
    #[clap(name = "EU_ONLY", alias = "eu", alias = "eu_only")]
    Eu,
    /// US-only residency (e.g. us-east-1).
    #[serde(rename = "US_ONLY")]
    #[clap(name = "US_ONLY", alias = "us", alias = "us_only")]
    Us,
    /// No residency constraint.
    #[serde(rename = "any")]
    #[clap(name = "any")]
    Any,
}

impl std::fmt::Display for DataResidency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Eu => "EU_ONLY",
            Self::Us => "US_ONLY",
            Self::Any => "any",
        })
    }
}

// ─── Catalog subcommand ───────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct CatalogArgs {
    #[command(subcommand)]
    pub command: CatalogCommand,
}

#[derive(Debug, ClapSubcommand)]
pub enum CatalogCommand {
    /// Seed catalog from a JSONL bundle.
    Seed(CatalogSeedArgs),
    /// List catalogs for a tenant.
    List(CatalogListArgs),
    /// Promote a catalog to a new bundle version.
    Promote(CatalogPromoteArgs),
}

#[derive(Debug, Args)]
pub struct CatalogSeedArgs {
    pub tenant: String,
    #[arg(long)]
    pub bundle: PathBuf,
    #[arg(long)]
    pub bundle_version: String,
    #[arg(long, default_value = "default")]
    pub catalog: String,
}

#[derive(Debug, Args)]
pub struct CatalogListArgs {
    pub tenant: String,
}

#[derive(Debug, Args)]
pub struct CatalogPromoteArgs {
    pub tenant: String,
    #[arg(long)]
    pub catalog: String,
    #[arg(long)]
    pub to_version: String,
}

// ─── Compliance subcommand ────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct ComplianceArgs {
    #[command(subcommand)]
    pub command: ComplianceCommand,
}

#[derive(Debug, ClapSubcommand)]
pub enum ComplianceCommand {
    /// Attest AI Act Art. 14 + GDPR Art. 22 for a tenant.
    Attest(ComplianceAttestArgs),
    /// Verify current compliance state for a tenant.
    Verify(ComplianceVerifyArgs),
}

#[derive(Debug, Args)]
pub struct ComplianceAttestArgs {
    pub tenant: String,
    /// Acknowledge AI Act Art. 14 (high-risk human oversight).
    #[arg(long)]
    pub ai_act_art14: bool,
    /// Acknowledge GDPR Art. 22 (right-not-to-be-subject).
    #[arg(long)]
    pub gdpr_art22: bool,
    /// Email of the natural person signing the attestation.
    #[arg(long)]
    pub signed_by: String,
    /// DPO contact email (required when --gdpr-art22 is set).
    #[arg(long)]
    pub dpo_contact: Option<String>,
}

#[derive(Debug, Args)]
pub struct ComplianceVerifyArgs {
    pub tenant: String,
}

// ─── Keys subcommand ──────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct KeysArgs {
    #[command(subcommand)]
    pub command: KeysCommand,
}

#[derive(Debug, ClapSubcommand)]
pub enum KeysCommand {
    /// Create a new API key.
    Create(KeyCreateArgs),
    /// Rotate an existing API key (issues a replacement).
    Rotate(KeyRotateArgs),
    /// Revoke an API key.
    Revoke(KeyRevokeArgs),
    /// List active keys for a tenant.
    List(KeyListArgs),
}

#[derive(Debug, Args)]
pub struct KeyCreateArgs {
    pub tenant: String,
    #[arg(long, value_enum, default_value_t = KeyScope::Classify)]
    pub scope: KeyScope,
    /// Expiration as a `humantime` duration (e.g. `30d`, `1y`, `never`).
    #[arg(long, default_value = "30d")]
    pub expires_in: String,
}

#[derive(Debug, Args)]
pub struct KeyRotateArgs {
    pub tenant: String,
    #[arg(long)]
    pub key_id: String,
}

#[derive(Debug, Args)]
pub struct KeyRevokeArgs {
    pub tenant: String,
    #[arg(long)]
    pub key_id: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, Args)]
pub struct KeyListArgs {
    pub tenant: String,
    /// Include revoked + expired keys.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyScope {
    ReadOnly,
    Classify,
    Admin,
}

impl std::fmt::Display for KeyScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ReadOnly => "read_only",
            Self::Classify => "classify",
            Self::Admin => "admin",
        })
    }
}

// ─── Quotas subcommand ────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct QuotasArgs {
    #[command(subcommand)]
    pub command: QuotasCommand,
}

#[derive(Debug, ClapSubcommand)]
pub enum QuotasCommand {
    Get(QuotasGetArgs),
    Set(QuotasSetArgs),
}

#[derive(Debug, Args)]
pub struct QuotasGetArgs {
    pub tenant: String,
}

#[derive(Debug, Args)]
pub struct QuotasSetArgs {
    pub tenant: String,
    #[arg(long)]
    pub max_rps: Option<u32>,
    #[arg(long)]
    pub max_tokens_per_day: Option<u64>,
}

// ─── Audit subcommand ─────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct AuditArgs {
    #[command(subcommand)]
    pub command: AuditCommand,
}

#[derive(Debug, ClapSubcommand)]
pub enum AuditCommand {
    Query(AuditQueryArgs),
    Export(AuditExportArgs),
}

#[derive(Debug, Args)]
pub struct AuditQueryArgs {
    pub tenant: String,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long, value_enum)]
    pub mode: Option<AuditMode>,
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct AuditExportArgs {
    pub tenant: String,
    #[arg(long)]
    pub since: String,
    #[arg(long)]
    pub to: PathBuf,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Serialize)]
pub enum AuditMode {
    #[serde(rename = "MODE_2")]
    Mode2,
    #[serde(rename = "MODE_AUTO_ESC")]
    Auto,
    #[serde(rename = "MODE_3")]
    Mode3,
}

// ─── Misc commands ────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct SmokeArgs {
    pub tenant: String,
    /// Path to JSONL of smoke sentences. Defaults to embedded fixture.
    #[arg(long)]
    pub sentences: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct HealthArgs {
    /// Override cluster URL (defaults to active config).
    #[arg(long)]
    pub cluster: Option<String>,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    #[arg(long)]
    pub idp_url: Option<String>,
    #[arg(long)]
    pub client_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: Shell,
}

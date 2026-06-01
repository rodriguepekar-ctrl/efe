//! `efe-admin agent` — M25 agent runtime CLI commands.
//!
//! Wire-up : add `Agent(agent::AgentArgs)` variant to `cli::Subcommand`
//! and dispatch to `agent::dispatch` in `main.rs`.

use clap::{Args, Subcommand};

use crate::context::AdminContext;
use crate::error::AdminError;

pub mod create;
pub mod list;
pub mod get;
pub mod tail;
pub mod approve;
pub mod reject;
pub mod pause;
pub mod resume;
pub mod terminate;
pub mod template;
pub mod cost;
pub mod dpia;
pub mod graduate;     // M26

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCmd,
}

#[derive(Debug, Subcommand)]
pub enum AgentCmd {
    /// Create an agent run from a template.
    Create {
        #[arg(long)] tenant: String,
        #[arg(long, alias = "tmpl")] template: String,
        #[arg(long, alias = "prompt-file")] prompt_file: Option<String>,
        #[arg(long)] timeout_seconds: Option<i32>,
        #[arg(long)] max_cost_usd: Option<f64>,
        #[arg(long, value_delimiter = ',')] tools: Option<Vec<String>>,
    },
    /// List active agent runs.
    List {
        #[arg(long)] tenant: String,
        #[arg(long, default_value_t = false)] all: bool,
        #[arg(long, default_value_t = 50)] limit: i32,
    },
    /// Get a single run + recent steps + pending approvals.
    Get { run_id: String },
    /// Tail steps live for a run.
    Tail { run_id: String },
    /// Approve a pending step.
    Approve {
        approval_id: String,
        #[arg(long)] reason: String,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
    /// Reject a pending step.
    Reject {
        approval_id: String,
        #[arg(long)] reason: String,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
    /// Pause an active run (state checkpointed).
    Pause { run_id: String, #[arg(long)] reason: Option<String> },
    /// Resume a paused run from its last checkpoint.
    Resume { run_id: String },
    /// Force-terminate a run.
    Terminate {
        run_id: String,
        #[arg(long)] reason: String,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
    /// Template management.
    #[command(subcommand)]
    Template(template::TemplateCmd),
    /// Cost reporting.
    #[command(subcommand)]
    Cost(cost::CostCmd),
    /// DPIA workflow (sign / list / refresh).
    #[command(subcommand)]
    Dpia(dpia::DpiaCmd),
    /// (M26) Tenant graduation EXPERIMENTAL → STABLE.
    #[command(subcommand)]
    Graduate(graduate::GraduateCmd),
}

pub async fn dispatch(args: AgentArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        AgentCmd::Create { tenant, template, prompt_file, timeout_seconds, max_cost_usd, tools } =>
            create::run(tenant, template, prompt_file, timeout_seconds, max_cost_usd, tools, ctx).await,
        AgentCmd::List { tenant, all, limit } => list::run(tenant, all, limit, ctx).await,
        AgentCmd::Get { run_id } => get::run(run_id, ctx).await,
        AgentCmd::Tail { run_id } => tail::run(run_id, ctx).await,
        AgentCmd::Approve { approval_id, reason, confirm } => approve::run(approval_id, reason, confirm, ctx).await,
        AgentCmd::Reject { approval_id, reason, confirm } => reject::run(approval_id, reason, confirm, ctx).await,
        AgentCmd::Pause { run_id, reason } => pause::run(run_id, reason, ctx).await,
        AgentCmd::Resume { run_id } => resume::run(run_id, ctx).await,
        AgentCmd::Terminate { run_id, reason, confirm } => terminate::run(run_id, reason, confirm, ctx).await,
        AgentCmd::Template(c) => template::dispatch(c, ctx).await,
        AgentCmd::Cost(c) => cost::dispatch(c, ctx).await,
        AgentCmd::Dpia(c) => dpia::dispatch(c, ctx).await,
        AgentCmd::Graduate(c) => graduate::dispatch(c, ctx).await,
    }
}

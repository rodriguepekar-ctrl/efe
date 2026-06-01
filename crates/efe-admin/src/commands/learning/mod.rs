//! `efe-admin learning` — M24 active learning CLI commands.
//!
//! Top-level dispatch + arg structs. Reachable from `efe-admin learning
//! <subcommand>` once the `Learning(LearningArgs)` variant is added to
//! `cli::Subcommand`.

use clap::{Args, Subcommand};

use crate::context::AdminContext;
use crate::error::AdminError;

pub mod queue;
pub mod proposal;
pub mod retune;
pub mod promote;
pub mod rollback;
pub mod gdpr;
pub mod stats;
pub mod dpo_consent;

#[derive(Debug, Args)]
pub struct LearningArgs {
    #[command(subcommand)]
    pub command: LearningCmd,
}

#[derive(Debug, Subcommand)]
pub enum LearningCmd {
    /// Queue management : list, stats, pause, resume.
    #[command(subcommand)]
    Queue(queue::QueueCmd),
    /// Proposal review : confirm, reject, edit.
    #[command(subcommand)]
    Proposal(proposal::ProposalCmd),
    /// Retune commands : trigger, list, get.
    #[command(subcommand)]
    Retune(retune::RetuneCmd),
    /// Promote an EvalPass retune run.
    Promote {
        run_id: String,
        #[arg(long, default_value_t = false)] skip_canary: bool,
    },
    /// Rollback a promoted bundle.
    Rollback {
        run_id: String,
        #[arg(long)] reason: String,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
    /// GDPR Art. 17 commands.
    #[command(subcommand)]
    Gdpr(gdpr::GdprCmd),
    /// Stats overview.
    #[command(subcommand)]
    Stats(stats::StatsCmd),
    /// DPO consent management (tenant_admin only).
    #[command(subcommand)]
    DpoConsent(dpo_consent::DpoConsentCmd),
}

pub async fn dispatch(args: LearningArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        LearningCmd::Queue(c) => queue::dispatch(c, ctx).await,
        LearningCmd::Proposal(c) => proposal::dispatch(c, ctx).await,
        LearningCmd::Retune(c) => retune::dispatch(c, ctx).await,
        LearningCmd::Promote { run_id, skip_canary } => promote::run(run_id, skip_canary, ctx).await,
        LearningCmd::Rollback { run_id, reason, confirm } => rollback::run(run_id, reason, confirm, ctx).await,
        LearningCmd::Gdpr(c) => gdpr::dispatch(c, ctx).await,
        LearningCmd::Stats(c) => stats::dispatch(c, ctx).await,
        LearningCmd::DpoConsent(c) => dpo_consent::dispatch(c, ctx).await,
    }
}

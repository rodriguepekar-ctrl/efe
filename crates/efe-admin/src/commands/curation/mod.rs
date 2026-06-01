//! `efe-admin curation` — M23 CLI commands.
//!
//! 14 sub-commands covering the full curation workflow :
//!   - tools : propose-split, propose-merge, explain, synth, audit
//!   - queue : list, get, approve, reject
//!   - versions : promote, rollback, diff
//!   - drift : list, ack, stats
//!   - cost : report
//!
//! Reachable from `efe-admin curation <subcommand>` once the
//! `Curation(CurationArgs)` variant is added to `cli::Subcommand` and
//! routed through `dispatch` in this module.

use clap::{Args, Subcommand};
use std::fmt;

use crate::context::AdminContext;
use crate::error::AdminError;

pub mod approve;
pub mod audit;
pub mod cost;
pub mod diff;
pub mod drift;
pub mod explain;
pub mod get;
pub mod list;
pub mod promote;
pub mod propose_merge;
pub mod propose_split;
pub mod reject;
pub mod rollback;
pub mod synth;

#[derive(Debug, Args)]
pub struct CurationArgs {
    #[command(subcommand)]
    pub command: CurationCmd,
}

#[derive(Debug, Subcommand)]
pub enum CurationCmd {
    /// List pending proposals.
    List(ListArgs),
    /// Show one proposal's full details.
    Get { id: String },
    /// Propose a class split (calls `efe_propose_taxonomy_split`).
    ProposeSplit(ProposeSplitArgs),
    /// Propose a class merge (calls `efe_suggest_class_merge`).
    ProposeMerge(ProposeMergeArgs),
    /// Explain why a sentence was misclassified.
    Explain(ExplainArgs),
    /// Generate synthetic examples for a class.
    Synth(SynthArgs),
    /// Audit a bundle against a gold dataset.
    Audit(AuditArgs),
    /// Approve a pending proposal (writes a curation_review row).
    Approve {
        id: String,
        #[arg(long)] reason: String,
    },
    /// Reject a pending proposal.
    Reject {
        id: String,
        #[arg(long)] reason: String,
    },
    /// Promote an APPROVED proposal (atomic, signed, canary-gated).
    Promote {
        id: String,
        #[arg(long, default_value_t = false)] skip_canary: bool,
    },
    /// Rollback a taxonomy version to a prior parent.
    Rollback {
        version_id: String,
        #[arg(long)] reason: String,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
    /// Compute a semantic diff between two taxonomy versions.
    Diff {
        from: String,
        to: String,
        #[arg(long, default_value = "table")] format: OutputFormat,
        #[arg(long)] gold_dataset_id: Option<String>,
    },
    /// Drift event commands.
    #[command(subcommand)]
    Drift(drift::DriftCmd),
    /// Cost reporting.
    #[command(subcommand)]
    Cost(cost::CostCmd),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long)] pub catalog: Option<String>,
    #[arg(long, default_value_t = 50)] pub limit: i64,
    #[arg(long, default_value = "table")] pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ProposeSplitArgs {
    #[arg(long)] pub catalog: String,
    #[arg(long)] pub class: String,
    #[arg(long, default_value_t = 3)] pub max_splits: u32,
    #[arg(long, default_value_t = 20)] pub sample_size: u32,
    #[arg(long, default_value = "SAMPLING_DELEGATION")] pub mode: String,
    #[arg(long, default_value_t = false)] pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ProposeMergeArgs {
    #[arg(long)] pub catalog: String,
    /// Comma-separated list of class IDs to merge.
    #[arg(long, value_delimiter = ',')] pub classes: Vec<String>,
    #[arg(long, default_value = "SAMPLING_DELEGATION")] pub mode: String,
    #[arg(long, default_value_t = false)] pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ExplainArgs {
    pub sentence_id: String,
    #[arg(long)] pub catalog: String,
    #[arg(long)] pub actual: String,
    #[arg(long)] pub expected: Option<String>,
    #[arg(long, default_value = "SAMPLING_DELEGATION")] pub mode: String,
}

#[derive(Debug, Args)]
pub struct SynthArgs {
    #[arg(long)] pub catalog: String,
    #[arg(long)] pub class: String,
    #[arg(long, default_value_t = 20)] pub n: u32,
    #[arg(long, default_value = "MEDIUM")] pub diversity: String,
    #[arg(long, default_value_t = false)] pub include_negative: bool,
    #[arg(long, default_value = "SAMPLING_DELEGATION")] pub mode: String,
}

#[derive(Debug, Args)]
pub struct AuditArgs {
    #[arg(long)] pub catalog: String,
    #[arg(long)] pub bundle: String,
    #[arg(long)] pub gold: String,
    #[arg(long, default_value_t = 10)] pub max_weak_spots: u32,
    #[arg(long, default_value_t = false)] pub with_llm: bool,
    #[arg(long, default_value = "SAMPLING_DELEGATION")] pub mode: String,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum OutputFormat { Table, Json, Yaml }

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Table => write!(f, "table"),
            Self::Json  => write!(f, "json"),
            Self::Yaml  => write!(f, "yaml"),
        }
    }
}

/// Top-level dispatch — called from `cli.rs` Subcommand::Curation arm.
pub async fn dispatch(args: CurationArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        CurationCmd::List(a)            => list::run(a, ctx).await,
        CurationCmd::Get { id }         => get::run(id, ctx).await,
        CurationCmd::ProposeSplit(a)    => propose_split::run(a, ctx).await,
        CurationCmd::ProposeMerge(a)    => propose_merge::run(a, ctx).await,
        CurationCmd::Explain(a)         => explain::run(a, ctx).await,
        CurationCmd::Synth(a)           => synth::run(a, ctx).await,
        CurationCmd::Audit(a)           => audit::run(a, ctx).await,
        CurationCmd::Approve { id, reason }    => approve::run(id, reason, ctx).await,
        CurationCmd::Reject  { id, reason }    => reject::run(id, reason, ctx).await,
        CurationCmd::Promote { id, skip_canary } => promote::run(id, skip_canary, ctx).await,
        CurationCmd::Rollback { version_id, reason, confirm } => {
            rollback::run(version_id, reason, confirm, ctx).await
        }
        CurationCmd::Diff { from, to, format, gold_dataset_id } => {
            diff::run(from, to, format, gold_dataset_id, ctx).await
        }
        CurationCmd::Drift(c) => drift::dispatch(c, ctx).await,
        CurationCmd::Cost(c)  => cost::dispatch(c, ctx).await,
    }
}

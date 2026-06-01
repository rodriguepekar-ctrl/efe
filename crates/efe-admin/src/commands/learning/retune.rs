//! `efe-admin learning retune {trigger,list,get}`.

use clap::Subcommand;
use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum RetuneCmd {
    /// On-demand retune trigger.
    Trigger {
        #[arg(long)] catalog: String,
        #[arg(long, default_value_t = false)] force_below_threshold: bool,
    },
    /// List recent retune runs.
    List {
        #[arg(long)] catalog: Option<String>,
        #[arg(long, default_value_t = 50)] limit: u32,
    },
    /// Show one retune run detail.
    Get { run_id: String },
}

pub async fn dispatch(cmd: RetuneCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::active_learning_client::ActiveLearningClient::connect(ctx).await?;
    match cmd {
        RetuneCmd::Trigger { catalog, force_below_threshold } => {
            let r = client.trigger_retune(&catalog, force_below_threshold).await?;
            println!("✓ Retune triggered : run {} started at {}", r.retune_run_id, r.started_at);
        }
        RetuneCmd::List { catalog, limit } => {
            let runs = client.list_retunes(catalog.as_deref(), limit).await?;
            println!("{:<14} {:<22} {:<10} {:<10} {:<12} {:<8} {}",
                     "ID", "CATALOG", "SEMVER", "TRAIN", "HOLDOUT", "FLAG", "STATUS");
            println!("{}", "─".repeat(90));
            for r in &runs {
                let flag = if r.flagged_for_promotion { "★" } else { " " };
                println!("{:<14} {:<22} {:<10} {:<10} {:<12} {:<8} {}",
                    &r.id[..14.min(r.id.len())], r.catalog_id, r.new_bundle_semver,
                    r.training_examples_count, r.holdout_examples_count, flag, r.status);
            }
            println!("\n{} runs.", runs.len());
        }
        RetuneCmd::Get { run_id } => {
            let d = client.get_retune(&run_id).await?;
            println!("Retune run : {}", d.summary.id);
            println!("  Catalog       : {}", d.summary.catalog_id);
            println!("  Semver        : {}", d.summary.new_bundle_semver);
            println!("  Training      : {}", d.summary.training_examples_count);
            println!("  Holdout       : {}", d.summary.holdout_examples_count);
            println!("  Status        : {}", d.summary.status);
            println!("  Flagged       : {}", d.summary.flagged_for_promotion);
            println!("  Started       : {}", d.summary.started_at);
            if let Some(c) = d.summary.completed_at {
                println!("  Completed     : {}", c);
            }
            if let Some(m) = d.eval_metrics {
                println!("\nEval metrics :\n{}", serde_json::to_string_pretty(&m).unwrap_or_default());
            }
        }
    }
    Ok(())
}

//! `efe-admin learning queue {list,stats}`.

use clap::Subcommand;
use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum QueueCmd {
    /// List pending queue items.
    List {
        #[arg(long)] catalog: Option<String>,
        #[arg(long)] status: Option<String>,
        #[arg(long, default_value_t = 50)] limit: u32,
    },
    /// Quick stats : counts by status, oldest age.
    Stats {
        #[arg(long)] catalog: Option<String>,
    },
}

pub async fn dispatch(cmd: QueueCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::active_learning_client::ActiveLearningClient::connect(ctx).await?;
    match cmd {
        QueueCmd::List { catalog, status, limit } => {
            let items = client.list_queue(catalog.as_deref(), status.as_deref(), limit).await?;
            println!("{:<14} {:<20} {:<18} {:<6} {:<12} {}", "ID", "CATALOG", "STATUS", "CONF", "AGE", "PROPOSED");
            println!("{}", "─".repeat(90));
            for it in &items {
                let age_min = (chrono::Utc::now() - it.enqueued_at).num_minutes();
                let conf = it.observed_confidence.map(|c| format!("{:.2}", c)).unwrap_or_default();
                let proposed = it.proposed_class_id.as_deref().unwrap_or("?");
                println!("{:<14} {:<20} {:<18} {:<6} {:<12} {}",
                    &it.id[..14.min(it.id.len())], it.catalog_id, it.status, conf,
                    format!("{}m", age_min), proposed);
            }
            println!("\n{} items.", items.len());
        }
        QueueCmd::Stats { catalog } => {
            let s = client.get_stats(catalog.as_deref()).await?;
            println!("Queue stats :");
            println!("  enqueued        : {}", s.queue_enqueued);
            println!("  labeling        : {}", s.queue_labeling);
            println!("  awaiting_review : {}", s.queue_awaiting_review);
            println!("  failed          : {}", s.queue_failed);
            println!("  oldest open age : {}s", s.oldest_open_age_seconds);
            println!();
            println!("Gold dataset :");
            println!("  total examples  : {}", s.total_gold_examples);
            println!("  holdout         : {}", s.total_holdout_examples);
            println!();
            println!("Retune (last 30d) : {} runs, {} promotions",
                     s.retune_runs_last_30d, s.promotions_last_30d);
            println!("Cost            : ${:.2} today, ${:.2} last 30d",
                     s.cost_cents_today as f64 / 100.0,
                     s.cost_cents_last_30d as f64 / 100.0);
        }
    }
    Ok(())
}

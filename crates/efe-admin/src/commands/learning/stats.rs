//! `efe-admin learning stats {overview}`.

use clap::Subcommand;
use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum StatsCmd {
    /// All-up M24 stats per catalog.
    Overview {
        #[arg(long)] catalog: Option<String>,
    },
}

pub async fn dispatch(cmd: StatsCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::active_learning_client::ActiveLearningClient::connect(ctx).await?;
    match cmd {
        StatsCmd::Overview { catalog } => {
            let s = client.get_stats(catalog.as_deref()).await?;
            println!("──────────────── M24 stats ────────────────");
            println!("Queue            :");
            println!("  enqueued        : {}", s.queue_enqueued);
            println!("  labeling        : {}", s.queue_labeling);
            println!("  awaiting_review : {}", s.queue_awaiting_review);
            println!("  failed          : {}", s.queue_failed);
            if s.oldest_open_age_seconds > 0 {
                let h = s.oldest_open_age_seconds / 3600;
                let m = (s.oldest_open_age_seconds % 3600) / 60;
                println!("  oldest age      : {}h {}m", h, m);
            }
            println!();
            println!("Gold dataset     :");
            println!("  total examples  : {}", s.total_gold_examples);
            println!("  holdout         : {} (~{:.1}%)",
                s.total_holdout_examples,
                if s.total_gold_examples > 0 {
                    100.0 * (s.total_holdout_examples as f64) / (s.total_gold_examples as f64)
                } else { 0.0 });
            println!();
            println!("Retune (last 30d):");
            println!("  runs            : {}", s.retune_runs_last_30d);
            println!("  promotions      : {}", s.promotions_last_30d);
            println!();
            println!("Cost             :");
            println!("  today           : ${:.2}", s.cost_cents_today as f64 / 100.0);
            println!("  last 30d        : ${:.2}", s.cost_cents_last_30d as f64 / 100.0);
        }
    }
    Ok(())
}

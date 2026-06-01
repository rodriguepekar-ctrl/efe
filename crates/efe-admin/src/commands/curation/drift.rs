//! `efe-admin curation drift {list|ack|stats}`

use clap::Subcommand;
use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum DriftCmd {
    /// List recent drift events.
    List {
        #[arg(long)] catalog: Option<String>,
        #[arg(long, default_value = "LOW")] min_severity: String,
        #[arg(long, default_value_t = false)] only_unacknowledged: bool,
        #[arg(long, default_value_t = 50)] limit: u32,
    },
    /// Acknowledge a drift event.
    Ack { event_id: String, #[arg(long)] note: Option<String> },
    /// Stats per catalog (last 30 days).
    Stats {
        #[arg(long)] catalog: Option<String>,
    },
}

pub async fn dispatch(cmd: DriftCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    match cmd {
        DriftCmd::List { catalog, min_severity, only_unacknowledged, limit } => {
            let events = client.list_drift_events(
                catalog.as_deref(), &min_severity, only_unacknowledged, limit,
            ).await?;
            println!("{:<14} {:<10} {:<12} {:<8} {:<6} {}",
                     "ID", "SEVERITY", "CATALOG/CLASS", "KL", "AGE", "STATUS");
            println!("{}", "─".repeat(80));
            for e in &events {
                let age = chrono::Utc::now().signed_duration_since(e.detected_at);
                let status = if e.acknowledged_at.is_some() { "ack" } else { "open" };
                println!("{:<14} {:<10} {:<12} {:<8.3} {:<6}h {}",
                         &e.id[..14.min(e.id.len())], e.severity, e.class_id,
                         e.kl_divergence, age.num_hours(), status);
            }
            println!("\n{} events.", events.len());
        }
        DriftCmd::Ack { event_id, note } => {
            client.acknowledge_drift_event(&event_id, note.as_deref()).await?;
            println!("✓ Drift event {} acknowledged", event_id);
        }
        DriftCmd::Stats { catalog } => {
            let stats = client.drift_stats(catalog.as_deref(), 30).await?;
            println!("Drift stats (last 30 days) :");
            println!("  total events     : {}", stats.total);
            println!("  HIGH severity    : {}", stats.high);
            println!("  MEDIUM severity  : {}", stats.medium);
            println!("  LOW severity     : {}", stats.low);
            println!("  unacknowledged   : {}", stats.unack);
        }
    }
    Ok(())
}

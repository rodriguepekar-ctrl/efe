//! `efe-admin curation cost report --period last-30d`

use clap::Subcommand;
use chrono::{Duration, Utc};
use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum CostCmd {
    /// Cost report grouped by tool and provider.
    Report {
        #[arg(long, default_value = "last-30d")] period: String,
        #[arg(long, value_delimiter = ',', default_values_t = vec!["tool_name".to_string(), "provider".to_string()])]
        group_by: Vec<String>,
    },
}

pub async fn dispatch(cmd: CostCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    match cmd {
        CostCmd::Report { period, group_by } => {
            let (from, to) = parse_period(&period)?;
            let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
            let report = client.cost_report(from, to, &group_by).await?;
            println!("Curation cost report : {} → {}", from, to);
            println!("Total : ${:.2} ({} cents)", report.total_cost_cents as f64 / 100.0, report.total_cost_cents);
            println!();
            println!("{:<30} {:<10} {:<10} {}", "GROUP", "CALLS", "TOKENS", "COST");
            println!("{}", "─".repeat(70));
            for b in &report.buckets {
                let label_pairs: Vec<String> = b.labels.iter()
                    .map(|(k, v)| format!("{}={}", k, v)).collect();
                println!("{:<30} {:<10} {:<10} ${:.2}",
                         label_pairs.join(","), b.call_count,
                         b.input_tokens + b.output_tokens,
                         b.cost_cents as f64 / 100.0);
            }
        }
    }
    Ok(())
}

fn parse_period(p: &str) -> Result<(chrono::DateTime<Utc>, chrono::DateTime<Utc>), AdminError> {
    let now = Utc::now();
    Ok(match p {
        "last-7d"  => (now - Duration::days(7), now),
        "last-30d" => (now - Duration::days(30), now),
        "last-90d" => (now - Duration::days(90), now),
        s if s.starts_with("last-") && s.ends_with('d') => {
            let n: i64 = s[5..s.len()-1].parse()
                .map_err(|_| AdminError::InvalidInput(format!("bad period : {}", s)))?;
            (now - Duration::days(n), now)
        }
        _ => return Err(AdminError::InvalidInput(format!("unknown period : {}", p))),
    })
}

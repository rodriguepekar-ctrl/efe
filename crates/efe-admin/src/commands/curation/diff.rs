//! `efe-admin curation diff <from> <to>` — semantic diff between two versions.

use super::OutputFormat;
use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(
    from: String, to: String, format: OutputFormat,
    gold_dataset_id: Option<String>, ctx: &AdminContext,
) -> Result<(), AdminError> {
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let diff = client.compute_diff(&from, &to, gold_dataset_id.as_deref()).await?;

    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&diff)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&diff)?),
        OutputFormat::Table => {
            println!("Diff : {} → {}", diff.from_semver, diff.to_semver);
            println!("{}", "─".repeat(60));
            for change in &diff.class_changes {
                println!("  {} {} → {}", change.kind_label(), change.label_from(), change.label_to());
            }
            if let Some(impact) = &diff.estimated_impact {
                println!();
                println!("Estimated impact :");
                println!("  affected examples  : {}", impact.affected_examples_count);
                println!("  accuracy delta     : {:+.2} pp", impact.estimated_accuracy_delta_pp);
                println!("  re-label required  : {}", impact.re_label_required);
            }
        }
    }
    Ok(())
}

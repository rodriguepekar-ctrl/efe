//! `efe-admin learning proposal {confirm,reject,edit}`.

use clap::Subcommand;
use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum ProposalCmd {
    /// Confirm an LLM-proposed label as-is.
    Confirm {
        id: String,
        #[arg(long)] reason: String,
    },
    /// Reject an LLM-proposed label.
    Reject {
        id: String,
        #[arg(long)] reason: String,
    },
    /// Edit and confirm (replace LLM-proposed class).
    Edit {
        id: String,
        #[arg(long)] class: String,
        #[arg(long)] reason: String,
    },
}

pub async fn dispatch(cmd: ProposalCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::active_learning_client::ActiveLearningClient::connect(ctx).await?;
    match cmd {
        ProposalCmd::Confirm { id, reason } => {
            if reason.trim().len() < 10 {
                return Err(AdminError::InvalidInput("rationale must be ≥ 10 chars (Art. 14)".into()));
            }
            let r = client.confirm(&id, &reason, None).await?;
            println!("✓ Proposal {} confirmed (review {})", id, r.review_id);
        }
        ProposalCmd::Reject { id, reason } => {
            if reason.trim().len() < 10 {
                return Err(AdminError::InvalidInput("rationale must be ≥ 10 chars".into()));
            }
            let r = client.reject(&id, &reason).await?;
            println!("✓ Proposal {} rejected (review {})", id, r.review_id);
        }
        ProposalCmd::Edit { id, class, reason } => {
            if reason.trim().len() < 10 {
                return Err(AdminError::InvalidInput("rationale must be ≥ 10 chars".into()));
            }
            let r = client.confirm(&id, &reason, Some(&class)).await?;
            println!("✓ Proposal {} edited to class '{}' and confirmed (review {})", id, class, r.review_id);
        }
    }
    Ok(())
}

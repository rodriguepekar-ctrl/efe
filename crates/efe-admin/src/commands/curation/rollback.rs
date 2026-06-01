//! `efe-admin curation rollback <version-id> --reason <...> [--confirm]`

use crate::context::AdminContext;
use crate::error::AdminError;
use std::io::{self, Write};

pub async fn run(
    version_id: String, reason: String, auto_confirm: bool, ctx: &AdminContext,
) -> Result<(), AdminError> {
    if reason.trim().len() < 10 {
        return Err(AdminError::InvalidInput("rationale must be ≥ 10 chars".into()));
    }

    if !auto_confirm {
        eprint!(
            "⚠ This will roll back to version {}.\n\
             A new ACTIVE version will be created (the current one becomes ROLLED_BACK).\n\
             This action is logged in the audit chain and cannot be hidden.\n\n\
             Type the version_id to confirm : ",
            version_id
        );
        io::stderr().flush()?;
        let mut typed = String::new();
        io::stdin().read_line(&mut typed)?;
        if typed.trim() != version_id {
            tracing::warn!("rollback confirmation mismatch — aborting");
            return Err(AdminError::UserAbort);
        }
    }

    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let new_id = client.rollback_version(&version_id, &reason).await?;
    println!("✓ Rollback complete. New ACTIVE version : {}", new_id);
    Ok(())
}

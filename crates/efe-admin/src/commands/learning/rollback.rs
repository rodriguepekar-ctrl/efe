//! `efe-admin learning rollback <run-id> --reason <...> [--confirm]`.

use std::io::{self, Write};

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(
    run_id: String, reason: String, auto_confirm: bool, ctx: &AdminContext,
) -> Result<(), AdminError> {
    if reason.trim().len() < 10 {
        return Err(AdminError::InvalidInput("rationale must be ≥ 10 chars".into()));
    }
    if !auto_confirm {
        eprint!(
            "⚠ Rolling back retune run {}.\n\
             Will restore previous ACTIVE bundle. This action is logged in\n\
             the audit chain (HMAC) and cannot be hidden.\n\n\
             Type the run_id to confirm : ",
            run_id
        );
        io::stderr().flush()?;
        let mut typed = String::new();
        io::stdin().read_line(&mut typed)?;
        if typed.trim() != run_id {
            tracing::warn!("rollback confirmation mismatch — aborting");
            return Err(AdminError::UserAbort);
        }
    }
    let client = crate::grpc::active_learning_client::ActiveLearningClient::connect(ctx).await?;
    let restored = client.rollback(&run_id, &reason).await?;
    println!("✓ Rollback complete. Restored ACTIVE version : {}", restored);
    Ok(())
}

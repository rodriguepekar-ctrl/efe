//! `efe-admin login` — OIDC device flow (post-MVP placeholder).
//!
//! v1.0.0 stub : returns a clear « not implemented » error so that
//! E2E tests exercising the command path know what to expect.

use crate::cli::LoginArgs;
use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(_a: LoginArgs, _ctx: &AdminContext) -> Result<(), AdminError> {
    Err(AdminError::AuthFailed(
        "OIDC device flow login is post-MVP (use auth_method=token in your config)".into(),
    ))
}

//! Per-invocation runtime context.
//!
//! Built once at startup, threaded through every command handler.
//! Holds the resolved config, the auth provider and lazy clients.

use std::sync::OnceLock;

use crate::auth::TokenProvider;
use crate::cli::GlobalArgs;
use crate::config::{AdminConfig, AuthMethod};
use crate::error::AdminError;
use crate::mcp::McpClient;
use crate::output::Renderer;

pub struct AdminContext {
    pub config: AdminConfig,
    pub global: GlobalArgs,
    pub auth: TokenProvider,
    pub renderer: Renderer,
    mcp: OnceLock<McpClient>,
}

impl AdminContext {
    pub async fn new(config: AdminConfig, global: GlobalArgs) -> Result<Self, AdminError> {
        let auth = match config.auth_method {
            AuthMethod::None => TokenProvider::none(),
            AuthMethod::Token => TokenProvider::file(
                config
                    .token_file
                    .clone()
                    .ok_or_else(|| AdminError::Config("token_file required for auth_method=token".into()))?,
            )?,
            AuthMethod::KubeSa => TokenProvider::kube_sa()?,
            AuthMethod::Sso => {
                // Placeholder : v1.0.0 ships with a stub that errors.
                return Err(AdminError::AuthFailed(
                    "SSO auth method not implemented in v1.0.0 (use --feature sso)".into(),
                ));
            }
        };

        let renderer = Renderer::new(config.output_format);

        Ok(Self {
            config,
            global,
            auth,
            renderer,
            mcp: OnceLock::new(),
        })
    }

    /// Returns the lazily-initialised mcp-server client.
    pub fn mcp(&self) -> &McpClient {
        self.mcp.get_or_init(|| {
            McpClient::new(self.config.cluster_url.clone(), self.auth.clone())
                .expect("mcp client init")
        })
    }

    /// True if confirmation should be skipped.
    pub fn confirm_bypass(&self) -> bool {
        self.global.yes
            || matches!(self.config.confirm_mode, crate::config::ConfirmMode::AlwaysYes)
    }
}

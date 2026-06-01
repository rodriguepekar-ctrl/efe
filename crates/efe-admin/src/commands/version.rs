//! `efe-admin version` — print version + active cluster info.

use serde::Serialize;

use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Serialize)]
struct VersionInfo {
    version: &'static str,
    binary: &'static str,
    cluster_alias: Option<String>,
    cluster_url: String,
    auth_method: String,
    output_format: String,
}

pub fn run(ctx: &AdminContext) -> Result<(), AdminError> {
    // `CARGO_BIN_NAME` is only set when compiling the bin target ; we
    // hard-code the constant string instead so the lib also builds.
    let info = VersionInfo {
        version: crate::VERSION,
        binary: "efe-admin",
        cluster_alias: ctx.config.cluster_alias.clone(),
        cluster_url: ctx.config.cluster_url.clone(),
        auth_method: format!("{:?}", ctx.config.auth_method).to_lowercase(),
        output_format: format!("{:?}", ctx.config.output_format).to_lowercase(),
    };
    ctx.renderer.print(&info)
}

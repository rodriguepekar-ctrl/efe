//! `efe-admin compliance <subcommand>` — AI Act Art. 14 + GDPR Art. 22 attestations.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cli::{ComplianceArgs, ComplianceAttestArgs, ComplianceCommand, ComplianceVerifyArgs};
use crate::context::AdminContext;
use crate::error::AdminError;
use crate::prompts::confirm;
use crate::validation;

const TOOL_ATTEST: &str = "efe_attest_compliance";
const TOOL_VERIFY: &str = "efe_verify_compliance";

pub async fn dispatch(args: ComplianceArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        ComplianceCommand::Attest(a) => attest(a, ctx).await,
        ComplianceCommand::Verify(a) => verify(a, ctx).await,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct AttestOutput {
    tenant_id: String,
    ai_act_art14_acknowledged: bool,
    gdpr_art22_acknowledged: bool,
    signed_by: String,
    attested_at: String,
}

async fn attest(a: ComplianceAttestArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    validation::email(&a.signed_by)?;

    if !a.ai_act_art14 && !a.gdpr_art22 {
        return Err(AdminError::invalid(
            "supply at least one of --ai-act-art14 or --gdpr-art22",
        ));
    }
    if a.gdpr_art22 && a.dpo_contact.is_none() {
        return Err(AdminError::invalid(
            "--dpo-contact is required when --gdpr-art22 is set (GDPR Art. 37)",
        ));
    }
    if let Some(dpo) = &a.dpo_contact {
        validation::email(dpo)?;
    }

    let summary = build_attest_summary(&a);
    if !ctx.confirm_bypass()
        && !confirm(
            &format!("Attest compliance for '{}' ?\n{}", a.tenant, summary),
            ctx.config.confirm_mode,
        )?
    {
        return Err(AdminError::UserAbort);
    }

    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_ATTEST,
            json!({
                "tenant_id": a.tenant,
                "ai_act_art14": a.ai_act_art14,
                "gdpr_art22": a.gdpr_art22,
                "signed_by": a.signed_by,
                "dpo_contact": a.dpo_contact,
                "confirm": true,
            }),
        )
        .await?;
    let parsed: AttestOutput = serde_json::from_value(resp)?;
    eprintln!(
        "✅ compliance attested for '{}' (AI Act Art.14={}, GDPR Art.22={}) by {} at {}",
        parsed.tenant_id,
        parsed.ai_act_art14_acknowledged,
        parsed.gdpr_art22_acknowledged,
        parsed.signed_by,
        parsed.attested_at,
    );
    ctx.renderer.print(&parsed)
}

fn build_attest_summary(a: &ComplianceAttestArgs) -> String {
    let mut out = String::new();
    if a.ai_act_art14 {
        out.push_str("  - EU AI Act Art. 14 (high-risk human oversight)\n");
    }
    if a.gdpr_art22 {
        out.push_str("  - GDPR Art. 22 (right not to be subject to automated decisions)\n");
        if let Some(c) = &a.dpo_contact {
            out.push_str(&format!("  - DPO contact: {c}\n"));
        }
    }
    out.push_str(&format!("  - Signed by:   {}\n", a.signed_by));
    out
}

#[derive(Debug, Serialize, Deserialize)]
struct VerifyOutput {
    tenant_id: String,
    ai_act_art14: AttestStatus,
    gdpr_art22: AttestStatus,
    soc2_in_scope: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AttestStatus {
    acknowledged: bool,
    acknowledged_at: Option<String>,
    signed_by: Option<String>,
    expires_in_days: Option<i64>,
}

async fn verify(a: ComplianceVerifyArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    let resp = ctx.mcp().call_tool(TOOL_VERIFY, json!({"tenant_id": a.tenant})).await?;
    let parsed: VerifyOutput = serde_json::from_value(resp)?;
    ctx.renderer.print(&parsed)
}

#[cfg(test)]
mod tests {
    use crate::cli::ComplianceAttestArgs;

    #[test]
    fn summary_lists_only_set_attestations() {
        let a = ComplianceAttestArgs {
            tenant: "acme".into(),
            ai_act_art14: true,
            gdpr_art22: false,
            signed_by: "jane@acme.com".into(),
            dpo_contact: None,
        };
        let s = super::build_attest_summary(&a);
        assert!(s.contains("AI Act"));
        assert!(!s.contains("GDPR"));
    }

    #[test]
    fn summary_with_gdpr_includes_dpo() {
        let a = ComplianceAttestArgs {
            tenant: "acme".into(),
            ai_act_art14: true,
            gdpr_art22: true,
            signed_by: "jane@acme.com".into(),
            dpo_contact: Some("dpo@acme.com".into()),
        };
        let s = super::build_attest_summary(&a);
        assert!(s.contains("AI Act"));
        assert!(s.contains("GDPR"));
        assert!(s.contains("dpo@acme.com"));
    }
}

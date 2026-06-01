//! `efe-admin agent graduate` — M26 tenant graduation workflow.

use clap::Subcommand;

use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum GraduateCmd {
    /// Read-only check : prints the 6 operational + 4 compliance criteria status.
    Check {
        #[arg(long)] tenant: String,
    },
    /// Confirm the graduation. Requires `--acknowledgement-hash sha256:...`
    /// and the operator `admin` role.
    Confirm {
        #[arg(long)] tenant: String,
        #[arg(long)] acknowledgement_hash: String,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
    /// Revoke graduation (move tenant back to EXPERIMENTAL).
    Revoke {
        #[arg(long)] tenant: String,
        #[arg(long)] reason: String,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
    /// Quick status print (channel + DPIA expiry).
    Status {
        #[arg(long)] tenant: String,
    },
}

pub async fn dispatch(c: GraduateCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    match c {
        GraduateCmd::Check { tenant } => check(tenant, ctx).await,
        GraduateCmd::Confirm { tenant, acknowledgement_hash, confirm } => {
            confirm_cmd(tenant, acknowledgement_hash, confirm, ctx).await
        }
        GraduateCmd::Revoke { tenant, reason, confirm } => revoke(tenant, reason, confirm, ctx).await,
        GraduateCmd::Status { tenant } => status(tenant, ctx).await,
    }
}

async fn check(tenant: String, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::agent_client::AgentClient::connect(ctx).await?;
    let result = client.graduate_check(&tenant).await?;

    println!("Graduation criteria for tenant '{}'", tenant);
    println!("");
    println!("Operational :");
    print_criterion(
        "30 days in EXPERIMENTAL",
        result.operational.days_in_experimental >= result.operational.min_days_required,
        format!(
            "{} / {}",
            result.operational.days_in_experimental, result.operational.min_days_required
        ),
    );
    print_criterion(
        "≥ 100 successful runs",
        result.operational.successful_runs >= result.operational.min_runs_required,
        format!(
            "{} / {}",
            result.operational.successful_runs, result.operational.min_runs_required
        ),
    );
    print_criterion(
        "0 sandbox escape attempts (90d)",
        result.operational.sandbox_escapes_90d == 0,
        format!("{}", result.operational.sandbox_escapes_90d),
    );
    print_criterion(
        "Failure rate < 5% (30d)",
        result.operational.failure_rate_pct_30d <= result.operational.max_failure_rate_pct,
        format!("{:.2}%", result.operational.failure_rate_pct_30d),
    );
    print_criterion(
        "DPIA valid",
        result.operational.dpia_valid,
        if result.operational.dpia_valid { "valid" } else { "invalid" }.into(),
    );
    print_criterion(
        "Production graduation acknowledgement signed",
        result.operational.graduation_acknowledgement_signed,
        if result.operational.graduation_acknowledgement_signed { "signed" } else { "missing" }.into(),
    );

    println!("");
    println!("Compliance :");
    print_criterion("Audit chain verified", result.compliance.audit_chain_verified, "".into());
    print_criterion("R-100 compliance pack OK", result.compliance.r100_pack_succeeded, "".into());
    print_criterion("DPO review (current quarter)", result.compliance.dpo_review_current_quarter, "".into());
    print_criterion("AI Act Art. 26 post-market submitted", result.compliance.ai_act_post_market_submitted, "".into());

    println!("");
    println!("Channel    : {}", result.channel);
    println!("Ready      : {}", if result.ready { "YES" } else { "NO" });
    if !result.blocking_items.is_empty() {
        println!("");
        println!("Blocking items :");
        for b in &result.blocking_items {
            println!("  - {}", b);
        }
    }
    Ok(())
}

fn print_criterion(label: &str, ok: bool, detail: String) {
    let mark = if ok { "✓" } else { "✗" };
    if detail.is_empty() {
        println!("  {} {}", mark, label);
    } else {
        println!("  {} {} ({})", mark, label, detail);
    }
}

async fn confirm_cmd(
    tenant: String,
    acknowledgement_hash: String,
    confirm: bool,
    ctx: &AdminContext,
) -> Result<(), AdminError> {
    if !confirm {
        return Err(AdminError::InvalidInput(
            "must pass --confirm to graduate (irreversible without revoke)".into(),
        ));
    }
    if !acknowledgement_hash.starts_with("sha256:") || acknowledgement_hash.len() != 71 {
        return Err(AdminError::InvalidInput(
            "acknowledgement_hash must be 'sha256:<64-hex>'".into(),
        ));
    }
    let client = crate::grpc::agent_client::AgentClient::connect(ctx).await?;
    let res = client.graduate_confirm(&tenant, &acknowledgement_hash).await?;
    println!("✓ Tenant '{}' graduated to STABLE channel", tenant);
    println!("  graduated_at         : {}", res.graduated_at);
    println!("  acknowledgement hash : {}", res.acknowledgement_hash);
    println!("  channel              : STABLE");
    println!("");
    println!("Next steps :");
    println!("  - The first 90 days post-graduation are free for early adopters.");
    println!("  - DPIA must be re-signed annually : `efe-admin agent dpia refresh --tenant {} --valid-days 365 --confirm`", tenant);
    Ok(())
}

async fn revoke(
    tenant: String,
    reason: String,
    confirm: bool,
    ctx: &AdminContext,
) -> Result<(), AdminError> {
    if !confirm {
        return Err(AdminError::InvalidInput(
            "must pass --confirm to revoke graduation".into(),
        ));
    }
    if reason.trim().len() < 10 {
        return Err(AdminError::InvalidInput(
            "reason must be ≥ 10 chars".into(),
        ));
    }
    let client = crate::grpc::agent_client::AgentClient::connect(ctx).await?;
    let res = client.graduate_revoke(&tenant, &reason).await?;
    println!("✓ Tenant '{}' graduation revoked", tenant);
    println!("  revoked_at  : {}", res.revoked_at);
    println!("  channel     : EXPERIMENTAL");
    println!("");
    println!("Note : in-flight runs continue with the rules they started with ;");
    println!("       new runs will use EXPERIMENTAL admission rules.");
    Ok(())
}

async fn status(tenant: String, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::agent_client::AgentClient::connect(ctx).await?;
    let s = client.graduate_status(&tenant).await?;
    println!("Tenant : {}", tenant);
    println!("Channel : {}", s.channel);
    if let Some(at) = s.graduated_at {
        println!("Graduated at  : {}", at);
    }
    if let Some(at) = s.revoked_at {
        println!("Revoked at    : {}", at);
    }
    if let Some(at) = s.dpia_expires_at {
        println!("DPIA expires  : {} ({} days)", at, (at - chrono::Utc::now()).num_days());
    }
    Ok(())
}

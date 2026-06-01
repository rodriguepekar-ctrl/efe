//! `efe-admin smoke-test <tenant>` — post-deploy validation.
//!
//! Steps :
//!   1. health check via mcp tool
//!   2. classify N smoke sentences (10 by default) via efe_get_tenant_health
//!   3. fetch last N audit rows
//!   4. verify HMAC chain unbroken
//!
//! Output : pass/fail summary + p99 latency.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cli::SmokeArgs;
use crate::context::AdminContext;
use crate::error::AdminError;
use crate::validation;

const TOOL_HEALTH: &str = "efe_get_tenant_health";
const TOOL_CLASSIFY: &str = "efe_classify";
const TOOL_VERIFY_CHAIN: &str = "efe_audit_health_check";

const SMOKE_SENTENCES: &[&str] = &[
    "facturation tardive sur la dernière commande",
    "changement d'adresse postale demandé",
    "demande d'annulation d'un contrat",
    "réclamation sur un produit endommagé",
    "demande de remboursement pour un service non rendu",
    "demande de facture pour les achats du mois",
    "modification du moyen de paiement",
    "demande d'information sur les conditions générales",
    "résiliation immédiate du contrat",
    "problème technique sur le compte client",
];

#[derive(Debug, Serialize, Deserialize)]
struct SmokeReport {
    tenant_id: String,
    health_ok: bool,
    classify_pass: u32,
    classify_fail: u32,
    p50_ms: u64,
    p99_ms: u64,
    audit_chain_ok: bool,
    summary: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClassifyOut {
    label: Option<String>,
    confidence: Option<f32>,
}

pub async fn run(a: SmokeArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    eprintln!("🧪 smoke-test for tenant '{}'", a.tenant);

    // 1. Health check
    let health = ctx
        .mcp()
        .call_tool(TOOL_HEALTH, json!({"tenant_id": a.tenant}))
        .await?;
    let health_ok = health.get("status").and_then(|s| s.as_str()) == Some("healthy");
    eprintln!("  └─ health: {}", if health_ok { "✅" } else { "❌" });

    // 2. Classify
    let mut latencies = Vec::with_capacity(SMOKE_SENTENCES.len());
    let mut pass = 0_u32;
    let mut fail = 0_u32;
    for sentence in SMOKE_SENTENCES {
        let t0 = Instant::now();
        let resp = ctx
            .mcp()
            .call_tool(
                TOOL_CLASSIFY,
                json!({
                    "tenant_id": a.tenant,
                    "catalog": "default",
                    "sentence": sentence,
                }),
            )
            .await;
        let elapsed_ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
        latencies.push(elapsed_ms);
        match resp {
            Ok(v) => {
                let parsed: ClassifyOut = serde_json::from_value(v).unwrap_or(ClassifyOut {
                    label: None,
                    confidence: None,
                });
                if parsed.label.is_some() {
                    pass += 1;
                } else {
                    fail += 1;
                }
            }
            Err(_) => fail += 1,
        }
    }
    eprintln!("  └─ classify: {pass}/{} pass", SMOKE_SENTENCES.len());

    let (p50, p99) = percentiles(&mut latencies);

    // 3. Audit chain
    let chain = ctx
        .mcp()
        .call_tool(TOOL_VERIFY_CHAIN, json!({"tenant_id": a.tenant}))
        .await?;
    let audit_ok = chain.get("intact").and_then(|b| b.as_bool()).unwrap_or(false);
    eprintln!("  └─ audit chain: {}", if audit_ok { "✅" } else { "❌" });

    let summary = if health_ok && fail == 0 && audit_ok {
        format!(
            "✅ smoke OK — {pass}/{} classify, p99={p99}ms",
            SMOKE_SENTENCES.len()
        )
    } else {
        format!("❌ smoke FAILED — health={health_ok} fail={fail} chain={audit_ok}")
    };
    eprintln!("{summary}");

    let report = SmokeReport {
        tenant_id: a.tenant.clone(),
        health_ok,
        classify_pass: pass,
        classify_fail: fail,
        p50_ms: p50,
        p99_ms: p99,
        audit_chain_ok: audit_ok,
        summary,
    };
    ctx.renderer.print(&report)?;

    if !health_ok || fail > 0 || !audit_ok {
        return Err(AdminError::backend("smoke_failed", "smoke test had failures"));
    }
    Ok(())
}

fn percentiles(latencies: &mut Vec<u64>) -> (u64, u64) {
    if latencies.is_empty() {
        return (0, 0);
    }
    latencies.sort_unstable();
    let p50_idx = latencies.len() / 2;
    let p99_idx = (latencies.len() * 99 / 100).min(latencies.len() - 1);
    (latencies[p50_idx], latencies[p99_idx])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_handle_empty() {
        let mut v: Vec<u64> = vec![];
        assert_eq!(percentiles(&mut v), (0, 0));
    }

    #[test]
    fn percentiles_simple() {
        let mut v: Vec<u64> = (1..=100).collect();
        let (p50, p99) = percentiles(&mut v);
        assert_eq!(p50, 51);
        assert_eq!(p99, 100);
    }

    #[test]
    fn percentiles_single_element() {
        let mut v = vec![42];
        let (p50, p99) = percentiles(&mut v);
        assert_eq!(p50, 42);
        assert_eq!(p99, 42);
    }
}

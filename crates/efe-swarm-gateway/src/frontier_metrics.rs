// SPDX-License-Identifier: UNLICENSED
//! **Couche 2 (M4 §2.4)** — métriques d'**egress frontière** pour `/metrics`. Le chemin
//! `/v1/llm/*` est le SEUL chemin **PAYANT** et **NON-souverain** : sans ces compteurs, un
//! opérateur pilote la dépense **à l'aveugle** (impossible d'alerter sur un pic de coût ou une
//! fuite d'egress). Alimentés au **choke-point** unique de scellement de mission
//! ([`crate::mission_graph::seal_and_store_mission_with_request`]) depuis les records drainés
//! — un [`efe_nano_sdk::LlmProvenanceRecord`] par appel : `compute_attestation.is_remote()` =
//! barreau **R4** (egress distant réel), `cost_accounting.cost_cents` = coût (×1000 ⟹ µ¢).
//!
//! **Cardinalité** : les totaux sont lock-free (`AtomicU64`) ; le par-tenant vit dans un
//! `Mutex<HashMap>` borné — la clé tenant est un **UUID validé** ([`crate::state::SwarmGatewayState::tenant_id`]),
//! jamais une chaîne attaquant-contrôlée. Section critique courte, jamais tenue à travers un `await`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Registre des métriques d'egress frontière (partagé via `Arc`, alimenté au scellement).
#[derive(Debug, Default)]
pub struct FrontierMetrics {
    calls_total: AtomicU64,
    remote_calls_total: AtomicU64,
    cost_microcents_total: AtomicU64,
    /// Dépense cumulée **par tenant** (µ¢). Clé = UUID tenant validé ⟹ cardinalité bornée.
    cost_by_tenant: Mutex<HashMap<String, u64>>,
}

impl FrontierMetrics {
    /// Registre neuf, prêt à partager.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Enregistre une mission frontière **scellée** : `calls` appels dont `remote` distants
    /// (R4), `microcents` de coût egress cumulé. Compteurs monotones (jamais décrémentés).
    pub fn observe_mission(&self, tenant: &str, calls: u64, remote: u64, microcents: u64) {
        self.calls_total.fetch_add(calls, Ordering::Relaxed);
        self.remote_calls_total.fetch_add(remote, Ordering::Relaxed);
        self.cost_microcents_total.fetch_add(microcents, Ordering::Relaxed);
        if microcents > 0 {
            if let Ok(mut m) = self.cost_by_tenant.lock() {
                *m.entry(tenant.to_string()).or_insert(0) += microcents;
            }
        }
    }

    /// Rendu Prometheus (texte 0.0.4). Snapshot du par-tenant **sous lock puis relâché** avant
    /// le formatage (jamais de lock tenu pendant l'écriture de la `String`).
    #[must_use]
    pub fn render_prometheus(&self) -> String {
        let calls = self.calls_total.load(Ordering::Relaxed);
        let remote = self.remote_calls_total.load(Ordering::Relaxed);
        let micro = self.cost_microcents_total.load(Ordering::Relaxed);
        let per_tenant: Vec<(String, u64)> =
            self.cost_by_tenant.lock().map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect()).unwrap_or_default();

        let mut out = format!(
            "# HELP efe_llm_frontier_calls_total Appels LLM frontière scellés (toutes missions /v1/llm).\n\
             # TYPE efe_llm_frontier_calls_total counter\n\
             efe_llm_frontier_calls_total {calls}\n\
             # HELP efe_llm_frontier_remote_calls_total Appels frontière à barreau R4 (egress distant réel).\n\
             # TYPE efe_llm_frontier_remote_calls_total counter\n\
             efe_llm_frontier_remote_calls_total {remote}\n\
             # HELP efe_llm_frontier_cost_microcents_total Coût egress frontière cumulé (1 µ¢ = 1/1000 ¢).\n\
             # TYPE efe_llm_frontier_cost_microcents_total counter\n\
             efe_llm_frontier_cost_microcents_total {micro}\n"
        );
        if !per_tenant.is_empty() {
            out.push_str(
                "# HELP efe_llm_frontier_cost_by_tenant_microcents_total Dépense egress par tenant (µ¢).\n\
                 # TYPE efe_llm_frontier_cost_by_tenant_microcents_total counter\n",
            );
            for (tenant, cost) in per_tenant {
                out.push_str(&format!(
                    "efe_llm_frontier_cost_by_tenant_microcents_total{{tenant=\"{}\"}} {cost}\n",
                    escape_label(&tenant)
                ));
            }
        }
        out
    }
}

/// Échappe une valeur de label Prometheus (`\`, `"`, saut de ligne) — robuste même si la clé
/// tenant cessait d'être un UUID sûr.
fn escape_label(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_accumulates_globals_and_per_tenant() {
        let m = FrontierMetrics::new();
        let t = "11111111-1111-1111-1111-111111111111";
        m.observe_mission(t, 3, 2, 45_000);
        m.observe_mission(t, 1, 1, 5_000);
        let out = m.render_prometheus();
        assert!(out.contains("efe_llm_frontier_calls_total 4"), "3+1 appels");
        assert!(out.contains("efe_llm_frontier_remote_calls_total 3"), "2+1 distants (R4)");
        assert!(out.contains("efe_llm_frontier_cost_microcents_total 50000"), "45000+5000 µ¢");
        assert!(
            out.contains(&format!("efe_llm_frontier_cost_by_tenant_microcents_total{{tenant=\"{t}\"}} 50000")),
            "dépense par tenant agrégée"
        );
    }

    #[test]
    fn zero_cost_does_not_create_tenant_series() {
        let m = FrontierMetrics::new();
        m.observe_mission("t", 2, 0, 0); // 2 appels locaux, 0 coût
        let out = m.render_prometheus();
        assert!(out.contains("efe_llm_frontier_calls_total 2"));
        assert!(!out.contains("by_tenant"), "coût nul ⟹ pas de série par-tenant (bruit évité)");
    }
}

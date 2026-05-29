// SPDX-License-Identifier: UNLICENSED
//! **R3.S5 — Politiques multi-tenant + registre.**
//!
//! EFE est « multi-tenant par défaut » (cf.
//! `efe_nano_app_coordinator::context` : *« multi-tenancy par défaut dans
//! EFE »*). Pourtant, jusqu'à R3.S4, l'isolation était une **fiction** : les
//! workflows construisaient leur `CallContext` avec
//! `coord.config.default_tenant` (un `Uuid::nil()` global), si bien que toutes
//! les entrées d'audit de tous les clients portaient le même `tenant_id`.
//!
//! R3.S5 livre la **couche de politiques par tenant** :
//!
//! - [`TenantPolicies`] — table de scoring, allow-list de LLM, plafond de coût
//!   µ¢, seuil de circuit — *chaque champ branché sur un point d'enforcement
//!   réel* (cf. architecture §4.1, principe « zéro dead config » hérité de
//!   R3.S4).
//! - [`PolicyRegistry`] — `HashMap<TenantId, TenantPolicies>` immuable après
//!   construction, détenu en `Arc` par le `Coordinator`.
//! - [`resolve`] — résout `(tenant_effectif, Option<&TenantPolicies>)` à partir
//!   du `tenant_id` de la requête + le `default_tenant` global.
//! - [`circuit_key`] — clé de circuit `"{tenant}:{robot}"` (isolation du blast
//!   radius inter-tenant) ou `"{robot}"` seul (compat R3.S2 mono-tenant).
//!
//! ## Sémantique de résolution (sécurité)
//!
//! Pour les champs **sécuritaires** (`allowed_llms`, `max_cost`), la politique
//! tenant est un **plafond dur** : une requête ne peut JAMAIS élargir au-delà
//! de sa policy. Encodé par [`TenantPolicies::filter_llms`] (intersection,
//! jamais union) et [`TenantPolicies::cap_microcents`] (`min`).
//!
//! Le champ `retry_policy` listé dans la roadmap est **volontairement absent** :
//! aucune infrastructure de retry/backoff n'existe à paramétrer (la « fallback
//! chain S8 » itère `available_llms` sans backoff). L'inclure serait du faux
//! contrôle — cf. architecture §4.3.

use std::collections::HashMap;
use std::sync::Arc;

use efe_nano_app_coordinator::types::TaskClassScoringTable;
use efe_nano_app_coordinator::TenantId;

/// Seuil d'échecs par défaut d'un circuit SLM (aligné R3.S2
/// `CoordinatorClients::default_slm_circuits` = 3 échecs consécutifs).
pub const DEFAULT_SLM_CIRCUIT_THRESHOLD: u32 = 3;

/// Politiques d'un tenant.
///
/// Tous les champs ont un point d'enforcement réel dans le pipeline (cf.
/// architecture §4.1) :
///
/// | Champ | Mission A | Mission B | Speculative |
/// |---|---|---|---|
/// | `scoring_table` | override router | N/A (pas de router) | N/A |
/// | `allowed_llms` | filtre `available_llms` | filtre `llms` | valide drafter/verifier |
/// | `max_cost_microcents_per_query` | resserre `budget` | best-effort (trace) | resserre `budget` |
/// | `slm_circuit_threshold` | circuit per-tenant | idem | (drafter circuit séparé) |
#[derive(Debug, Clone)]
pub struct TenantPolicies {
    /// Identité (= clé dans le registre, dupliquée pour rendre la structure
    /// self-contained dans l'audit).
    pub tenant_id: TenantId,

    /// Table de scoring tenant-spécifique (override router Mission A).
    ///
    /// `None` ⇒ router global / table fournie par la requête. La priorité de
    /// résolution (requête > tenant > global) est appliquée dans
    /// `run_mission_a` (champ non-sécuritaire : la requête gagne).
    pub scoring_table: Option<Arc<TaskClassScoringTable>>,

    /// Allow-list de provider IDs (3 états sémantiques) :
    ///
    /// - `None` ⇒ **aucun filtrage** (tous les LLM permis).
    /// - `Some(vec![…])` ⇒ seuls ces IDs sont permis (intersection avec la
    ///   liste demandée par la requête).
    /// - `Some(vec![])` ⇒ **deny-all explicite** (fail-closed). Une policy
    ///   « vide » ne doit JAMAIS ouvrir par accident — c'est l'invariant de
    ///   sécurité central.
    pub allowed_llms: Option<Vec<String>>,

    /// Plafond **dur** de coût par requête en microcents.
    ///
    /// `None` ⇒ pas de plafond tenant (seul le `Budget` de la requête
    /// s'applique). `Some(c)` ⇒ le budget effectif est `min(req_budget, c)`.
    pub max_cost_microcents_per_query: Option<u64>,

    /// Seuil d'échecs du circuit SLM (per-tenant). Défaut R3.S2 = 3.
    pub slm_circuit_threshold: u32,
}

impl TenantPolicies {
    /// Politique « ouverte » : aucun override, défauts globaux.
    ///
    /// Base idiomatique pour `..` struct-update et pour les tests.
    #[must_use]
    pub fn open(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            scoring_table: None,
            allowed_llms: None,
            max_cost_microcents_per_query: None,
            slm_circuit_threshold: DEFAULT_SLM_CIRCUIT_THRESHOLD,
        }
    }

    /// Résout l'allow-list effective : **intersection** `requested ∩ allowed`
    /// (jamais union — une requête ne peut élargir au-delà de la policy).
    ///
    /// - `allowed_llms == None` ⇒ renvoie `requested` inchangé (pas de filtre).
    /// - `allowed_llms == Some([])` ⇒ renvoie `[]` (deny-all, fail-closed).
    ///
    /// L'ordre de `requested` est préservé (le router/fan-out dépendent de
    /// l'ordre pour leur fallback).
    #[must_use]
    pub fn filter_llms(&self, requested: &[String]) -> Vec<String> {
        match &self.allowed_llms {
            None => requested.to_vec(),
            Some(allow) => requested
                .iter()
                .filter(|llm| allow.iter().any(|a| a == *llm))
                .cloned()
                .collect(),
        }
    }

    /// `true` si `llm` est autorisé par l'allow-list.
    ///
    /// `allowed_llms == None` ⇒ tout est autorisé. `Some([])` ⇒ rien.
    #[must_use]
    pub fn permits_llm(&self, llm: &str) -> bool {
        match &self.allowed_llms {
            None => true,
            Some(allow) => allow.iter().any(|a| a == llm),
        }
    }

    /// Plafond de coût effectif : `min(req_budget_µ¢, policy_cap)` (le plus
    /// strict). Si la policy n'a pas de plafond, renvoie `req_budget_microcents`
    /// inchangé.
    #[must_use]
    pub fn cap_microcents(&self, req_budget_microcents: u64) -> u64 {
        match self.max_cost_microcents_per_query {
            None => req_budget_microcents,
            Some(cap) => req_budget_microcents.min(cap),
        }
    }
}

/// Registre de politiques. Lookup O(1) par [`TenantId`].
///
/// Conçu **immuable après construction** : les politiques sont chargées au boot
/// par le gateway. Le `Coordinator` le détient en `Arc` pour rester `Clone`
/// cheap (aligné sur `clients: Arc<CoordinatorClients>`).
#[derive(Debug, Default, Clone)]
pub struct PolicyRegistry {
    by_tenant: HashMap<TenantId, TenantPolicies>,
}

impl PolicyRegistry {
    /// Registre vide (= déploiement mono-tenant : tous les lookups → `None` ⇒
    /// comportement global, backward-compat R3.S4).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insère/écrase la policy d'un tenant. Builder-style (chaînable).
    #[must_use]
    pub fn with(mut self, p: TenantPolicies) -> Self {
        self.by_tenant.insert(p.tenant_id, p);
        self
    }

    /// Insère/écrase en place (pour construction impérative).
    pub fn insert(&mut self, p: TenantPolicies) {
        self.by_tenant.insert(p.tenant_id, p);
    }

    /// Lookup. `None` ⇒ tenant inconnu ⇒ comportement global (pas d'escalade).
    #[must_use]
    pub fn get(&self, tenant: &TenantId) -> Option<&TenantPolicies> {
        self.by_tenant.get(tenant)
    }

    /// Nombre de tenants enregistrés.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_tenant.len()
    }

    /// `true` si aucun tenant (= mono-tenant global).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_tenant.is_empty()
    }
}

/// Résout le tenant effectif et sa policy.
///
/// - `req_tenant` : `req.tenant_id` (peut être `None`).
/// - `default_tenant` : `coord.config.default_tenant` (fallback global).
///
/// Retourne `(tenant_effectif, Option<&TenantPolicies>)`. Le `tenant_effectif`
/// **remplace** `default_tenant` dans la construction du `CallContext`,
/// mettant fin à la fiction d'isolation (L5-a).
#[must_use]
pub fn resolve(
    registry: &PolicyRegistry,
    req_tenant: Option<TenantId>,
    default_tenant: TenantId,
) -> (TenantId, Option<&TenantPolicies>) {
    let tid = req_tenant.unwrap_or(default_tenant);
    (tid, registry.get(&tid))
}

/// Clé de circuit SLM, tenant-aware.
///
/// - `has_policy == true` ⇒ `"{tenant}:{robot}"` (isolation du blast radius :
///   un tenant bruyant qui ouvre son circuit `analyzer` n'affecte pas les
///   autres tenants).
/// - `has_policy == false` ⇒ `"{robot}"` (sémantique R3.S2 mono-tenant
///   préservée).
#[must_use]
pub fn circuit_key(tenant: TenantId, robot: &str, has_policy: bool) -> String {
    if has_policy {
        format!("{tenant}:{robot}")
    } else {
        robot.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ids() -> Vec<String> {
        vec!["claude".to_string(), "gemini".to_string(), "gpt".to_string()]
    }

    #[test]
    fn policies_open_has_no_overrides() {
        let t = Uuid::now_v7();
        let p = TenantPolicies::open(t);
        assert_eq!(p.tenant_id, t);
        assert!(p.scoring_table.is_none());
        assert!(p.allowed_llms.is_none());
        assert!(p.max_cost_microcents_per_query.is_none());
        assert_eq!(p.slm_circuit_threshold, DEFAULT_SLM_CIRCUIT_THRESHOLD);
    }

    #[test]
    fn filter_llms_intersection_not_union() {
        let p = TenantPolicies {
            allowed_llms: Some(vec!["claude".into(), "gemini".into()]),
            ..TenantPolicies::open(Uuid::now_v7())
        };
        // requested = [claude, gemini, gpt] ∩ allowed = [claude, gemini]
        let got = p.filter_llms(&ids());
        assert_eq!(got, vec!["claude".to_string(), "gemini".to_string()]);
        // jamais d'union : un LLM demandé hors policy disparaît.
        assert!(!got.iter().any(|x| x == "gpt"));
    }

    #[test]
    fn filter_llms_preserves_request_order() {
        let p = TenantPolicies {
            allowed_llms: Some(vec!["gpt".into(), "claude".into(), "gemini".into()]),
            ..TenantPolicies::open(Uuid::now_v7())
        };
        // L'ordre suit `requested`, pas `allowed`.
        let req = vec!["gemini".to_string(), "claude".to_string()];
        assert_eq!(p.filter_llms(&req), req);
    }

    #[test]
    fn filter_llms_none_policy_passthrough() {
        let p = TenantPolicies::open(Uuid::now_v7());
        assert_eq!(p.filter_llms(&ids()), ids());
    }

    #[test]
    fn filter_llms_deny_all_yields_empty() {
        let p = TenantPolicies {
            allowed_llms: Some(vec![]),
            ..TenantPolicies::open(Uuid::now_v7())
        };
        assert!(p.filter_llms(&ids()).is_empty(), "Some([]) = deny-all");
    }

    #[test]
    fn permits_llm_respects_allow_list() {
        let p = TenantPolicies {
            allowed_llms: Some(vec!["gemini".into()]),
            ..TenantPolicies::open(Uuid::now_v7())
        };
        assert!(p.permits_llm("gemini"));
        assert!(!p.permits_llm("claude"));
    }

    #[test]
    fn permits_llm_none_allows_all() {
        let p = TenantPolicies::open(Uuid::now_v7());
        assert!(p.permits_llm("anything"));
    }

    #[test]
    fn cap_microcents_takes_min() {
        let tight = TenantPolicies {
            max_cost_microcents_per_query: Some(500),
            ..TenantPolicies::open(Uuid::now_v7())
        };
        assert_eq!(tight.cap_microcents(2_000), 500, "policy plus stricte gagne");
        assert_eq!(tight.cap_microcents(300), 300, "requête plus stricte gagne");
    }

    #[test]
    fn cap_microcents_none_policy_passthrough() {
        let p = TenantPolicies::open(Uuid::now_v7());
        assert_eq!(p.cap_microcents(12_345), 12_345);
    }

    #[test]
    fn registry_lookup_hit_and_miss() {
        let t1 = Uuid::now_v7();
        let t2 = Uuid::now_v7();
        let reg = PolicyRegistry::new().with(TenantPolicies::open(t1));
        assert_eq!(reg.len(), 1);
        assert!(reg.get(&t1).is_some());
        assert!(reg.get(&t2).is_none());
    }

    #[test]
    fn resolve_uses_request_tenant_when_present() {
        let req_t = Uuid::now_v7();
        let default_t = Uuid::nil();
        let reg = PolicyRegistry::new().with(TenantPolicies::open(req_t));
        let (tid, policy) = resolve(&reg, Some(req_t), default_t);
        assert_eq!(tid, req_t);
        assert!(policy.is_some());
    }

    #[test]
    fn resolve_falls_back_to_default_tenant() {
        let default_t = Uuid::now_v7();
        let reg = PolicyRegistry::new().with(TenantPolicies::open(default_t));
        // req_tenant = None → on retombe sur default_tenant.
        let (tid, policy) = resolve(&reg, None, default_t);
        assert_eq!(tid, default_t);
        assert!(policy.is_some());
    }

    #[test]
    fn resolve_unknown_tenant_yields_no_policy() {
        let reg = PolicyRegistry::new();
        let (tid, policy) = resolve(&reg, Some(Uuid::now_v7()), Uuid::nil());
        assert!(policy.is_none(), "tenant inconnu → comportement global");
        assert_ne!(tid, Uuid::nil());
    }

    #[test]
    fn circuit_key_tenant_aware_vs_shared() {
        let t = Uuid::now_v7();
        assert_eq!(circuit_key(t, "analyzer", false), "analyzer");
        assert_eq!(circuit_key(t, "analyzer", true), format!("{t}:analyzer"));
    }
}

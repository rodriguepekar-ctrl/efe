// SPDX-License-Identifier: UNLICENSED
//! **L'isolation tenant, structurelle.** Pas un *contrôle* qu'on pourrait oublier : une **propriété de
//! construction** (cf. l'architecture §7). Il n'existe **aucune surface de requête globale** — chaque
//! requête est résolue vers **exactement un** [`AuditService`] (un store, une clé). Le tenant vient du
//! **contexte d'auth** ; le routeur exige que le `{t}` du chemin **égale** ce tenant authentifié (le
//! chemin *nomme*, il ne *choisit* pas). Défense en profondeur : même un record mal-routé d'un autre
//! tenant **ne re-prouverait pas** (sceau clé-tenant — propriété de S0).

use crate::auth::{AuthContext, TenantId};
use crate::project::ProjectionConfig;
use crate::service::AuditService;
use provenance_dag::TenantKey;
use std::collections::HashMap;

/// Erreur de résolution de tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Le tenant authentifié n'a pas de service provisionné ici.
    UnknownTenant(String),
}
impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::UnknownTenant(t) => write!(f, "tenant inconnu : {t}"),
        }
    }
}
impl std::error::Error for RegistryError {}

/// Le registre des services par tenant. La seule porte d'accès aux données : [`Self::resolve`].
#[derive(Default)]
pub struct TenantRegistry {
    services: HashMap<String, AuditService>,
}

impl TenantRegistry {
    pub fn new() -> Self {
        TenantRegistry { services: HashMap::new() }
    }

    /// Provisionne (ou remplace) le service d'un tenant. En déploiement souverain, chaque tenant a son
    /// périmètre ; ce registre modélise le routage **vers** le bon store.
    pub fn provision(&mut self, tenant: &str, key: TenantKey, config: ProjectionConfig) {
        self.services.insert(tenant.to_string(), AuditService::new(key, config));
    }

    /// Insère un service déjà construit (utile pour brancher la capture S1 avant d'enregistrer).
    pub fn insert(&mut self, tenant: &str, service: AuditService) {
        self.services.insert(tenant.to_string(), service);
    }

    /// **La seule porte d'accès** : résout le service du tenant **authentifié**. Il n'existe aucun moyen
    /// d'obtenir le service d'un autre tenant que celui de l'`AuthContext` — l'isolation est structurelle.
    pub fn resolve(&mut self, auth: &AuthContext) -> Result<&mut AuditService, RegistryError> {
        self.services
            .get_mut(auth.tenant.as_str())
            .ok_or_else(|| RegistryError::UnknownTenant(auth.tenant.as_str().to_string()))
    }

    /// Le tenant est-il provisionné ?
    pub fn has(&self, tenant: &TenantId) -> bool {
        self.services.contains_key(tenant.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use provenance_capture::LlmCall;

    fn reg() -> TenantRegistry {
        let mut r = TenantRegistry::new();
        r.provision("acme", TenantKey::derive(b"root", "acme"), ProjectionConfig::default());
        r.provision("globex", TenantKey::derive(b"root", "globex"), ProjectionConfig::default());
        // un cas chez ACME seulement.
        let acme = r.services.get_mut("acme").unwrap();
        acme.ingest_llm(&LlmCall { case_id: "A", step: "s", agent_role: "sanction_check", inputs: b"i", model: "m", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
        r
    }

    #[test]
    fn resolves_only_the_authenticated_tenant() {
        let mut r = reg();
        let acme = AuthContext::new("acme", Role::ComplianceOfficer, "o");
        // ACME voit son cas.
        assert!(r.resolve(&acme).unwrap().case_view("A", &acme, 1).is_ok());
        // GLOBEX ne voit RIEN de ACME — son service est un store distinct et vide.
        let glob = AuthContext::new("globex", Role::ComplianceOfficer, "o");
        let glob_svc = r.resolve(&glob).unwrap();
        assert!(glob_svc.case_view("A", &glob, 1).is_err(), "le cas de ACME est invisible chez GLOBEX");
        assert_eq!(glob_svc.store().records().len(), 0, "stores disjoints");
    }

    #[test]
    fn unknown_tenant_is_rejected() {
        let mut r = reg();
        let ghost = AuthContext::new("ghost", Role::Admin, "o");
        assert!(matches!(r.resolve(&ghost), Err(RegistryError::UnknownTenant(_))));
    }

    #[test]
    fn cross_tenant_record_does_not_reprove() {
        // Défense en profondeur : un record de GLOBEX re-prouvé avec la clé d'ACME échoue (sceau clé-tenant).
        use provenance_dag::reprove_dag;
        let r = reg();
        let acme_key = TenantKey::derive(b"root", "acme");
        let globex_key = TenantKey::derive(b"root", "globex");
        let mut globex_store = provenance_capture::CaptureStore::new(globex_key);
        globex_store.record_llm(&LlmCall { case_id: "B", step: "s", agent_role: "r", inputs: b"i", model: "m", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
        let v = reprove_dag(globex_store.records(), &acme_key);
        assert!(!v.ok, "record d'un autre tenant ne re-prouve pas avec la mauvaise clé");
        let _ = r;
    }
}

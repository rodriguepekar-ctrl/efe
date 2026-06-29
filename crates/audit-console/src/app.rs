// SPDX-License-Identifier: UNLICENSED
//! **L'état de la console** — le registre des tenants (S2), les **clés tenant** (détenues par le backend,
//! pour piloter l'export), et les sessions. Point de souveraineté **central** : la clé vit **ici, dans le
//! backend**, à l'intérieur de la frontière du tenant — elle ne traverse **jamais** vers le navigateur
//! (aucune réponse HTTP ne la contient). Le backend en a besoin pour l'**export** (S4 `assemble` et S6
//! `derive` re-prouvent), pas la console ; c'est cohérent avec « la clé re-prouve, elle ne s'écrit pas ».

use crate::session::Sessions;
use audit_api::{AuthContext, ProjectionConfig, Role, TenantRegistry};
use provenance_dag::{Record, TenantKey};
use std::collections::HashMap;

/// L'état applicatif de la console (un par processus backend, dans la frontière du tenant).
pub struct Console {
    reg: TenantRegistry,
    /// Les clés tenant — **jamais** sérialisées vers le navigateur ; utilisées pour piloter l'export.
    keys: HashMap<String, TenantKey>,
    sessions: Sessions,
    config: ProjectionConfig,
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

impl Console {
    pub fn new() -> Self {
        Console { reg: TenantRegistry::new(), keys: HashMap::new(), sessions: Sessions::new(), config: ProjectionConfig::default() }
    }

    /// Provisionne un tenant : le registre S2 **et** la clé détenue par le backend (pour l'export).
    pub fn provision(&mut self, tenant: &str, key: TenantKey) {
        self.reg.provision(tenant, key.clone(), self.config.clone());
        self.keys.insert(tenant.to_string(), key);
    }

    /// Réhydrate un tenant depuis des records déjà scellés (redémarrage / log persisté) + la clé.
    pub fn rehydrate(&mut self, tenant: &str, key: TenantKey, records: Vec<Record>) {
        self.reg.rehydrate(tenant, key.clone(), records, self.config.clone());
        self.keys.insert(tenant.to_string(), key);
    }

    /// Enregistre une session (jeton → tenant/rôle/sujet). L'authentification réelle est en amont.
    pub fn add_session(&mut self, token: &str, tenant: &str, role: Role, subject: &str) {
        self.sessions.insert(token, tenant, role, subject);
    }

    pub fn sessions(&self) -> &Sessions {
        &self.sessions
    }

    /// Accès mutable au registre S2 (la console l'interroge via le routeur / les services — cf. handler).
    pub fn registry_mut(&mut self) -> &mut TenantRegistry {
        &mut self.reg
    }

    /// La clé d'un tenant (pour l'export — backend uniquement, jamais servie).
    pub fn key(&self, tenant: &str) -> Option<&TenantKey> {
        self.keys.get(tenant)
    }

    /// Ingestion de capture (S1) — exposée pour le `main` de démo / les tests, pas une route HTTP.
    /// (La capture réelle est alimentée par le runtime ; la console ne capte pas.)
    pub fn resolve_service(&mut self, auth: &AuthContext) -> Option<&mut audit_api::AuditService> {
        self.reg.resolve(auth).ok()
    }
}

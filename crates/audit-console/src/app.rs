// SPDX-License-Identifier: UNLICENSED
//! **L'état de la console** — le registre des tenants (S2), les **clés tenant** (détenues par le backend,
//! pour piloter l'export), et les sessions. Point de souveraineté **central** : la clé vit **ici, dans le
//! backend**, à l'intérieur de la frontière du tenant — elle ne traverse **jamais** vers le navigateur
//! (aucune réponse HTTP ne la contient). Le backend en a besoin pour l'**export** (S4 `assemble` et S6
//! `derive` re-prouvent), pas la console ; c'est cohérent avec « la clé re-prouve, elle ne s'écrit pas ».

use crate::canal::Plancher;
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
    /// **M8** — le plancher de canal exigé pour **sceller** un acte humain. Défaut : boucle locale,
    /// c'est-à-dire exactement le comportement d'avant M8 (`assert_local` interdisait tout bind
    /// distant). Lire n'est pas concerné : lire sur un canal faible perd de la confidentialité,
    /// sceller y inscrit une attestation non répudiable que rien ne soutient.
    plancher: Plancher,
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

impl Console {
    pub fn new() -> Self {
        Console {
            reg: TenantRegistry::new(),
            keys: HashMap::new(),
            sessions: Sessions::new(),
            config: ProjectionConfig::default(),
            plancher: Plancher::default(),
        }
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

    /// **M7** — le service est-il en état de servir quelqu'un ?
    ///
    /// Un tenant sans session est illisible ; une session sans tenant ne résout rien. Il faut les
    /// deux, et c'est exactement ce qu'un montage de configuration manquant fait rater — donc
    /// exactement ce qu'un redémarrage peut réparer. C'est le critère de [`crate::handler`] pour
    /// `/readyz`, et rien de plus.
    #[must_use]
    pub fn est_provisionne(&self) -> bool {
        !self.keys.is_empty() && !self.sessions.is_empty()
    }

    /// **M8** — abaisse le plancher d'écriture à A2, pour un déploiement qui **déclare** un
    /// terminateur de canal. Il n'existe aucun moyen de descendre plus bas (cf. [`Plancher`]).
    pub fn avec_plancher(&mut self, p: Plancher) {
        self.plancher = p;
    }

    /// Le plancher exigé pour sceller un acte humain.
    #[must_use]
    pub fn plancher(&self) -> Plancher {
        self.plancher
    }

    /// Les sessions, en écriture — la connexion (`POST /login`) y pose la session d'un porteur.
    pub fn sessions_mut(&mut self) -> &mut Sessions {
        &mut self.sessions
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

    /// Le service d'un tenant, par son identifiant.
    ///
    /// Fabrique un contexte interne : `TenantRegistry::resolve` ne lit que `auth.tenant`, et faire
    /// écrire ce détail à chaque appelant multiplierait les endroits où un rôle est choisi au
    /// hasard. Réservé au câblage (provisionnement, balayage de conservation) — jamais une route.
    pub fn service_de(&mut self, tenant: &str) -> Option<&mut audit_api::AuditService> {
        let auth = AuthContext::new(tenant, Role::Admin, "scella:cablage");
        self.reg.resolve(&auth).ok()
    }

    /// Ingestion de capture (S1) — exposée pour le `main` de démo / les tests, pas une route HTTP.
    /// (La capture réelle est alimentée par le runtime ; la console ne capte pas.)
    pub fn resolve_service(&mut self, auth: &AuthContext) -> Option<&mut audit_api::AuditService> {
        self.reg.resolve(auth).ok()
    }
}

// SPDX-License-Identifier: UNLICENSED
//! **Identité, rôles & autorisation.** S2 enforce l'**autorisation** (qui peut quoi) ; l'**authentification**
//! (prouver l'identité) est le travail de la coquille de transport / passerelle en amont, qui construit un
//! [`AuthContext`] *déjà vérifié*. S2 fait confiance à ce contexte comme à l'identité authentifiée, et
//! tranche le reste **côté Rust** — jamais dans la vue (cf. l'architecture §8.1).
//!
//! La **rédaction** (masquer `output`/`justification` selon le rôle) est une **opération de vue** : elle
//! n'altère JAMAIS le record scellé (le sceau couvre toujours le champ entier ; un export autorisé le
//! porte intact) — on **choisit ce qu'on projette**, on ne « caviarde » pas le sceau (§8.2).

use serde::Serialize;

/// L'identifiant d'un tenant — la frontière d'isolation (un store, une clé, un contexte par tenant).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TenantId(pub String);
impl TenantId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Les rôles, du moins au plus privilégié.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Lecture seule, **données sensibles rédigées** (output/justification masqués).
    Viewer,
    /// L'agent de conformité : lit en clair, agit (approuve/outrepasse), exporte.
    ComplianceOfficer,
    /// L'auditeur externe : lit en clair, exporte (mais n'agit pas — il atteste, il ne décide pas).
    Auditor,
    /// Administration du tenant.
    Admin,
}
impl Role {
    /// Tout rôle authentifié peut lire (la rédaction module *quoi*, §8.2).
    pub fn can_view(&self) -> bool {
        true
    }
    /// Voir les champs sensibles en clair (sinon rédigés).
    pub fn sees_unredacted(&self) -> bool {
        !matches!(self, Role::Viewer)
    }
    /// Sceller un acte de supervision (approve/override/escalate). L'auditeur **n'agit pas**.
    pub fn can_act(&self) -> bool {
        matches!(self, Role::ComplianceOfficer | Role::Admin)
    }
    /// Produire un export forensic (le bundle remis à un tiers).
    pub fn can_export(&self) -> bool {
        matches!(self, Role::ComplianceOfficer | Role::Auditor | Role::Admin)
    }
}

/// Le contexte d'auth **déjà vérifié** fourni par la coquille de transport.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Le tenant authentifié — la SEULE frontière de données accessible (cf. [`crate::registry`]).
    pub tenant: TenantId,
    pub role: Role,
    /// L'identité de l'acteur (pour sceller les actes humains comme `human_id`).
    pub subject: String,
}
impl AuthContext {
    pub fn new(tenant: &str, role: Role, subject: &str) -> Self {
        AuthContext { tenant: TenantId(tenant.to_string()), role, subject: subject.to_string() }
    }
}

/// Le marqueur d'un champ rédigé (rendu à la place du contenu sensible pour un rôle non autorisé).
pub const REDACTED: &str = "[rédigé]";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_capabilities() {
        assert!(Role::Viewer.can_view() && !Role::Viewer.sees_unredacted());
        assert!(!Role::Viewer.can_act() && !Role::Viewer.can_export());
        assert!(Role::ComplianceOfficer.can_act() && Role::ComplianceOfficer.can_export());
        assert!(Role::ComplianceOfficer.sees_unredacted());
        // l'auditeur atteste mais N'AGIT PAS.
        assert!(!Role::Auditor.can_act(), "l'auditeur ne scelle pas d'acte humain");
        assert!(Role::Auditor.can_export() && Role::Auditor.sees_unredacted());
        assert!(Role::Admin.can_act() && Role::Admin.can_export());
    }
}

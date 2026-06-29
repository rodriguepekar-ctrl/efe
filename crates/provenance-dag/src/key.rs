// SPDX-License-Identifier: UNLICENSED
//! **La clé tenant** — l'ancre de souveraineté.
//!
//! Le sceau de chaque maillon du DAG est `HMAC(K_tenant, …)`. `K_tenant` est **dérivée par client** à
//! partir d'un secret racine et de l'identifiant tenant. Le **client détient sa clé** : lui seul (ou
//! l'auditeur à qui il la confie) peut re-prouver — la confiance n'exige PAS de faire confiance à
//! l'éditeur. C'est le pendant code du garde-fou de souveraineté EFE (`HmacKeySource ∘ TenantId`).

use crate::hmac::hmac_sha256;

/// Le contexte de dérivation (versionné — un changement de version change toutes les clés, par dessein).
const DERIVE_CTX: &[u8] = b"efe-prov-dag:v1:tenant:";

/// Une clé tenant de 32 octets. `Debug` est **rédigé** : la clé ne fuit jamais dans les logs.
#[derive(Clone, PartialEq, Eq)]
pub struct TenantKey([u8; 32]);

impl TenantKey {
    /// Dérive la clé d'un tenant : `K = HMAC(root_secret, "efe-prov-dag:v1:tenant:" ‖ tenant_id)`.
    /// Déterministe → re-dérivable à l'identique (re-prouvabilité).
    pub fn derive(root_secret: &[u8], tenant_id: &str) -> TenantKey {
        let mut ctx = Vec::with_capacity(DERIVE_CTX.len() + tenant_id.len());
        ctx.extend_from_slice(DERIVE_CTX);
        ctx.extend_from_slice(tenant_id.as_bytes());
        TenantKey(hmac_sha256(root_secret, &ctx))
    }

    /// Reconstruit une clé depuis ses 32 octets bruts (ex. l'auditeur recharge la clé qu'on lui a remise).
    pub fn from_bytes(bytes: [u8; 32]) -> TenantKey {
        TenantKey(bytes)
    }

    /// Les octets de la clé (pour le scellement/la re-preuve). `pub(crate)` : ne fuit pas hors de la crate.
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for TenantKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Empreinte courte (8 hex) pour le diagnostic, JAMAIS la clé entière.
        let fp: String = self.0.iter().take(4).map(|b| format!("{b:02x}")).collect();
        write!(f, "TenantKey(…{fp}…[rédigée])")
    }
}

#[cfg(test)]
mod tests {
    use super::TenantKey;

    #[test]
    fn derivation_is_deterministic_and_per_tenant() {
        let root = b"root-secret-editeur";
        let a1 = TenantKey::derive(root, "acme-corp");
        let a2 = TenantKey::derive(root, "acme-corp");
        let b = TenantKey::derive(root, "globex");
        assert_eq!(a1, a2, "même (root,tenant) → même clé (re-dérivable)");
        assert_ne!(a1, b, "tenants distincts → clés distinctes (isolation)");
        // round-trip d'octets (ce que l'auditeur recharge).
        assert_eq!(a1, TenantKey::from_bytes(*a1.as_bytes()));
    }

    #[test]
    fn debug_redacts_the_key() {
        let k = TenantKey::derive(b"r", "t");
        let s = format!("{k:?}");
        assert!(s.contains("rédigée"), "Debug ne doit pas exposer la clé : {s}");
        // La représentation ne contient pas l'hex complet de la clé.
        let full: String = k.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert!(!s.contains(&full));
    }
}

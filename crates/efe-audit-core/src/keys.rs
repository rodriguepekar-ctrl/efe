// SPDX-License-Identifier: UNLICENSED
//! # `keys` — dérivation de clés HMAC **par tenant** (HKDF-SHA256).
//!
//! Réplique EXACTE de `efe_nano::audit::hkdf` (paramètres gelés Sprint 6) afin que
//! les clés dérivées soient **byte-identiques** ⇒ chaînes d'essaim et chaînes de
//! trace mutuellement vérifiables avec le même master.
//!
//! - **Hash** : SHA-256 · **Salt** : UUID du tenant (16 o) · **Info** :
//!   `b"efe-nano/audit/v1"` · **Sortie** : 32 o.
//! - **`EnvHkdf`** : lit `EFE_NANO_AUDIT_HMAC_MASTER` (≥ 32 o) — **production**.
//! - **`StaticMaster`** : tests UNIQUEMENT (déterministe).
//! - **`VaultTransit { path }`** : **placeholder Phase 4** → `NotImplemented`. Le
//!   custody réel (HashiCorp Vault Transit) est câblé côté serveur ; **aucune clé
//!   n'est jamais manipulée ici**.

use efe_nano_sdk::TenantId;
use hkdf::Hkdf;
use sha2::Sha256;
use thiserror::Error;

/// Nom de la variable d'environnement du master (côté serveur uniquement).
pub const MASTER_ENV: &str = "EFE_NANO_AUDIT_HMAC_MASTER";

/// Paramètre `info` HKDF (versionné pour rotation) — identique efe-nano.
pub const HKDF_INFO: &[u8] = b"efe-nano/audit/v1";

/// Erreurs de dérivation de clé.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuditKeyError {
    /// Master absent ou trop court, ou échec HKDF.
    #[error("clé d'audit indisponible : {0}")]
    KeyMissing(String),
    /// Source non implémentée (ex. VaultTransit, différé Phase 4 serveur).
    #[error("source de clé non implémentée : {0}")]
    NotImplemented(String),
}

/// Source de clé HMAC par tenant (mirroir de `efe_nano::audit::HmacKeySource`).
#[derive(Debug, Clone)]
pub enum HmacKeySource {
    /// Lit le master depuis `EFE_NANO_AUDIT_HMAC_MASTER` (≥ 32 o). Production.
    EnvHkdf,
    /// Master littéral — **tests uniquement**.
    StaticMaster(Vec<u8>),
    /// Vault Transit (placeholder Phase 4 — câblage serveur).
    VaultTransit {
        /// Chemin Vault, ex. "efe-nano/keys/audit/v1".
        path: String,
    },
}

impl HmacKeySource {
    /// Source de test déterministe (master fixe ≥ 32 o).
    pub fn test_key() -> Self {
        Self::StaticMaster(b"test-master-32-bytes-fixed-aaaaaaaaaaaa".to_vec())
    }

    /// Dérive une clé tenant de 32 octets (HKDF-SHA256).
    pub fn derive(&self, tenant: TenantId) -> Result<Vec<u8>, AuditKeyError> {
        match self {
            Self::EnvHkdf => {
                let master = std::env::var(MASTER_ENV)
                    .map_err(|_| AuditKeyError::KeyMissing(format!("{MASTER_ENV} non défini")))?;
                if master.len() < 32 {
                    return Err(AuditKeyError::KeyMissing(format!(
                        "{MASTER_ENV} trop court : {} octets (≥ 32 requis)",
                        master.len()
                    )));
                }
                derive_with_hkdf(master.as_bytes(), tenant)
            }
            Self::StaticMaster(master) => {
                if master.len() < 16 {
                    return Err(AuditKeyError::KeyMissing(format!(
                        "master statique trop court : {} octets",
                        master.len()
                    )));
                }
                derive_with_hkdf(master, tenant)
            }
            Self::VaultTransit { path } => Err(AuditKeyError::NotImplemented(format!(
                "VaultTransit({path}) différé Phase 4 (custody serveur)"
            ))),
        }
    }
}

/// HKDF-SHA256 → clé tenant de 32 octets. Salt = UUID tenant, info = [`HKDF_INFO`].
fn derive_with_hkdf(master: &[u8], tenant: TenantId) -> Result<Vec<u8>, AuditKeyError> {
    let salt = tenant.as_uuid().as_bytes().to_vec();
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), master);
    let mut out = vec![0u8; 32];
    hkdf.expand(HKDF_INFO, &mut out)
        .map_err(|e| AuditKeyError::KeyMissing(format!("échec HKDF expand : {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        let mut s = String::with_capacity(b.len() * 2);
        for x in b {
            s.push_str(&format!("{x:02x}"));
        }
        s
    }

    #[test]
    fn static_master_derives_deterministically_32_bytes() {
        let src = HmacKeySource::test_key();
        let t = TenantId::system();
        let k1 = src.derive(t).unwrap();
        let k2 = src.derive(t).unwrap();
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32);
    }

    #[test]
    fn different_tenants_get_different_keys() {
        let src = HmacKeySource::test_key();
        assert_ne!(src.derive(TenantId::new()).unwrap(), src.derive(TenantId::new()).unwrap());
    }

    #[test]
    fn static_master_too_short_errors() {
        let src = HmacKeySource::StaticMaster(b"short".to_vec());
        assert!(matches!(src.derive(TenantId::system()), Err(AuditKeyError::KeyMissing(_))));
    }

    #[test]
    fn vault_transit_is_not_implemented() {
        let src = HmacKeySource::VaultTransit { path: "efe-nano/keys/audit/v1".into() };
        assert!(matches!(src.derive(TenantId::system()), Err(AuditKeyError::NotImplemented(_))));
    }

    #[test]
    fn env_hkdf_without_master_does_not_panic() {
        // Selon l'environnement : Ok (master présent) ou Err(KeyMissing). Jamais panique.
        let _ = HmacKeySource::EnvHkdf.derive(TenantId::system());
    }

    #[test]
    fn golden_key_locks_hkdf_construction() {
        // Vecteur FIGÉ : test_key + tenant system → clé HKDF déterministe. Toute dérive
        // des paramètres HKDF (salt/info/hash) casse ce test ET romprait la
        // cross-vérifiabilité avec efe-nano (mêmes paramètres gelés Sprint 6).
        let k = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
        assert_eq!(hex(&k), "4f849020850db9409ad503b209412186e588114247a023ad70b128b0acfb560d");
    }
}

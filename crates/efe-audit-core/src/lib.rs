// SPDX-License-Identifier: UNLICENSED
//! # `efe-audit-core` — primitives d'audit **partagées** (Phase 4).
//!
//! Le socle crypto commun aux chaînes d'audit EFE :
//! - [`hmac`] : HMAC-SHA256 chaîné ([`compute_hmac`] par UUID — compat `efe-nano` ;
//!   [`chain_hmac`] par HMAC précédent — chaîne *hash-linkée* de la provenance
//!   par-swarm dont le tip est le `provenance_root`).
//! - [`keys`] : dérivation de clés **par tenant** (HKDF-SHA256, [`HmacKeySource`]),
//!   byte-identique à `efe_nano::audit::hkdf`.
//!
//! **Pourquoi un crate séparé** : un **seul** endroit où vit la crypto d'audit
//! (anti-duplication, anti-dérive). Réplique fidèlement la construction d'`efe-nano`
//! (golden vectors) ⇒ chaînes mutuellement vérifiables. Un refactor ultérieur pourra
//! rebrancher `efe-nano::audit` sur ce crate sans changer un octet de chaîne.
//!
//! **Sécurité** : aucune clé n'est jamais saisie ici. Le master vient de
//! l'environnement serveur (`EFE_NANO_AUDIT_HMAC_MASTER`) ou de Vault ; les tests
//! utilisent `StaticMaster`.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod hmac;
pub mod keys;

pub use hmac::{chain_hmac, compute_hmac, GENESIS};
pub use keys::{AuditKeyError, HmacKeySource, HKDF_INFO, MASTER_ENV};

/// Version du crate (semver).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

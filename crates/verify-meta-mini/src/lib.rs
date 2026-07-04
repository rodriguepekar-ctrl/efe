// SPDX-License-Identifier: UNLICENSED
//! # `verify-meta-mini` — le re-prouveur de consensus de méta-essaim à **dépendances vides**.
//!
//! Le « verify-mini » de l'étage méta-essaim, poussé jusqu'au bout : **ne croire que les
//! maths**. Il ré-implémente tout — SHA-256 (FIPS 180-4), HMAC-SHA256 (RFC 2104), un
//! parseur JSONL fait main (pas de serde), l'encodage canonique, la chaîne et la
//! re-preuve — de sorte que `cargo tree --edges normal` ne montre que **lui**. Un tiers
//! sceptique le re-compile en quelques secondes, hors-ligne, et re-prouve un bundle de
//! consensus méta (`meta_steps.jsonl` + `meta_manifest.json` + la clé tenant) **sans
//! faire confiance à personne ni à aucune dépendance**.
//!
//! **Fidélité prouvée, pas assertée.** La construction du sceau —
//! `seal_i = HMAC-SHA256(clé, parent‖SHA256(contenu_i))`, `parent = b"genesis"` à la
//! genèse — est **byte-identique** à [`efe_audit_core::chain_hmac`]. C'est prouvé de deux
//! façons (cf. `tests/cross_check.rs`) : (1) des **vecteurs golden gelés** repris
//! d'`efe-audit-core` ; (2) un **cross-check round-trip** — sceller avec `efe-meta-verify`
//! (la référence), sérialiser, puis re-parser + re-prouver ici → concordance octet à
//! octet. `efe-meta-verify` n'est qu'en **dev-dépendance** : le binaire release reste
//! zéro-dépendance.
//!
//! Contrat CLI (identique à `verify_meta`) : `0` intègre · `1` altéré/réordonné/mauvaise
//! clé (faute **localisée**) · `2` usage/E-S.

pub mod hmac;
pub mod json;
pub mod sha256;
pub mod verify;

pub use verify::{verify, Outcome, Step};

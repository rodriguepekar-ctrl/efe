// SPDX-License-Identifier: UNLICENSED
//! # `provenance` — le câblage de la provenance re-prouvable **dans le moteur** (feature `provenance`).
//!
//! Un [`StepObserver`](crate::StepObserver) qui **scelle chaque pas de consensus** au fil de
//! l'itération, dans une chaîne HMAC re-prouvable, avec la crypte **partagée** [`efe_audit_core`]
//! (`chain_hmac` + HKDF par tenant) — byte-identique à `efe-nano` et `efe-swarm-provenance`. La
//! provenance devient une **sortie de première classe** du moteur :
//! [`MetaSwarm::iterate_with_provenance`] rend le résultat **et** un [`MetaProvenance`] (root + pas
//! scellés, **sans clé**), re-prouvable hors-ligne par `verify_meta` (P1.1) et `verify_meta_mini`
//! (P1.2, zéro-dép).
//!
//! - **Non-interférence** : le sink n'observe que le pas **déjà calculé** ; le numérique est identique
//!   avec ou sans provenance (testé — `provenance_is_non_interfering`).
//! - **Souveraineté** : la clé vit dans le sink le temps du run et est **droppée** à la génération du
//!   [`MetaProvenance`] — elle n'entre jamais dans la sortie ni dans le bundle.

use crate::iteration::{ObservedStep, StepObserver};
use crate::{MetaSwarm, MetaSwarmResult, Result};

pub use efe_audit_core::HmacKeySource;
pub use efe_meta_verify::{MetaChainState, MetaProvenance, RunBinding, SealedMetaStep};
pub use efe_nano_sdk::TenantId;

/// L'observateur qui **scelle** chaque pas de consensus dans une chaîne HMAC (crypto partagée).
pub struct HmacProvenanceSink {
    chain: MetaChainState,
}

impl HmacProvenanceSink {
    /// Nouveau sink pour `meta_id`, scellant sous `key` (32 o, dérivée par tenant).
    #[must_use]
    pub fn new(meta_id: &str, key: Vec<u8>) -> Self {
        Self { chain: MetaChainState::new(meta_id, key) }
    }

    /// **v4** — même chose, en liant le run à ses acteurs et à sa règle ([`RunBinding`]).
    ///
    /// La liaison est un invariant de run : elle est posée ici, une fois, et aucun `on_step` ne
    /// peut la contredire — il n'y a pas d'endroit où l'écrire.
    #[must_use]
    pub fn new_bound(meta_id: &str, key: Vec<u8>, binding: RunBinding) -> Self {
        Self { chain: MetaChainState::new_bound(meta_id, key, binding) }
    }

    /// Consomme le sink → l'artefact de provenance (**la clé est droppée**).
    #[must_use]
    pub fn finish(self) -> MetaProvenance {
        self.chain.finish()
    }
}

impl StepObserver for HmacProvenanceSink {
    fn on_step(&mut self, o: &ObservedStep) {
        // Widening f32 → f64 (exact) : serde_json round-trip le f64 bit-à-bit (cf. efe-meta-verify,
        // feature `float_roundtrip`), donc le méta-consensus se re-hache à l'identique côté vérifieur.
        let consensus: Vec<f64> = o.meta_consensus.as_slice().iter().map(|&v| f64::from(v)).collect();
        // v3 : un hash de contenu par contribution de participant → « quel essaim a contribué quoi ».
        let vote_hashes: Vec<String> = o
            .votes
            .iter()
            .map(|v| efe_meta_verify::content_hash_hex(&v.as_slice().iter().map(|&x| f64::from(x)).collect::<Vec<_>>()))
            .collect();
        // v2 : ‖z − z̃‖ (enveloppe) ; v3 : les hashes de contribution — tout scellé dans la chaîne.
        self.chain.push(o.step as u64, o.residual, consensus, Some(o.safety_delta), Some(vote_hashes));
    }
}

impl MetaSwarm {
    /// Itère **en scellant chaque pas** de consensus dans une chaîne HMAC re-prouvable. Rend le
    /// résultat *et* l'artefact [`MetaProvenance`] (root + pas scellés, **sans clé**). La clé est
    /// dérivée de `key_source`/`tenant` au début du run, tenue par le sink, puis droppée.
    ///
    /// # Errors
    /// Erreur si la dérivation de clé échoue (p. ex. master `EnvHkdf` absent) ou si le moteur échoue
    /// (H1 non tenue, etc.) — dans les deux cas, **aucun** artefact à-moitié-scellé n'est produit.
    pub fn iterate_with_provenance(
        &self,
        key_source: &HmacKeySource,
        tenant: TenantId,
        binding: Option<RunBinding>,
    ) -> Result<(MetaSwarmResult, MetaProvenance)> {
        let key = key_source
            .derive(tenant)
            .map_err(|e| crate::MetaSwarmError::Internal(format!("dérivation de la clé de provenance : {e}")))?;
        let mut sink = match binding {
            // v4 : le reçu nomme ses acteurs et sa règle.
            Some(b) => HmacProvenanceSink::new_bound(self.id(), key, b),
            // v3 : trace positionnelle seule — conservé pour que les bundles antérieurs restent
            // re-prouvables à l'identique, pas comme un mode « allégé » qu'on recommanderait.
            None => HmacProvenanceSink::new(self.id(), key),
        };
        let result = self.iterate_observed(&mut sink)?;
        Ok((result, sink.finish()))
    }
}

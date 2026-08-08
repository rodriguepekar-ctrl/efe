// SPDX-License-Identifier: UNLICENSED
//! # `provenance-bridge` — Fil P : joindre le reçu servi au vérifieur auditable
//!
//! Le dépôt possède **deux moitiés de moat qui ne se touchent pas**. La **pile A**
//! (`efe-swarm-provenance` + la gateway) scelle les missions réellement servies, avec un contenu
//! riche : quel modèle a réécrit quoi, quels microcentimes sont sortis, quel hop était local. La
//! **pile B** (`provenance-dag` et sa famille) possède un vérifieur dont l'arbre de dépendances
//! est **littéralement vide** — donc intégralement lisible par un auditeur — et la machinerie de
//! re-preuve contre une racine publiée. Mais la pile B n'a **rien à vérifier** : rien, dans tout le
//! dépôt, ne construit de DAG hors des exemples et des tests.
//!
//! Cette crate est le producteur qui manquait.
//!
//! ## Ce qu'elle fait, en une phrase
//!
//! Elle **dérive** — de façon pure et déterministe — les nœuds typés de la pile B depuis un run
//! déjà scellé de la pile A. Le bundle obtenu n'ajoute **aucune information** au reçu : il en
//! change le **vérifieur**. C'est tout, et c'est précisément ce qui manquait.
//!
//! ## Les trois invariants du fil
//!
//! 1. **Le pont est une vue, jamais une seconde vérité.** Une double écriture (alimenter la pile B
//!    au moment du scellement) créerait deux sources qui peuvent diverger, sans que personne ne le
//!    voie. Une dérivation ne peut pas diverger : elle est une *fonction* de la pile A.
//! 2. **Le sens des dépendances est *pont → piles*, jamais l'inverse.** Ni `efe-swarm-provenance`
//!    ni `provenance-dag` ne dépendent d'ici : aucune des deux ne peut donc être cassée par le
//!    pont, et `verify-mini` conserve son arbre de dépendances vide.
//! 3. **Jamais de bundle partiel.** Toute anomalie est un refus typé ([`DeriveError`]). Un bundle
//!    amputé serait indistinguable d'un bundle tronqué par un attaquant — le fabriquer soi-même
//!    reviendrait à créer l'ambiguïté que le fil cherche à supprimer.
//!
//! ## Ce que cette crate contient, jalon par jalon
//!
//! - **P.1** — [`nodes_from_run`] : la dérivation pure et déterministe.
//! - **P.2** — [`seal`] : le scellement du bundle et la clé de la pile B ([`stack_b_key`], dont
//!   les octets ne sont **jamais** ceux de la pile A).
//! - **P.3** — [`attest`] : l'attestation ed25519 de la racine, vérifiable **sans aucun secret**.
//!
//! Reste devant : **P.4**, l'ancre publiée **hors** du bundle. Tant qu'elle manque, un producteur
//! malveillant peut tronquer un run *puis* signer honnêtement la racine du bundle tronqué — la
//! signature serait valide et le mensonge intact. P.3 authentifie l'**émetteur** ; P.4 fixe
//! l'**attendu**. Ne jamais confondre les deux.
//!
//! Cette crate n'écrit rien nulle part : elle ne fait que dériver, sceller et signer.
//!
//! ## Le tableau des niveaux (rappel — §6 du doc d'architecture)
//!
//! Chaque niveau achète **une** chose, et n'emprunte rien au suivant :
//!
//! | Niveau | Ce qu'un tiers peut conclure | Ce qu'il doit détenir |
//! |---|---|---|
//! | **N1** *(P.0, fait)* | le run n'a pas été tronqué depuis son scellement | la clé tenant A |
//! | **N2** *(P.1-P.2)* | idem, **mais re-prouvé par un vérifieur dont il lit toute la source** | la clé tenant B |
//! | **N3** *(P.3)* | cette racine a bien été émise par le détenteur de la clé | la clé **publique** |
//! | **N4** *(P.4)* | le run existait à la date T ; la troncature est détectable **sans** clé | l'ancre publiée |
//!
//! Et ce qu'aucun niveau ne donne : la **correction**. Le reçu prouve *ce qui a tourné*, jamais que
//! le résultat est vrai. Un modèle honnête-mais-faux produit un bundle parfaitement valide.
//!
//! ## Exemple
//!
//! ```no_run
//! use provenance_bridge::{nodes_from_run, RunView};
//! # fn demo(run_id: uuid::Uuid, records: &[efe_swarm_provenance::AgentCallRecord],
//! #         sealed_root: &str, key: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//! let view = RunView::new(run_id, "acme-corp", records, &[], Some(sealed_root));
//! let nodes = nodes_from_run(&view, key)?; // refuse si la racine ne se reproduit pas
//! # let _ = nodes; Ok(()) }
//! ```
//!
//! Voir `docs/architecture/FIL-P-pont-pile-A-vers-pile-B-ARCHITECTURE.md`.

#![forbid(unsafe_code)]

pub mod derive;
pub mod error;
pub mod mapping;
pub mod attest;
pub mod seal;
pub mod view;

pub use derive::nodes_from_run;
pub use error::DeriveError;
pub use mapping::{HASH_ABSENT, HASH_PREFIX, MAPPING_VERSION};
pub use attest::{
    attest_tuple, key_id_of, verify_attestation, verify_attestation_binds, AttestError,
    BundleSigner, RootAttestation, ATTEST_SCHEMA_V1,
};
pub use seal::{bundle_from_run, seal_bundle, stack_b_key, Bundle, BundleError};
pub use view::RunView;

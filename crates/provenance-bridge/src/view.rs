// SPDX-License-Identifier: UNLICENSED
//! **La vue d'entrée** — ce que le pont exige d'un run, et rien de plus.
//!
//! # Pourquoi une vue plutôt que `StoredRun`
//!
//! Le §4 du document d'architecture annonçait `nodes_from_run(stored: &StoredRun)`. C'était une
//! **erreur**, corrigée ici : `StoredRun` vit dans `efe-swarm-gateway`, et le prendre en entrée
//! ferait dépendre une crate de dérivation **pure** de tout le serveur HTTP (axum, tower, sqlx…) —
//! en plus d'inverser le sens des dépendances que le fil s'impose (*pont → piles, jamais
//! l'inverse*).
//!
//! [`RunView`] est donc le contrat minimal : des emprunts sur ce qui est déjà persisté, sans
//! copie, sans possession. C'est l'appelant — la gateway en P.2 — qui construit la vue depuis son
//! `StoredRun`. Le pont, lui, ignore jusqu'à l'existence d'un serveur.

use efe_nano_sdk::LlmProvenanceRecord;
use efe_swarm_provenance::AgentCallRecord;
use uuid::Uuid;

/// Un run scellé de la pile A, vu par le pont.
///
/// Tous les champs sont des **emprunts** : dériver ne copie pas le run, ne le possède pas, et ne
/// peut donc pas le modifier. C'est la garantie de pureté, rendue par le système de types.
#[derive(Debug, Clone, Copy)]
pub struct RunView<'a> {
    /// L'identifiant du run — il devient le **cas** dans les identifiants de nœuds
    /// (`{run}::{étape}`), suivant la convention de la pile B.
    pub run_id: Uuid,
    /// Le tenant propriétaire. Le pont ne s'en sert pas pour dériver ; il est là pour que
    /// l'appelant ne puisse pas mélanger deux runs par accident, et pour l'ancre de P.4.
    pub tenant: &'a str,
    /// Les enregistrements chaînés — l'évidence de la pile A.
    pub records: &'a [AgentCallRecord],
    /// La forensique LLM (un enregistrement par appel Mode C). Vide pour un run Mode A pur.
    pub llm_records: &'a [LlmProvenanceRecord],
    /// La racine **scellée à l'exécution**, telle que persistée.
    ///
    /// `None` pour un run antérieur à sa persistance ou pour un run vide. Sa présence **arme** le
    /// contrôle de complétude de [`crate::nodes_from_run`] : dès qu'un run porte une racine, la
    /// dérivation exige de la reproduire — c'est la même règle asymétrique qu'en P.0, et pour la
    /// même raison (un préfixe de chaîne HMAC valide est lui-même valide).
    pub sealed_root: Option<&'a str>,
}

impl<'a> RunView<'a> {
    /// Construit une vue. Constructeur explicite plutôt que littéral de structure : il donne un
    /// point unique où documenter l'ordre des arguments, et laisse la porte ouverte à une
    /// validation ultérieure sans casser les appelants.
    #[must_use]
    pub fn new(
        run_id: Uuid,
        tenant: &'a str,
        records: &'a [AgentCallRecord],
        llm_records: &'a [LlmProvenanceRecord],
        sealed_root: Option<&'a str>,
    ) -> Self {
        Self { run_id, tenant, records, llm_records, sealed_root }
    }

    /// Le run ne porte-t-il aucun enregistrement ?
    ///
    /// Utile à l'appelant, et surtout au pont : un run vide **avec** racine scellée est le cas
    /// vicieux que P.0 a appris à ne pas laisser passer (`verify_swarm` le déclare trivialement
    /// intègre et ne reconstruit aucune racine).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

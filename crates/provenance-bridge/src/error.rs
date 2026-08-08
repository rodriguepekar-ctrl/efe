// SPDX-License-Identifier: UNLICENSED
//! **Les refus typés** — chaque raison de ne pas dériver, nommée.
//!
//! Le pont ne rend **jamais** un DAG partiel. Un bundle amputé est indistinguable d'un bundle
//! tronqué par un attaquant : produire « ce qu'on a pu » reviendrait à fabriquer soi-même
//! l'ambiguïté que le fil entier cherche à supprimer. Toute anomalie est donc un `Err`, et chaque
//! `Err` porte de quoi la localiser sans relire le run.

use std::fmt;

use uuid::Uuid;

/// Pourquoi une dérivation a été refusée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeriveError {
    /// Un enregistrement déclare un parent causal absent du run.
    ///
    /// La pile B rejetterait de toute façon l'arête (référence pendante) ; le pont préfère le dire
    /// **avant**, avec les deux identifiants, plutôt que de laisser échouer un `append` plus loin
    /// avec moins de contexte.
    PendingRef {
        /// L'enregistrement fautif (celui qui cite).
        node: String,
        /// Le parent introuvable, par `record_id`.
        missing: Uuid,
    },

    /// Deux nœuds porteraient le même identifiant.
    ///
    /// L'identité vient de `seq`, strictement croissant dans une chaîne : un doublon signale donc
    /// une anomalie **de la pile A** (deux enregistrements de même `seq`). Le pont la remonte au
    /// lieu de la masquer — masquer reviendrait à sceller un run dont l'ordre est douteux.
    DuplicateNodeId(String),

    /// Un enregistrement est daté **avant** l'époque Unix.
    ///
    /// Aucune conversion silencieuse : une horloge qui ment est un fait à dire.
    TimestampBeforeEpoch {
        /// L'enregistrement fautif.
        record: Uuid,
    },

    /// Le run ne porte **aucune** racine scellée alors qu'il contient des enregistrements.
    ///
    /// On refuse de dériver d'un run dont on ne peut pas prouver la complétude : le bundle
    /// hériterait d'une autorité qu'il n'a pas. (Un run *vide* et sans racine est, lui, dérivable
    /// en un DAG vide — il n'y a rien à trahir.)
    MissingRoot,

    /// La racine reconstruite ne reproduit pas la racine scellée.
    ///
    /// **Le refus le plus important du pont.** Le run est déjà compromis — tronqué, amputé, ou
    /// altéré. Lui donner un bundle re-prouvable par le vérifieur auditable de la pile B
    /// **blanchirait** l'attaque : on offrirait un témoin crédible à un reçu menteur. C'est
    /// exactement l'inverse du but du fil, et c'est pire que de ne rien faire.
    RootMismatch {
        /// La racine telle que scellée à l'exécution.
        sealed: String,
        /// Ce que la re-preuve reconstruit (`None` si la chaîne ne se recompose pas, ou si le run
        /// a été entièrement vidé).
        recomputed: Option<String>,
    },
}

impl fmt::Display for DeriveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeriveError::PendingRef { node, missing } => {
                write!(f, "référence causale pendante : « {node} » cite le parent {missing}, absent du run")
            }
            DeriveError::DuplicateNodeId(id) => {
                write!(f, "identifiant de nœud dupliqué : « {id} » (deux enregistrements de même seq ?)")
            }
            DeriveError::TimestampBeforeEpoch { record } => {
                write!(f, "horodatage antérieur à l'époque Unix sur l'enregistrement {record}")
            }
            DeriveError::MissingRoot => {
                write!(f, "aucune racine scellée : la complétude du run ne peut pas être établie")
            }
            DeriveError::RootMismatch { sealed, recomputed } => match recomputed {
                Some(r) => write!(
                    f,
                    "racine scellée non reproduite — scellée « {sealed} », reconstruite « {r} » : \
                     le run a été tronqué ou altéré, aucun bundle ne sera produit"
                ),
                None => write!(
                    f,
                    "racine scellée « {sealed} » NON reconstructible (chaîne rompue ou run vidé) : \
                     aucun bundle ne sera produit"
                ),
            },
        }
    }
}

impl std::error::Error for DeriveError {}

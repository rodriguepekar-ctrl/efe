// SPDX-License-Identifier: UNLICENSED
//! # `scella-rfc3161` — l'ancre tierce, lue et écrite à la main.
//!
//! Voir [`der`] pour le lecteur, [`requete`] pour la demande d'horodatage, [`jeton`] pour la
//! lecture de la réponse.
pub mod der;
pub mod fabrique;
pub mod jeton;
pub mod requete;

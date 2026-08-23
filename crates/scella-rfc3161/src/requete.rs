// SPDX-License-Identifier: UNLICENSED
//! **La demande d'horodatage** (`TimeStampReq`, RFC 3161 §2.4.1).
//!
//! ```text
//! TimeStampReq ::= SEQUENCE {
//!   version        INTEGER { v1(1) },
//!   messageImprint MessageImprint,
//!   reqPolicy      TSAPolicyId  OPTIONAL,
//!   nonce          INTEGER      OPTIONAL,
//!   certReq        BOOLEAN      DEFAULT FALSE,
//!   extensions [0] IMPLICIT Extensions OPTIONAL }
//! ```
//!
//! ## Ce qu'on soumet, et pourquoi c'est la racine elle-même
//!
//! `hashedMessage` est **les 32 octets du manifest-root**, tels quels. Le root EST déjà une
//! empreinte SHA-256 ; le re-hacher ajouterait un étage que le tiers devrait reproduire. En le
//! soumettant nu, la vérification d'un auditeur tient en une ligne qu'il tape lui-même :
//!
//! ```text
//! openssl ts -verify -digest <manifest_root> -in anchor.tsr -CAfile <ca-de-lhorodateur>
//! ```
//!
//! ## Deux champs qui ne sont pas des détails
//!
//! * **`certReq = TRUE`** — demande à l'horodateur d'inclure son certificat dans le jeton. Sans
//!   lui, le jeton est vérifiable seulement par qui sait déjà où trouver ce certificat : on
//!   livrerait une preuve qui exige une recherche préalable, ce qui est le contraire d'opposable.
//! * **`nonce`** — lie la réponse à **cette** demande. Sans lui, un horodateur (ou un intermédiaire)
//!   pourrait rendre un jeton ancien portant le même condensé ; il attesterait alors un temps qui
//!   n'est pas celui qu'on croit. Le nonce est **fourni par l'appelant** : ce crate n'a aucune
//!   source d'entropie, et il n'en invente pas — même discipline que `CaseKey`.

use crate::der::{ecrire, ecrire_entier_positif, NULL, OCTET_STRING, OID, SEQUENCE};

/// L'OID de SHA-256 (`2.16.840.1.101.3.4.2.1`), déjà encodé.
pub const OID_SHA256: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];

/// Le type de contenu d'une demande, pour l'en-tête HTTP.
pub const TYPE_REQUETE: &str = "application/timestamp-query";
/// Le type de contenu attendu en réponse.
pub const TYPE_REPONSE: &str = "application/timestamp-reply";

/// Construit une demande d'horodatage pour un condensé SHA-256 de 32 octets.
///
/// `nonce` : les octets d'un entier positif, fournis par l'appelant. Vide = pas de nonce (accepté,
/// mais la réponse n'est alors liée à rien de particulier).
#[must_use]
pub fn demande(condense: &[u8; 32], nonce: &[u8]) -> Vec<u8> {
    // AlgorithmIdentifier ::= SEQUENCE { algorithm OID, parameters NULL }
    let algo = ecrire(
        SEQUENCE,
        &[&ecrire(OID, OID_SHA256)[..], &ecrire(NULL, &[])[..]].concat(),
    );
    // MessageImprint ::= SEQUENCE { hashAlgorithm, hashedMessage OCTET STRING }
    let imprint = ecrire(
        SEQUENCE,
        &[&algo[..], &ecrire(OCTET_STRING, condense)[..]].concat(),
    );

    let mut corps = Vec::with_capacity(128);
    corps.extend_from_slice(&ecrire_entier_positif(&[1])); // version v1
    corps.extend_from_slice(&imprint);
    if !nonce.is_empty() {
        corps.extend_from_slice(&ecrire_entier_positif(nonce));
    }
    // certReq TRUE — DER exige `FF` pour vrai (toute valeur non nulle est du BER).
    corps.extend_from_slice(&ecrire(0x01, &[0xFF]));

    ecrire(SEQUENCE, &corps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::der::{Lecteur, INTEGER};

    fn condense() -> [u8; 32] {
        let mut d = [0u8; 32];
        for (i, b) in d.iter_mut().enumerate() {
            *b = i as u8;
        }
        d
    }

    #[test]
    fn la_demande_se_relit_et_porte_le_condense_nu() {
        let der = demande(&condense(), &[0x11, 0x22]);
        let mut l = Lecteur::nouveau(&der);
        let seq = l.attendre(SEQUENCE).expect("TimeStampReq");
        l.fin().expect("rien en trop");

        let mut r = l.entrer(seq).expect("entrer");
        assert_eq!(r.attendre(INTEGER).expect("version"), &[1]);

        let imprint = r.attendre(SEQUENCE).expect("messageImprint");
        let mut mi = r.entrer(imprint).expect("entrer");
        let algo = mi.attendre(SEQUENCE).expect("algorithme");
        let mut a = mi.entrer(algo).expect("entrer");
        assert_eq!(a.attendre(OID).expect("oid"), OID_SHA256);
        // Le condensé est soumis TEL QUEL : c'est ce qui rend la commande de l'auditeur simple.
        assert_eq!(mi.attendre(OCTET_STRING).expect("condensé"), &condense()[..]);
    }

    #[test]
    fn certreq_est_vrai_et_encode_en_der() {
        let der = demande(&condense(), &[]);
        // 01 01 FF — `FF`, pas `01` : toute autre valeur non nulle serait du BER.
        assert!(
            der.windows(3).any(|w| w == [0x01, 0x01, 0xFF]),
            "certReq doit être TRUE : sans le certificat, le jeton exige une recherche préalable"
        );
    }

    #[test]
    fn le_nonce_est_present_ou_absent_selon_lappelant() {
        let avec = demande(&condense(), &[0x42]);
        let sans = demande(&condense(), &[]);
        assert!(avec.len() > sans.len(), "le nonce occupe de la place");
        assert!(avec.windows(3).any(|w| w == [0x02, 0x01, 0x42]), "et il est là");
    }

    /// Ce crate n'invente aucune entropie : sans nonce fourni, il n'en fabrique pas.
    #[test]
    fn aucun_nonce_nest_invente() {
        let src = include_str!("requete.rs");
        let hors_tests = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        let code: String =
            hors_tests.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for interdit in ["rand", "random", "SystemTime", "nanos"] {
            assert!(!code.contains(interdit), "« {interdit} » : l'entropie appartient à l'appelant");
        }
    }

    #[test]
    fn deux_demandes_identiques_sont_octet_pour_octet_identiques() {
        // Déterminisme : un test doit pouvoir figer une demande.
        assert_eq!(demande(&condense(), &[1, 2, 3]), demande(&condense(), &[1, 2, 3]));
    }
}

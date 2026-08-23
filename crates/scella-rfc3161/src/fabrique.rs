// SPDX-License-Identifier: UNLICENSED
//! **Fabrique de jetons — pour éprouver des lecteurs, jamais pour ancrer quoi que ce soit.**
//!
//! Ce module produit un jeton d'horodatage **structurellement valide et NON SIGNÉ**. Il existe
//! parce que trois crates ont besoin d'éprouver leur chemin d'ancrage sans dépendre d'un horodateur
//! réel — et qu'écrire trois fois le même faux jeton en ferait trois formats divergents.
//!
//! ## Pourquoi l'exposer publiquement ne retire aucune garantie
//!
//! On pourrait craindre de livrer un outil à forger des ancres. Il n'en est rien, et le raisonnement
//! mérite d'être écrit : ce dépôt **ne vérifie aucune signature** — c'est sa limite déclarée, et le
//! tiers la comble avec `openssl ts -verify`. Fabriquer un jeton non signé ne franchit donc aucune
//! garantie : cela rend simplement **visible** ce que la limite implique déjà. Un jeton d'ici est
//! rejeté par le premier vérifieur qui regarde sa signature — c'est-à-dire par celui qui compte.
//!
//! Cacher cette fabrique aurait laissé croire que nos jetons sont infalsifiables. Ils ne le sont pas
//! **par nous** ; ils le sont par l'horodateur.

use crate::der::{ecrire, ecrire_entier_positif, CTX0, GENERALIZED_TIME, NULL, OCTET_STRING, OID, SEQUENCE, SET};
use crate::requete::OID_SHA256;

const OID_SIGNED_DATA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02];
const OID_TST_INFO: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x10, 0x01, 0x04];

/// Un jeton **non signé** engageant `empreinte` à l'instant `temps` (`GeneralizedTime`, ex.
/// `"20260823120000Z"`).
///
/// ⚠ **Ne jamais employer comme ancre.** Un vérifieur tiers le rejette — c'est le but.
#[must_use]
pub fn jeton_non_signe(empreinte: &[u8; 32], temps: &str) -> Vec<u8> {
    jeton_non_signe_avec(empreinte, OID_SHA256, temps, &[])
}

/// La variante complète — pour éprouver les cas tordus (mauvais algorithme, nonce présent…).
#[must_use]
pub fn jeton_non_signe_avec(empreinte: &[u8], algo: &[u8], temps: &str, nonce: &[u8]) -> Vec<u8> {
    let algo_seq = ecrire(SEQUENCE, &[&ecrire(OID, algo)[..], &ecrire(NULL, &[])[..]].concat());
    let mi = ecrire(SEQUENCE, &[&algo_seq[..], &ecrire(OCTET_STRING, empreinte)[..]].concat());

    let mut tst = Vec::new();
    tst.extend_from_slice(&ecrire_entier_positif(&[1]));
    tst.extend_from_slice(&ecrire(OID, &[0x2A, 0x03])); // politique de démonstration
    tst.extend_from_slice(&mi);
    tst.extend_from_slice(&ecrire_entier_positif(&[0x2A]));
    tst.extend_from_slice(&ecrire(GENERALIZED_TIME, temps.as_bytes()));
    if !nonce.is_empty() {
        tst.extend_from_slice(&ecrire_entier_positif(nonce));
    }
    let tst = ecrire(SEQUENCE, &tst);

    let encap = ecrire(
        SEQUENCE,
        &[&ecrire(OID, OID_TST_INFO)[..], &ecrire(CTX0, &ecrire(OCTET_STRING, &tst))[..]].concat(),
    );
    // `signerInfos` est un SET VIDE : le jeton porte donc, dans sa structure même, l'aveu qu'il
    // n'est signé par personne.
    let signed = ecrire(
        SEQUENCE,
        &[
            &ecrire_entier_positif(&[3])[..],
            &ecrire(SET, &[])[..],
            &encap[..],
            &ecrire(SET, &[])[..],
        ]
        .concat(),
    );
    ecrire(SEQUENCE, &[&ecrire(OID, OID_SIGNED_DATA)[..], &ecrire(CTX0, &signed)[..]].concat())
}

/// Une réponse d'horodatage complète autour d'un jeton (statut 0 = accordé).
#[must_use]
pub fn reponse(statut: u8, jeton: Option<&[u8]>) -> Vec<u8> {
    let mut corps = ecrire(SEQUENCE, &ecrire_entier_positif(&[statut]));
    if let Some(j) = jeton {
        corps.extend_from_slice(j);
    }
    ecrire(SEQUENCE, &corps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jeton::lire_jeton;

    #[test]
    fn un_jeton_fabrique_se_relit_et_engage_ce_quon_lui_a_donne() {
        let e = [9u8; 32];
        let info = lire_jeton(&jeton_non_signe(&e, "20260823120000Z")).expect("relu");
        assert!(info.engage(&e));
        assert_eq!(info.instant, 1_787_486_400);
    }

    /// Le jeton porte dans sa structure l'aveu qu'il n'est signé par personne : `signerInfos` vide.
    #[test]
    fn un_jeton_fabrique_navoue_aucun_signataire() {
        let j = jeton_non_signe(&[1u8; 32], "20260823120000Z");
        // 31 00 = SET vide. Il y en a deux : digestAlgorithms et signerInfos.
        assert_eq!(j.windows(2).filter(|w| *w == [0x31, 0x00]).count(), 2);
    }
}

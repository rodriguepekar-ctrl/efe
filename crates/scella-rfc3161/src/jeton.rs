// SPDX-License-Identifier: UNLICENSED
//! **La lecture de la réponse** — et la seule question qui compte : *ce jeton engage-t-il CETTE
//! racine ?*
//!
//! ## Pourquoi lire le jeton, alors qu'on ne vérifie pas sa signature
//!
//! C'est la décision centrale de M9, et elle mérite d'être écrite. Sans lecture, `bound_root` d'une
//! `AnchorProof` serait **notre affirmation** sur ce que le jeton couvre. Le vérifieur comparerait
//! alors notre affirmation à notre racine : une liaison **circulaire**, que n'importe quel jeton
//! satisferait — y compris celui d'un autre document, ou d'un autre jour.
//!
//! Extraire le `messageImprint` du `TSTInfo` casse ce cercle. Le jeton dit lui-même ce qu'il
//! engage, et on le compare à la racine. C'est ce pas — et lui seul — qui fait passer l'ancre de
//! *déclarée* à *opposable*.
//!
//! ## Ce que ce module NE fait PAS, et qui n'est pas un oubli
//!
//! Il ne vérifie **ni la signature CMS, ni la chaîne de certificats** de l'horodateur. La règle du
//! dépôt est explicite — *« aucune crypto maison, aucune 3ᵉ pile »* — et une vérification de
//! signature écrite ici serait exactement cela. Elle appartient au tiers, qui la fait avec un outil
//! qu'il choisit :
//!
//! ```text
//! openssl ts -verify -digest <manifest_root> -in anchor.tsr -CAfile <ca-de-lhorodateur>
//! ```
//!
//! La division est nette : **nous** garantissons que le jeton livré engage bien la racine livrée ;
//! **le tiers** garantit que l'horodateur l'a bien signé. Aucun des deux ne demande à l'autre d'être
//! cru.
//!
//! ```text
//! TimeStampResp ::= SEQUENCE { status PKIStatusInfo, timeStampToken ContentInfo OPTIONAL }
//! ContentInfo   ::= SEQUENCE { contentType OID(signedData), content [0] EXPLICIT SignedData }
//! SignedData    ::= SEQUENCE { version, digestAlgorithms SET, encapContentInfo, ... }
//! EncapContentInfo ::= SEQUENCE { eContentType OID(TSTInfo), eContent [0] EXPLICIT OCTET STRING }
//! TSTInfo       ::= SEQUENCE { version, policy, messageImprint, serialNumber, genTime, ... }
//! ```

use crate::der::{
    ErreurDer, Lecteur, CTX0, GENERALIZED_TIME, INTEGER, OCTET_STRING, OID, SEQUENCE, SET,
};
use crate::requete::OID_SHA256;

/// `id-signedData` — 1.2.840.113549.1.7.2
const OID_SIGNED_DATA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02];
/// `id-ct-TSTInfo` — 1.2.840.113549.1.9.16.1.4
const OID_TST_INFO: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x10, 0x01, 0x04];

/// Un jeton plus grand que cela n'est pas un horodatage : c'est autre chose.
pub const TAILLE_MAX: usize = 1 << 20;

/// Ce qui peut mal se passer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Erreur {
    /// La structure ne se lit pas.
    Der(ErreurDer),
    /// L'horodateur a refusé, avec son code de statut PKI et son texte s'il en a donné.
    Refus { statut: u64, texte: String },
    /// Réponse acceptée mais sans jeton — RFC 3161 l'interdit pour un statut 0/1.
    SansJeton { statut: u64 },
    /// Le contenu n'est pas un `SignedData`, ou n'encapsule pas un `TSTInfo`.
    PasUnJeton(&'static str),
    /// Le temps annoncé ne se lit pas.
    TempsInvalide(String),
    /// Le jeton dépasse [`TAILLE_MAX`].
    TropGrand { taille: usize },
}

impl From<ErreurDer> for Erreur {
    fn from(e: ErreurDer) -> Self {
        Erreur::Der(e)
    }
}

impl core::fmt::Display for Erreur {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Erreur::Der(e) => write!(f, "{e}"),
            Erreur::Refus { statut, texte } if texte.is_empty() => {
                write!(f, "l'horodateur a refusé (statut PKI {statut})")
            }
            Erreur::Refus { statut, texte } => {
                write!(f, "l'horodateur a refusé (statut PKI {statut}) : {texte}")
            }
            Erreur::SansJeton { statut } => write!(
                f,
                "réponse de statut {statut} mais sans jeton — RFC 3161 §2.4.2 l'interdit ; on ne \
                 peut donc rien ancrer, et prétendre le contraire serait une ancre inventée"
            ),
            Erreur::PasUnJeton(quoi) => write!(f, "ce n'est pas un jeton d'horodatage : {quoi}"),
            Erreur::TempsInvalide(t) => write!(f, "temps illisible : « {t} »"),
            Erreur::TropGrand { taille } => {
                write!(f, "{taille} octets : au-delà de {TAILLE_MAX}, ce n'est plus un horodatage")
            }
        }
    }
}

/// Le contenu utile d'un `TSTInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TstInfo {
    /// L'OID de la politique de l'horodateur, en notation pointée.
    pub politique: String,
    /// L'OID de l'algorithme du condensé, brut.
    pub algorithme: Vec<u8>,
    /// **Le champ décisif** : ce que le jeton engage.
    pub empreinte: Vec<u8>,
    /// Le numéro de série, brut (identifie le jeton chez l'horodateur).
    pub serie: Vec<u8>,
    /// `genTime`, converti en secondes depuis l'époque.
    pub instant: u64,
    /// Le nonce rendu, s'il y en a un — à comparer à celui qu'on a envoyé.
    pub nonce: Vec<u8>,
}

impl TstInfo {
    /// **Ce jeton engage-t-il cette racine ?** La question de tout le module.
    ///
    /// Vérifie l'algorithme **et** l'empreinte. Sans le premier contrôle, on comparerait 32 octets
    /// à 32 octets sans savoir de quelle fonction ils sortent.
    #[must_use]
    pub fn engage(&self, racine: &[u8; 32]) -> bool {
        self.algorithme == OID_SHA256 && self.empreinte == racine
    }
}

/// Lit une réponse d'horodatage complète : rend le **jeton brut** (à livrer tel quel) et son
/// `TSTInfo`.
pub fn lire_reponse(octets: &[u8]) -> Result<(Vec<u8>, TstInfo), Erreur> {
    if octets.len() > TAILLE_MAX {
        return Err(Erreur::TropGrand { taille: octets.len() });
    }
    let mut l = Lecteur::nouveau(octets);
    let resp = l.attendre(SEQUENCE)?;
    let mut r = l.entrer(resp)?;

    // PKIStatusInfo ::= SEQUENCE { status INTEGER, statusString OPTIONAL, failInfo OPTIONAL }
    let info = r.attendre(SEQUENCE)?;
    let mut si = r.entrer(info)?;
    let statut = entier(si.attendre(INTEGER)?)?;
    // Le texte de refus est facultatif ; on le rend s'il est là, parce que « statut 2 » n'aide
    // personne à comprendre pourquoi son horodateur dit non.
    let texte = si
        .etiquette_suivante()
        .and_then(|_| si.suivant().ok())
        .map(|e| String::from_utf8_lossy(e.contenu).chars().filter(|c| !c.is_control()).collect())
        .unwrap_or_default();

    // 0 = granted, 1 = grantedWithMods. Tout le reste est un refus.
    if statut > 1 {
        return Err(Erreur::Refus { statut, texte });
    }
    if r.fini() {
        return Err(Erreur::SansJeton { statut });
    }

    // Le jeton est rendu TEL QUEL, en-tête compris : c'est lui qu'un tiers passera à `openssl`.
    let debut = octets.len() - r.restant();
    let jeton_element = r.suivant()?;
    let fin = octets.len() - r.restant();
    let jeton = octets[debut..fin].to_vec();
    if jeton_element.etiquette != SEQUENCE {
        return Err(Erreur::PasUnJeton("le jeton n'est pas une SEQUENCE"));
    }

    let info = lire_jeton(&jeton)?;
    Ok((jeton, info))
}

/// Lit un jeton d'horodatage seul (un `ContentInfo` DER — le contenu d'un fichier `.tsr`).
pub fn lire_jeton(jeton: &[u8]) -> Result<TstInfo, Erreur> {
    if jeton.len() > TAILLE_MAX {
        return Err(Erreur::TropGrand { taille: jeton.len() });
    }
    let mut l = Lecteur::nouveau(jeton);
    let ci = l.attendre(SEQUENCE)?;
    let mut c = l.entrer(ci)?;

    if c.attendre(OID)? != OID_SIGNED_DATA {
        return Err(Erreur::PasUnJeton("le type de contenu n'est pas signedData"));
    }
    let explicite = c.attendre(CTX0)?;
    let mut e = c.entrer(explicite)?;
    let signed = e.attendre(SEQUENCE)?;
    let mut sd = e.entrer(signed)?;

    sd.attendre(INTEGER)?; // version
    sd.attendre(SET)?; // digestAlgorithms
    let encap = sd.attendre(SEQUENCE)?; // encapContentInfo
    let mut en = sd.entrer(encap)?;

    if en.attendre(OID)? != OID_TST_INFO {
        return Err(Erreur::PasUnJeton("le contenu encapsulé n'est pas un TSTInfo"));
    }
    let ctx = en.attendre(CTX0)?;
    let mut cx = en.entrer(ctx)?;
    let tst_der = cx.attendre(OCTET_STRING)?;

    lire_tstinfo(tst_der)
}

fn lire_tstinfo(der: &[u8]) -> Result<TstInfo, Erreur> {
    let mut l = Lecteur::nouveau(der);
    let seq = l.attendre(SEQUENCE)?;
    let mut t = l.entrer(seq)?;

    t.attendre(INTEGER)?; // version
    let politique = oid_pointe(t.attendre(OID)?);

    // MessageImprint ::= SEQUENCE { hashAlgorithm AlgorithmIdentifier, hashedMessage OCTET STRING }
    let mi = t.attendre(SEQUENCE)?;
    let mut m = t.entrer(mi)?;
    let algo_seq = m.attendre(SEQUENCE)?;
    let mut a = m.entrer(algo_seq)?;
    let algorithme = a.attendre(OID)?.to_vec();
    let empreinte = m.attendre(OCTET_STRING)?.to_vec();

    let serie = t.attendre(INTEGER)?.to_vec();
    let brut = t.attendre(GENERALIZED_TIME)?;
    let texte = String::from_utf8_lossy(brut).to_string();
    let instant = temps_generalise(&texte).ok_or_else(|| Erreur::TempsInvalide(texte.clone()))?;

    // accuracy / ordering / nonce sont facultatifs et dans cet ordre. On ne cherche QUE le nonce :
    // un INTEGER de premier niveau après le temps, une fois accuracy (SEQUENCE) et ordering
    // (BOOLEAN) écartés.
    let mut nonce = Vec::new();
    while let Some(etiq) = t.etiquette_suivante() {
        match etiq {
            INTEGER => {
                nonce = t.attendre(INTEGER)?.to_vec();
                break;
            }
            SEQUENCE | 0x01 => t.sauter()?, // accuracy, ordering
            _ => break,                     // [0] tsa, [1] extensions : plus de nonce possible
        }
    }

    Ok(TstInfo { politique, algorithme, empreinte, serie, instant, nonce })
}

/// Un INTEGER DER non négatif, borné à `u64`.
fn entier(octets: &[u8]) -> Result<u64, Erreur> {
    if octets.is_empty() {
        return Err(Erreur::Der(ErreurDer::Contenu("entier vide")));
    }
    if octets[0] & 0x80 != 0 {
        return Err(Erreur::Der(ErreurDer::Contenu("entier négatif")));
    }
    let sig: Vec<u8> = octets.iter().copied().skip_while(|b| *b == 0).collect();
    if sig.len() > 8 {
        return Err(Erreur::Der(ErreurDer::Contenu("entier trop grand")));
    }
    Ok(sig.iter().fold(0u64, |acc, b| (acc << 8) | *b as u64))
}

/// Un OID DER en notation pointée.
fn oid_pointe(der: &[u8]) -> String {
    let Some((&premier, reste)) = der.split_first() else { return String::new() };
    let mut out = format!("{}.{}", premier / 40, premier % 40);
    let mut v: u128 = 0;
    for b in reste {
        // Un OID pathologique ne doit pas déborder : au-delà, on s'arrête plutôt que d'inventer.
        v = match v.checked_mul(128).and_then(|x| x.checked_add((b & 0x7F) as u128)) {
            Some(x) => x,
            None => return out,
        };
        if b & 0x80 == 0 {
            out.push('.');
            out.push_str(&v.to_string());
            v = 0;
        }
    }
    out
}

/// `GeneralizedTime` → secondes depuis l'époque. Exige `Z` (UTC) : RFC 3161 §2.4.2 l'impose, et
/// accepter un décalage local ferait dépendre le temps attesté du fuseau de qui l'a écrit.
fn temps_generalise(s: &str) -> Option<u64> {
    let s = s.trim();
    let s = s.strip_suffix('Z')?;
    // La partie fractionnaire est autorisée et sans intérêt ici : on horodate à la seconde.
    let s = s.split('.').next()?;
    if s.len() != 14 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (an, mois, jour) = (n(0, 4)?, n(4, 6)?, n(6, 8)?);
    let (h, mi, sec) = (n(8, 10)?, n(10, 12)?, n(12, 14)?);
    if !(1..=12).contains(&mois) || !(1..=31).contains(&jour) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let jours = jours_depuis_epoque(an, mois, jour)?;
    u64::try_from(jours * 86_400 + h * 3600 + mi * 60 + sec).ok()
}

/// Jours civils depuis 1970-01-01 (algorithme de Howard Hinnant, sans dépendance).
fn jours_depuis_epoque(an: i64, mois: i64, jour: i64) -> Option<i64> {
    let y = if mois <= 2 { an - 1 } else { an };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mois + 9) % 12;
    let doy = (153 * mp + 2) / 5 + jour - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::der::{ecrire, ecrire_entier_positif, NULL};

    fn racine(n: u8) -> [u8; 32] {
        [n; 32]
    }

    /// Fabrique un TSTInfo, puis un jeton, puis une réponse — pour éprouver la lecture sans dépendre
    /// d'un horodateur réel. Le test d'intégration, lui, lit un jeton VÉRITABLE.
    fn tstinfo_der(empreinte: &[u8], algo: &[u8], temps: &str, nonce: &[u8]) -> Vec<u8> {
        let algo_seq = ecrire(SEQUENCE, &[&ecrire(OID, algo)[..], &ecrire(NULL, &[])[..]].concat());
        let mi = ecrire(SEQUENCE, &[&algo_seq[..], &ecrire(OCTET_STRING, empreinte)[..]].concat());
        let mut corps = Vec::new();
        corps.extend_from_slice(&ecrire_entier_positif(&[1]));
        corps.extend_from_slice(&ecrire(OID, &[0x2A, 0x03])); // une politique quelconque
        corps.extend_from_slice(&mi);
        corps.extend_from_slice(&ecrire_entier_positif(&[0x2A]));
        corps.extend_from_slice(&ecrire(GENERALIZED_TIME, temps.as_bytes()));
        if !nonce.is_empty() {
            corps.extend_from_slice(&ecrire_entier_positif(nonce));
        }
        ecrire(SEQUENCE, &corps)
    }

    fn jeton_der(tst: &[u8]) -> Vec<u8> {
        let encap = ecrire(
            SEQUENCE,
            &[
                &ecrire(OID, OID_TST_INFO)[..],
                &ecrire(CTX0, &ecrire(OCTET_STRING, tst))[..],
            ]
            .concat(),
        );
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
        ecrire(
            SEQUENCE,
            &[&ecrire(OID, OID_SIGNED_DATA)[..], &ecrire(CTX0, &signed)[..]].concat(),
        )
    }

    fn reponse(statut: u8, jeton: Option<&[u8]>) -> Vec<u8> {
        let si = ecrire(SEQUENCE, &ecrire_entier_positif(&[statut]));
        let mut corps = si;
        if let Some(j) = jeton {
            corps.extend_from_slice(j);
        }
        ecrire(SEQUENCE, &corps)
    }

    #[test]
    fn une_reponse_accordee_rend_le_jeton_et_son_contenu() {
        let tst = tstinfo_der(&racine(7), OID_SHA256, "20260823120000Z", &[0x11]);
        let jeton = jeton_der(&tst);
        let (rendu, info) = lire_reponse(&reponse(0, Some(&jeton))).expect("lue");
        assert_eq!(rendu, jeton, "le jeton est rendu TEL QUEL — c'est lui qu'un tiers vérifiera");
        assert_eq!(info.empreinte, racine(7));
        assert_eq!(info.algorithme, OID_SHA256);
        assert_eq!(info.nonce, vec![0x11]);
        assert_eq!(info.politique, "1.2.3");
        assert_eq!(info.serie, vec![0x2A]);
    }

    /// ⭐ **La question de tout le module.** Sans elle, `bound_root` serait notre affirmation, et le
    /// vérifieur comparerait notre affirmation à notre racine — un cercle.
    #[test]
    fn un_jeton_dune_autre_racine_nengage_pas_la_notre() {
        let tst = tstinfo_der(&racine(1), OID_SHA256, "20260823120000Z", &[]);
        let info = lire_jeton(&jeton_der(&tst)).expect("lu");
        assert!(info.engage(&racine(1)), "il engage la sienne");
        assert!(!info.engage(&racine(2)), "et PAS une autre");
    }

    /// Comparer 32 octets à 32 octets sans savoir de quelle fonction ils sortent ne prouve rien.
    #[test]
    fn un_condense_dun_autre_algorithme_nengage_rien() {
        let sha1 = &[0x2B, 0x0E, 0x03, 0x02, 0x1A];
        let tst = tstinfo_der(&racine(3), sha1, "20260823120000Z", &[]);
        let info = lire_jeton(&jeton_der(&tst)).expect("lu");
        assert!(!info.engage(&racine(3)), "même empreinte, mauvais algorithme : refusé");
    }

    #[test]
    fn un_refus_est_rendu_avec_son_statut() {
        let e = lire_reponse(&reponse(2, None)).expect_err("refus");
        assert!(matches!(e, Erreur::Refus { statut: 2, .. }), "{e}");
        assert!(e.to_string().contains("statut PKI 2"));
    }

    /// Un statut accordé sans jeton : RFC 3161 l'interdit, et « ancrer » là-dessus serait inventer
    /// une ancre.
    #[test]
    fn un_statut_accorde_sans_jeton_est_une_erreur() {
        let e = lire_reponse(&reponse(0, None)).expect_err("sans jeton");
        assert!(matches!(e, Erreur::SansJeton { statut: 0 }));
        assert!(e.to_string().contains("ancre inventée"));
    }

    #[test]
    fn ce_qui_nest_pas_un_jeton_est_refuse_avec_sa_raison() {
        // Un ContentInfo dont le type n'est pas signedData.
        let faux = ecrire(SEQUENCE, &[&ecrire(OID, &[0x2A, 0x03])[..], &ecrire(CTX0, &[])[..]].concat());
        assert_eq!(lire_jeton(&faux), Err(Erreur::PasUnJeton("le type de contenu n'est pas signedData")));
    }

    #[test]
    fn le_temps_generalise_se_convertit() {
        assert_eq!(temps_generalise("19700101000000Z"), Some(0));
        assert_eq!(temps_generalise("20000101000000Z"), Some(946_684_800));
        assert_eq!(temps_generalise("20260823120000Z"), Some(1_787_486_400));
        assert_eq!(temps_generalise("20260823120000.123Z"), Some(1_787_486_400), "fraction ignorée");
    }

    /// Sans `Z`, le temps attesté dépendrait du fuseau de qui l'a écrit.
    #[test]
    fn un_temps_sans_z_ou_malforme_est_refuse() {
        for t in ["20260823120000", "20260823120000+0200", "2026082312Z", "AAAABBCCDDEEFFZ", "", "Z"] {
            assert_eq!(temps_generalise(t), None, "doit refuser « {t} »");
        }
        assert_eq!(temps_generalise("20261323120000Z"), None, "mois 13");
        assert_eq!(temps_generalise("20260832120000Z"), None, "jour 32");
    }

    #[test]
    fn un_jeton_trop_grand_est_refuse_avant_analyse() {
        let enorme = vec![0x30; TAILLE_MAX + 1];
        assert!(matches!(lire_jeton(&enorme), Err(Erreur::TropGrand { .. })));
    }

    /// La réponse vient du réseau : aucun tampon ne doit faire paniquer.
    #[test]
    fn aucune_reponse_hostile_ne_fait_paniquer() {
        let bon = reponse(0, Some(&jeton_der(&tstinfo_der(&racine(1), OID_SHA256, "20260823120000Z", &[]))));
        let mut hostiles: Vec<Vec<u8>> = vec![
            vec![],
            vec![0x30, 0x00],
            vec![0xFF; 100],
            (0u8..=255).collect(),
        ];
        // Tous les préfixes du bon message : chaque troncature possible.
        for n in 0..bon.len() {
            hostiles.push(bon[..n].to_vec());
        }
        // Et chaque octet retourné, un par un.
        for i in 0..bon.len().min(64) {
            let mut m = bon.clone();
            m[i] ^= 0xFF;
            hostiles.push(m);
        }
        for h in &hostiles {
            let _ = lire_reponse(h); // ne doit ni paniquer ni boucler
            let _ = lire_jeton(h);
        }
    }
}

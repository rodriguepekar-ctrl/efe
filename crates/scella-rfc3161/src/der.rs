// SPDX-License-Identifier: UNLICENSED
//! **Un lecteur DER minimal, écrit pour de l'entrée hostile.**
//!
//! Ce module lit la réponse d'un horodateur — c'est-à-dire des octets venus du réseau, fabriqués par
//! quelqu'un d'autre. C'est l'entrée la plus hostile de tout le dépôt, et le lecteur est écrit en
//! conséquence :
//!
//! * **aucune indexation nue** : toute lecture passe par une borne vérifiée, donc aucune panique ;
//! * **longueur indéfinie refusée** — elle n'est pas du DER (elle est du BER), et l'accepter
//!   ouvrirait une lecture qui cherche sa fin ;
//! * **longueur non minimale refusée** : `81 05` au lieu de `05` encode la même valeur de deux
//!   façons. Deux encodages pour une valeur, c'est deux lectures possibles d'un même octet — et un
//!   jeton qui se lit différemment selon l'outil n'est plus une preuve ;
//! * **profondeur bornée** : une imbrication profonde ferait déborder la pile ;
//! * **forme longue de tag refusée** : aucune structure RFC 3161 n'en emploie, et la refuser retire
//!   un chemin d'analyse entier plutôt que de le durcir.
//!
//! Il ne lit **que** ce dont M9 a besoin. Ce n'est pas une bibliothèque ASN.1 ; c'est le strict
//! nécessaire pour répondre à une seule question : *ce jeton engage-t-il CETTE racine ?*

/// La profondeur maximale d'imbrication acceptée. Les structures de RFC 3161 en emploient six ;
/// seize laisse de la marge sans laisser de place à un débordement de pile.
pub const PROFONDEUR_MAX: usize = 16;

/// Les étiquettes dont ce module a besoin.
pub const INTEGER: u8 = 0x02;
pub const BIT_STRING: u8 = 0x03;
pub const OCTET_STRING: u8 = 0x04;
pub const NULL: u8 = 0x05;
pub const OID: u8 = 0x06;
pub const GENERALIZED_TIME: u8 = 0x18;
pub const SEQUENCE: u8 = 0x30;
pub const SET: u8 = 0x31;
/// `[0]` explicite (constructed, contextuel).
pub const CTX0: u8 = 0xA0;

/// Ce qui peut mal se passer en lisant. Chaque cas est **nommé** : « DER invalide » ne dit pas à un
/// exploitant si son horodateur est cassé ou si quelqu'un lui a rendu autre chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErreurDer {
    /// Le tampon s'arrête au milieu d'un élément.
    Tronque { position: usize, attendu: usize, restant: usize },
    /// Longueur indéfinie (`80`) — c'est du BER, pas du DER.
    LongueurIndefinie { position: usize },
    /// Longueur encodée de façon non minimale, ou réservée (`FF`).
    LongueurNonMinimale { position: usize },
    /// Forme longue d'étiquette (`1F`) — refusée par construction.
    EtiquetteLongue { position: usize },
    /// L'étiquette lue n'est pas celle attendue.
    Inattendu { position: usize, attendu: u8, trouve: u8 },
    /// Trop d'imbrications.
    TropProfond,
    /// Des octets traînent après l'élément attendu.
    OctetsEnTrop { restant: usize },
    /// Un contenu qui ne respecte pas sa propre forme (entier vide, temps malformé…).
    Contenu(&'static str),
}

impl core::fmt::Display for ErreurDer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ErreurDer::Tronque { position, attendu, restant } => write!(
                f,
                "DER tronqué à l'octet {position} : {attendu} octets annoncés, {restant} disponibles"
            ),
            ErreurDer::LongueurIndefinie { position } => write!(
                f,
                "longueur indéfinie à l'octet {position} : c'est du BER, pas du DER — refusée"
            ),
            ErreurDer::LongueurNonMinimale { position } => write!(
                f,
                "longueur non minimale à l'octet {position} : deux encodages pour une valeur, donc \
                 deux lectures possibles d'un même jeton"
            ),
            ErreurDer::EtiquetteLongue { position } => {
                write!(f, "forme longue d'étiquette à l'octet {position} : refusée par construction")
            }
            ErreurDer::Inattendu { position, attendu, trouve } => write!(
                f,
                "étiquette inattendue à l'octet {position} : {trouve:#04x} au lieu de {attendu:#04x}"
            ),
            ErreurDer::TropProfond => write!(f, "imbrication au-delà de {PROFONDEUR_MAX} niveaux"),
            ErreurDer::OctetsEnTrop { restant } => {
                write!(f, "{restant} octet(s) en trop après l'élément lu")
            }
            ErreurDer::Contenu(quoi) => write!(f, "contenu invalide : {quoi}"),
        }
    }
}

/// Un élément lu : son étiquette et son contenu (sans l'en-tête).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Element<'a> {
    pub etiquette: u8,
    pub contenu: &'a [u8],
}

/// Le lecteur : un curseur sur un tampon, qui n'indexe jamais sans vérifier.
#[derive(Debug, Clone)]
pub struct Lecteur<'a> {
    octets: &'a [u8],
    pos: usize,
    profondeur: usize,
}

impl<'a> Lecteur<'a> {
    #[must_use]
    pub fn nouveau(octets: &'a [u8]) -> Self {
        Lecteur { octets, pos: 0, profondeur: 0 }
    }

    fn a_profondeur(octets: &'a [u8], profondeur: usize) -> Self {
        Lecteur { octets, pos: 0, profondeur }
    }

    /// Reste-t-il quelque chose ?
    #[must_use]
    pub fn fini(&self) -> bool {
        self.pos >= self.octets.len()
    }

    #[must_use]
    pub fn restant(&self) -> usize {
        self.octets.len().saturating_sub(self.pos)
    }

    fn octet(&mut self) -> Result<u8, ErreurDer> {
        let b = *self.octets.get(self.pos).ok_or(ErreurDer::Tronque {
            position: self.pos,
            attendu: 1,
            restant: 0,
        })?;
        self.pos += 1;
        Ok(b)
    }

    /// Lit l'élément suivant, quel qu'il soit.
    pub fn suivant(&mut self) -> Result<Element<'a>, ErreurDer> {
        let debut = self.pos;
        let etiquette = self.octet()?;
        if etiquette & 0x1F == 0x1F {
            return Err(ErreurDer::EtiquetteLongue { position: debut });
        }
        let longueur = self.longueur()?;
        let restant = self.restant();
        if longueur > restant {
            return Err(ErreurDer::Tronque { position: debut, attendu: longueur, restant });
        }
        let contenu = &self.octets[self.pos..self.pos + longueur];
        self.pos += longueur;
        Ok(Element { etiquette, contenu })
    }

    fn longueur(&mut self) -> Result<usize, ErreurDer> {
        let debut = self.pos;
        let premier = self.octet()?;
        if premier < 0x80 {
            return Ok(premier as usize);
        }
        if premier == 0x80 {
            return Err(ErreurDer::LongueurIndefinie { position: debut });
        }
        if premier == 0xFF {
            return Err(ErreurDer::LongueurNonMinimale { position: debut });
        }
        let n = (premier & 0x7F) as usize;
        // Plus de 8 octets de longueur ne tient de toute façon pas dans un `usize` raisonnable, et
        // aucun jeton légitime n'en emploie autant.
        if n > core::mem::size_of::<usize>() {
            return Err(ErreurDer::LongueurNonMinimale { position: debut });
        }
        let mut v: usize = 0;
        for i in 0..n {
            let b = self.octet()?;
            if i == 0 && b == 0 {
                // Un zéro de tête rend l'encodage non minimal.
                return Err(ErreurDer::LongueurNonMinimale { position: debut });
            }
            v = (v << 8) | b as usize;
        }
        // La forme longue ne doit pas encoder ce que la forme courte pourrait dire.
        if v < 0x80 {
            return Err(ErreurDer::LongueurNonMinimale { position: debut });
        }
        Ok(v)
    }

    /// Lit l'élément suivant en exigeant son étiquette.
    pub fn attendre(&mut self, etiquette: u8) -> Result<&'a [u8], ErreurDer> {
        let debut = self.pos;
        let e = self.suivant()?;
        if e.etiquette != etiquette {
            return Err(ErreurDer::Inattendu {
                position: debut,
                attendu: etiquette,
                trouve: e.etiquette,
            });
        }
        Ok(e.contenu)
    }

    /// Un lecteur sur le contenu d'un élément — un cran plus profond.
    pub fn entrer(&self, contenu: &'a [u8]) -> Result<Lecteur<'a>, ErreurDer> {
        if self.profondeur + 1 > PROFONDEUR_MAX {
            return Err(ErreurDer::TropProfond);
        }
        Ok(Lecteur::a_profondeur(contenu, self.profondeur + 1))
    }

    /// Regarde l'étiquette suivante sans consommer. `None` si le tampon est fini.
    #[must_use]
    pub fn etiquette_suivante(&self) -> Option<u8> {
        self.octets.get(self.pos).copied()
    }

    /// Consomme et jette l'élément suivant.
    pub fn sauter(&mut self) -> Result<(), ErreurDer> {
        self.suivant().map(|_| ())
    }

    /// Exige que le tampon soit épuisé — ce qui interdit qu'un jeton porte une queue silencieuse.
    pub fn fin(&self) -> Result<(), ErreurDer> {
        if self.fini() {
            Ok(())
        } else {
            Err(ErreurDer::OctetsEnTrop { restant: self.restant() })
        }
    }
}

/// Écrit un élément DER (étiquette + longueur minimale + contenu).
#[must_use]
pub fn ecrire(etiquette: u8, contenu: &[u8]) -> Vec<u8> {
    let n = contenu.len();
    let mut out = Vec::with_capacity(n + 6);
    out.push(etiquette);
    if n < 0x80 {
        out.push(n as u8);
    } else {
        // Forme longue MINIMALE : on retire les zéros de tête.
        let brut = n.to_be_bytes();
        let significatifs: Vec<u8> = brut.iter().copied().skip_while(|b| *b == 0).collect();
        out.push(0x80 | significatifs.len() as u8);
        out.extend_from_slice(&significatifs);
    }
    out.extend_from_slice(contenu);
    out
}

/// Écrit un INTEGER non négatif à partir de ses octets significatifs.
///
/// Un `0x80` de tête ferait un entier **négatif** en complément à deux : DER exige alors un octet
/// nul devant. L'oublier produirait un nonce négatif — accepté par certains horodateurs, refusé par
/// d'autres, et impossible à diagnostiquer.
#[must_use]
pub fn ecrire_entier_positif(v: &[u8]) -> Vec<u8> {
    let sig: Vec<u8> = v.iter().copied().skip_while(|b| *b == 0).collect();
    if sig.is_empty() {
        return ecrire(INTEGER, &[0]);
    }
    if sig[0] & 0x80 != 0 {
        let mut avec_zero = Vec::with_capacity(sig.len() + 1);
        avec_zero.push(0);
        avec_zero.extend_from_slice(&sig);
        return ecrire(INTEGER, &avec_zero);
    }
    ecrire(INTEGER, &sig)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_sequence_simple_se_lit() {
        // SEQUENCE { INTEGER 1, OCTET STRING "ab" }
        let der = ecrire(SEQUENCE, &[&ecrire(INTEGER, &[1])[..], &ecrire(OCTET_STRING, b"ab")[..]].concat());
        let mut l = Lecteur::nouveau(&der);
        let seq = l.attendre(SEQUENCE).expect("séquence");
        l.fin().expect("rien en trop");
        let mut inner = l.entrer(seq).expect("entrer");
        assert_eq!(inner.attendre(INTEGER).expect("entier"), &[1]);
        assert_eq!(inner.attendre(OCTET_STRING).expect("chaîne"), b"ab");
        assert!(inner.fini());
    }

    #[test]
    fn la_longueur_indefinie_est_refusee() {
        // 30 80 ... : du BER. L'accepter ouvrirait une lecture qui cherche sa fin.
        let der = [0x30, 0x80, 0x02, 0x01, 0x01, 0x00, 0x00];
        assert_eq!(
            Lecteur::nouveau(&der).suivant(),
            Err(ErreurDer::LongueurIndefinie { position: 1 })
        );
    }

    #[test]
    fn une_longueur_non_minimale_est_refusee() {
        for der in [
            vec![0x04, 0x81, 0x05, 1, 2, 3, 4, 5],       // 81 05 au lieu de 05
            vec![0x04, 0x82, 0x00, 0x05, 1, 2, 3, 4, 5], // zéro de tête
            vec![0x04, 0xFF, 1],                          // réservé
        ] {
            assert!(
                matches!(
                    Lecteur::nouveau(&der).suivant(),
                    Err(ErreurDer::LongueurNonMinimale { .. })
                ),
                "doit refuser : {der:02x?}"
            );
        }
    }

    #[test]
    fn une_longueur_qui_deborde_est_tronquee_pas_paniquante() {
        // Annonce 200 octets, en fournit 2.
        let der = [0x04, 0x81, 0xC8, 1, 2];
        assert!(matches!(Lecteur::nouveau(&der).suivant(), Err(ErreurDer::Tronque { .. })));
    }

    #[test]
    fn la_forme_longue_detiquette_est_refusee() {
        let der = [0x1F, 0x81, 0x00, 0x00];
        assert_eq!(
            Lecteur::nouveau(&der).suivant(),
            Err(ErreurDer::EtiquetteLongue { position: 0 })
        );
    }

    #[test]
    fn un_tampon_vide_ne_panique_pas() {
        assert!(matches!(Lecteur::nouveau(&[]).suivant(), Err(ErreurDer::Tronque { .. })));
        assert!(Lecteur::nouveau(&[]).fin().is_ok());
        assert_eq!(Lecteur::nouveau(&[]).etiquette_suivante(), None);
    }

    #[test]
    fn limbrication_est_bornee() {
        let mut l = Lecteur::nouveau(&[]);
        for _ in 0..PROFONDEUR_MAX {
            l = l.entrer(&[]).expect("dans la limite");
        }
        assert_eq!(l.entrer(&[]).err(), Some(ErreurDer::TropProfond));
    }

    #[test]
    fn une_queue_silencieuse_est_refusee() {
        let mut der = ecrire(INTEGER, &[1]);
        der.extend_from_slice(&[0xDE, 0xAD]);
        let mut l = Lecteur::nouveau(&der);
        l.suivant().expect("premier élément");
        assert_eq!(l.fin(), Err(ErreurDer::OctetsEnTrop { restant: 2 }));
    }

    #[test]
    fn lecriture_emploie_la_forme_minimale() {
        assert_eq!(ecrire(OCTET_STRING, &[1, 2]), vec![0x04, 0x02, 1, 2]);
        let long = vec![7u8; 200];
        let e = ecrire(OCTET_STRING, &long);
        assert_eq!(&e[..3], &[0x04, 0x81, 0xC8], "forme longue sur UN octet");
        // Et ce qu'on écrit se relit.
        let mut l = Lecteur::nouveau(&e);
        assert_eq!(l.attendre(OCTET_STRING).expect("relu").len(), 200);
    }

    /// Un `0x80` de tête ferait un entier NÉGATIF en complément à deux. Certains horodateurs
    /// l'acceptent, d'autres le refusent — et le diagnostic serait introuvable.
    #[test]
    fn un_entier_positif_ne_devient_jamais_negatif() {
        assert_eq!(ecrire_entier_positif(&[0x80]), vec![0x02, 0x02, 0x00, 0x80]);
        assert_eq!(ecrire_entier_positif(&[0x7F]), vec![0x02, 0x01, 0x7F]);
        assert_eq!(ecrire_entier_positif(&[0x00, 0x00, 0x05]), vec![0x02, 0x01, 0x05], "zéros ôtés");
        assert_eq!(ecrire_entier_positif(&[0, 0, 0]), vec![0x02, 0x01, 0x00], "zéro reste zéro");
    }

    /// Le lecteur doit survivre à n'importe quoi — c'est du réseau. Aucun de ces tampons ne doit
    /// paniquer ; tous doivent rendre une erreur nommée.
    #[test]
    fn aucun_tampon_hostile_ne_fait_paniquer() {
        let hostiles: Vec<Vec<u8>> = vec![
            vec![],
            vec![0x30],
            vec![0x30, 0x00],
            vec![0x30, 0x7F],
            vec![0x30, 0x84, 0xFF, 0xFF, 0xFF, 0xFF],
            vec![0x30, 0x88, 255, 255, 255, 255, 255, 255, 255, 255],
            vec![0xFF; 64],
            vec![0x00; 64],
            (0u8..=255).collect(),
        ];
        for h in hostiles {
            let mut l = Lecteur::nouveau(&h);
            // On lit jusqu'à épuisement ou erreur : ni panique, ni boucle infinie.
            let mut tours = 0;
            while !l.fini() && l.suivant().is_ok() {
                tours += 1;
                assert!(tours < 1000, "boucle : {h:02x?}");
            }
        }
    }
}

// SPDX-License-Identifier: UNLICENSED
//! La **rotation de la clé tenant**, par générations.
//!
//! ## Le problème, énoncé exactement
//!
//! La clé tenant scelle la chaîne. La faire tourner naïvement — remplacer la clé — rend
//! **irrécupérable tout ce qui a déjà été scellé**, c'est-à-dire l'intégralité du registre que la
//! rotation prétendait protéger. Le jour où l'on fait tourner la clé pour la bonne raison (départ
//! d'un administrateur, exigence d'audit, suspicion de fuite), on détruirait la preuve.
//!
//! Une rotation utilisable est donc **additive**, jamais substitutive : on cesse de sceller avec
//! l'ancienne, on continue de lire ce qu'elle a scellé.
//!
//! ## Quatre états, et pourquoi pas trois
//!
//! Le trousseau des clés de cas de M6 en a trois — vivante, broyée, inconnue — et il a raison :
//! une clé de cas n'a aucune raison d'exister en lecture seule, puisqu'un cas clos ne reçoit plus
//! de nouveaux nœuds.
//!
//! Une clé de tenant, si. C'est **exactement** ce que la rotation ajoute, et c'est
//! [`EtatGeneration::Retiree`] : la clé ne scelle plus, mais elle re-prouve encore. Sans cet état,
//! « faire tourner » et « détruire » seraient le même geste, et la rotation resterait impraticable.
//!
//! ## Ce que ce module ne peut pas faire, structurellement
//!
//! Il ne peut pas écrire une clé. [`provenance_dag::TenantKey`] garde ses octets en `pub(crate)` :
//! aucun accesseur n'existe hors de S0. Ce crate détient donc des clés, les passe à la re-preuve, et
//! est **incapable** de les sérialiser — même par erreur, même sous la pression d'un correctif
//! pressé. L'invariant « une archive ne porte jamais sa clé » n'est pas une discipline qu'on tient,
//! c'est une opération que le langage refuse.
//!
//! ## Le piège des numéros réutilisés
//!
//! Une archive nomme sa génération par un **numéro**. Si un numéro pouvait resservir, une archive
//! disant « génération 3 » se résoudrait, des années plus tard, vers une *autre* clé — et la
//! restauration échouerait en accusant la chaîne au lieu d'accuser la nomenclature. [`TrousseauMaitre`]
//! impose donc une **stricte croissance** : un numéro déjà connu, fût-il détruit, ne revient jamais.

use std::collections::BTreeMap;
use std::fmt;

use provenance_dag::TenantKey;

/// L'état d'une génération de clé tenant. **Quatre**, pas trois.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EtatGeneration {
    /// Elle scelle le neuf **et** re-prouve l'ancien. Il y en a au plus une.
    Vivante {
        /// Depuis quand (secondes d'époque).
        depuis: u64,
    },
    /// Elle ne scelle plus rien, mais elle re-prouve encore. C'est l'état que la rotation crée.
    Retiree {
        /// Quand elle a cessé de sceller.
        le: u64,
    },
    /// Sa clé a été détruite, à cette date. Ce qu'elle a scellé ne se re-prouvera plus — et se lit
    /// « détruite le … », **jamais** « invalide ».
    Detruite {
        /// Quand.
        le: u64,
    },
    /// Ce trousseau ne connaît pas ce numéro. **Pas** une affirmation d'inexistence : une autre
    /// installation, ou un autre coffre, en détient peut-être la clé.
    Inconnue,
}

/// Une génération vivante ou retirée : son numéro, sa clé, et depuis quand.
#[derive(Debug, Clone)]
pub struct Generation {
    /// Le numéro, tel qu'une archive le nomme.
    pub numero: u32,
    /// L'instant de son entrée en service.
    pub depuis: u64,
    /// L'instant de son retrait, si elle est retirée.
    pub retiree_le: Option<u64>,
}

/// Pourquoi un geste de rotation a été refusé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefusRotation {
    /// Le numéro proposé n'est pas strictement supérieur à tous ceux déjà connus.
    NumeroNonCroissant { propose: u32, plus_haut_connu: u32 },
    /// Une génération vivante existe déjà — il faut la retirer d'abord, ou passer par [`TrousseauMaitre::tourner`].
    DejaUneVivante { numero: u32 },
    /// Le numéro visé n'est pas connu de ce trousseau.
    Inconnue { numero: u32 },
    /// Le geste demandé n'a pas de sens depuis l'état actuel.
    EtatIncompatible { numero: u32, etat: EtatGeneration },
}

impl fmt::Display for RefusRotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NumeroNonCroissant { propose, plus_haut_connu } => write!(
                f,
                "génération {propose} refusée : le trousseau connaît déjà {plus_haut_connu}. Réutiliser \
                 un numéro ferait pointer une archive ancienne vers une autre clé."
            ),
            Self::DejaUneVivante { numero } => write!(
                f,
                "la génération {numero} est encore vivante — deux vivantes rendraient « laquelle scelle ? » \
                 ambigu ; retirez-la, ou utilisez `tourner`"
            ),
            Self::Inconnue { numero } => write!(f, "la génération {numero} est inconnue de ce trousseau"),
            Self::EtatIncompatible { numero, etat } => {
                write!(f, "la génération {numero} est {etat:?} — ce geste n'a pas de sens depuis cet état")
            }
        }
    }
}

impl std::error::Error for RefusRotation {}

/// Le trousseau des générations de clé tenant — et la mémoire de celles qui ont été détruites.
///
/// Il retient les destructions **exprès**, comme le trousseau de cas retient les broyages : un
/// trousseau qui se contenterait d'oublier ne saurait plus distinguer « détruite » de « jamais vue »,
/// et perdrait la seule information qu'un régulateur demande — *quand*.
#[derive(Debug, Default)]
pub struct TrousseauMaitre {
    vivantes: BTreeMap<u32, (TenantKey, u64)>,
    retirees: BTreeMap<u32, (TenantKey, u64, u64)>, // clé, depuis, retirée_le
    detruites: BTreeMap<u32, u64>,
}

impl TrousseauMaitre {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Le plus haut numéro que ce trousseau ait jamais connu, tous états confondus.
    ///
    /// Les détruites comptent : c'est tout l'objet de la garde de croissance.
    #[must_use]
    pub fn plus_haut_connu(&self) -> Option<u32> {
        self.vivantes
            .keys()
            .chain(self.retirees.keys())
            .chain(self.detruites.keys())
            .copied()
            .max()
    }

    /// Installe une génération. Refuse un numéro non strictement croissant, ou une seconde vivante.
    pub fn poser(&mut self, numero: u32, cle: TenantKey, depuis: u64) -> Result<(), RefusRotation> {
        if let Some(h) = self.plus_haut_connu() {
            if numero <= h {
                return Err(RefusRotation::NumeroNonCroissant { propose: numero, plus_haut_connu: h });
            }
        }
        if let Some(v) = self.vivantes.keys().next().copied() {
            return Err(RefusRotation::DejaUneVivante { numero: v });
        }
        self.vivantes.insert(numero, (cle, depuis));
        Ok(())
    }

    /// Restaure une **pierre tombale** depuis un état persisté, sans jamais avoir tenu la clé.
    ///
    /// Le pendant exact de `Trousseau::poser_broye` de M6, et pour la même raison : un trousseau vit
    /// en mémoire, un redémarrage le reconstruit. Sans cette porte, une génération détruite avant
    /// l'arrêt reviendrait `Inconnue` — la destruction resterait effective, mais sa **date** serait
    /// perdue, et « détruite le 12 mars » redeviendrait « jamais vue ».
    pub fn poser_detruite(&mut self, numero: u32, le: u64) -> Result<(), RefusRotation> {
        if self.vivantes.contains_key(&numero)
            || self.retirees.contains_key(&numero)
            || self.detruites.contains_key(&numero)
        {
            return Err(RefusRotation::EtatIncompatible { numero, etat: self.etat(numero) });
        }
        self.detruites.insert(numero, le);
        Ok(())
    }

    /// Retire une génération : elle cesse de sceller, elle continue de re-prouver. **Rien n'est perdu.**
    pub fn retirer(&mut self, numero: u32, le: u64) -> Result<(), RefusRotation> {
        match self.vivantes.remove(&numero) {
            Some((cle, depuis)) => {
                self.retirees.insert(numero, (cle, depuis, le));
                Ok(())
            }
            None => Err(match self.etat(numero) {
                EtatGeneration::Inconnue => RefusRotation::Inconnue { numero },
                etat => RefusRotation::EtatIncompatible { numero, etat },
            }),
        }
    }

    /// **Détruit** une génération : sa clé disparaît, la date est retenue. Irréversible.
    ///
    /// C'est le crypto-shredding de M6 appliqué à une génération entière : ce qu'elle a scellé ne se
    /// re-prouvera plus, et devra se lire `Detruite{le}` — un fait daté — et non « invalide ».
    pub fn detruire(&mut self, numero: u32, le: u64) -> Result<(), RefusRotation> {
        if self.vivantes.remove(&numero).is_some() || self.retirees.remove(&numero).is_some() {
            self.detruites.insert(numero, le);
            return Ok(());
        }
        Err(match self.etat(numero) {
            EtatGeneration::Inconnue => RefusRotation::Inconnue { numero },
            etat => RefusRotation::EtatIncompatible { numero, etat },
        })
    }

    /// **Le geste de rotation** : retirer la vivante et installer la suivante, ou ne rien changer.
    ///
    /// Les deux vérifications sont faites **avant** la première mutation : un refus laisse le
    /// trousseau exactement dans l'état où il était. Une rotation à moitié faite — l'ancienne
    /// retirée, la nouvelle refusée — laisserait un service incapable de sceller quoi que ce soit.
    pub fn tourner(&mut self, numero: u32, cle: TenantKey, quand: u64) -> Result<(), RefusRotation> {
        if let Some(h) = self.plus_haut_connu() {
            if numero <= h {
                return Err(RefusRotation::NumeroNonCroissant { propose: numero, plus_haut_connu: h });
            }
        }
        let sortante = self.vivantes.keys().next().copied();
        if let Some(s) = sortante {
            self.retirer(s, quand)?;
        }
        self.vivantes.insert(numero, (cle, quand));
        Ok(())
    }

    /// L'état d'un numéro. Ne ment jamais par omission : l'absence rend [`EtatGeneration::Inconnue`].
    #[must_use]
    pub fn etat(&self, numero: u32) -> EtatGeneration {
        if let Some((_, depuis)) = self.vivantes.get(&numero) {
            return EtatGeneration::Vivante { depuis: *depuis };
        }
        if let Some((_, _, le)) = self.retirees.get(&numero) {
            return EtatGeneration::Retiree { le: *le };
        }
        if let Some(le) = self.detruites.get(&numero) {
            return EtatGeneration::Detruite { le: *le };
        }
        EtatGeneration::Inconnue
    }

    /// La génération qui scelle le neuf, s'il y en a une.
    #[must_use]
    pub fn courante(&self) -> Option<(u32, &TenantKey)> {
        self.vivantes.iter().next().map(|(n, (c, _))| (*n, c))
    }

    /// La clé qui **re-prouve** ce qu'une génération a scellé — vivante ou retirée, indifféremment.
    ///
    /// C'est la fonction qui porte tout le bénéfice de la rotation : après un tour de clé, les
    /// archives anciennes se restaurent **sans rien changer**, parce que leur génération est
    /// seulement retirée.
    #[must_use]
    pub fn cle_de_lecture(&self, numero: u32) -> Option<&TenantKey> {
        self.vivantes
            .get(&numero)
            .map(|(c, _)| c)
            .or_else(|| self.retirees.get(&numero).map(|(c, _, _)| c))
    }

    /// L'inventaire lisible des générations qui portent encore une clé.
    #[must_use]
    pub fn inventaire(&self) -> Vec<Generation> {
        let mut v: Vec<Generation> = self
            .vivantes
            .iter()
            .map(|(n, (_, d))| Generation { numero: *n, depuis: *d, retiree_le: None })
            .chain(
                self.retirees
                    .iter()
                    .map(|(n, (_, d, r))| Generation { numero: *n, depuis: *d, retiree_le: Some(*r) }),
            )
            .collect();
        v.sort_by_key(|g| g.numero);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cle(n: u8) -> TenantKey {
        TenantKey::from_bytes([n; 32])
    }

    #[test]
    fn une_rotation_ne_detruit_rien() {
        let mut t = TrousseauMaitre::new();
        t.poser(1, cle(1), 100).unwrap();
        t.tourner(2, cle(2), 200).unwrap();
        assert_eq!(t.etat(1), EtatGeneration::Retiree { le: 200 });
        assert!(t.cle_de_lecture(1).is_some(), "la génération retirée RE-PROUVE encore");
        assert_eq!(t.courante().map(|(n, _)| n), Some(2), "c'est la nouvelle qui scelle");
    }

    #[test]
    fn deux_vivantes_sont_impossibles() {
        let mut t = TrousseauMaitre::new();
        t.poser(1, cle(1), 100).unwrap();
        assert!(matches!(t.poser(2, cle(2), 200), Err(RefusRotation::DejaUneVivante { numero: 1 })));
    }

    #[test]
    fn un_numero_ne_resservira_jamais_meme_apres_destruction() {
        let mut t = TrousseauMaitre::new();
        t.poser(7, cle(7), 100).unwrap();
        t.detruire(7, 300).unwrap();
        assert!(
            matches!(t.poser(7, cle(9), 400), Err(RefusRotation::NumeroNonCroissant { .. })),
            "réutiliser 7 ferait pointer une archive ancienne vers une AUTRE clé"
        );
        assert_eq!(t.etat(7), EtatGeneration::Detruite { le: 300 }, "et la pierre tombale reste");
    }

    #[test]
    fn detruite_n_est_pas_inconnue() {
        let mut t = TrousseauMaitre::new();
        t.poser(1, cle(1), 100).unwrap();
        t.detruire(1, 250).unwrap();
        assert_eq!(t.etat(1), EtatGeneration::Detruite { le: 250 });
        assert_eq!(t.etat(2), EtatGeneration::Inconnue);
        assert!(t.cle_de_lecture(1).is_none(), "détruite : plus de clé");
    }

    #[test]
    fn la_pierre_tombale_se_restaure_avec_sa_date() {
        let mut t = TrousseauMaitre::new();
        t.poser_detruite(4, 999).unwrap();
        assert_eq!(t.etat(4), EtatGeneration::Detruite { le: 999 }, "la DATE survit au redémarrage");
    }

    #[test]
    fn une_rotation_refusee_ne_laisse_pas_le_trousseau_a_moitie_tourne() {
        let mut t = TrousseauMaitre::new();
        t.poser(5, cle(5), 100).unwrap();
        let refus = t.tourner(3, cle(3), 200);
        assert!(matches!(refus, Err(RefusRotation::NumeroNonCroissant { .. })));
        assert_eq!(t.etat(5), EtatGeneration::Vivante { depuis: 100 }, "la sortante n'a PAS été retirée");
        assert_eq!(t.courante().map(|(n, _)| n), Some(5), "le service peut encore sceller");
    }

    #[test]
    fn retirer_une_inconnue_le_dit_au_lieu_de_reussir_en_silence() {
        let mut t = TrousseauMaitre::new();
        assert!(matches!(t.retirer(1, 100), Err(RefusRotation::Inconnue { numero: 1 })));
    }

    #[test]
    fn l_inventaire_ne_montre_que_ce_qui_porte_une_cle() {
        let mut t = TrousseauMaitre::new();
        t.poser(1, cle(1), 10).unwrap();
        t.tourner(2, cle(2), 20).unwrap();
        t.tourner(3, cle(3), 30).unwrap();
        t.detruire(1, 40).unwrap();
        let inv = t.inventaire();
        assert_eq!(inv.iter().map(|g| g.numero).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(inv[0].retiree_le, Some(30), "2 a été retirée quand 3 est arrivée");
        assert_eq!(inv[1].retiree_le, None, "3 est la vivante");
    }
}

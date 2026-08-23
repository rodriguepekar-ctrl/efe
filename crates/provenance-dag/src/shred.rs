// SPDX-License-Identifier: UNLICENSED
//! # `shred` — M6 : la conservation limitée, sur une chaîne qui ne se coupe pas.
//!
//! ## Le problème, énoncé exactement
//!
//! Une chaîne scellée ne tolère pas la suppression. Ici, le repliement est
//! `chaîne ← HMAC(clé, chaîne ‖ octets_canoniques(nœud))` : le sceau porte sur les **octets
//! entiers** du nœud, pas sur une empreinte. Retirer un record casse donc son sceau **et tous ceux
//! qui suivent**, et `reprove_dag` refuse en plus les références devenues pendantes.
//!
//! « Un cas expiré disparaît, et tout le reste se re-prouve encore » ressemble donc à une demande
//! contradictoire. Elle ne l'est pas — à condition de faire disparaître autre chose que le record.
//!
//! ## Le crypto-shredding : ce que le produit avait DÉJÀ promis
//!
//! Cette réponse n'est pas un choix ouvert. `reg-evidence` la publie depuis longtemps dans un
//! artefact qui part chez des régulateurs :
//!
//! > *« L'articulation de l'append-only avec le droit à l'effacement (art. 17) est le
//! > **crypto-shredding** (raffinement nommé coordonné S0/S1) — pas une "compatibilité effacement"
//! > du DAG tel quel. »*
//!
//! Implémenter autre chose — une purge par préfixe, par exemple, qui marcherait très bien — aurait
//! fait **mentir cet artefact**. Ce module est le versant S0 du raffinement annoncé.
//!
//! ## Comment il tient
//!
//! Les champs effaçables ne sont pas scellés en clair : ils sont scellés **chiffrés**. La chaîne
//! porte donc du chiffré, qui ne change jamais. Broyer, c'est détruire la clé du cas :
//!
//! * le contenu devient irrécupérable — c'est le « disparaît » ;
//! * **pas un octet du DAG ne bouge** — c'est le « tout le reste se re-prouve encore » ;
//! * et `verify-mini`, le vérifieur tiers, n'a **rien** à changer : il recalcule le sceau sur des
//!   champs dont il n'a jamais eu à comprendre le contenu.
//!
//! Ce dernier point n'est pas un détail de commodité. Si un cas broyé n'était vérifiable que par
//! nous, la promesse du produit — *un tiers re-prouve sans nous croire* — tomberait précisément le
//! jour où l'on exerce un droit à l'effacement.
//!
//! ## Le piège qui rendrait tout cela théâtral
//!
//! **La clé d'un cas doit être ALÉATOIRE, jamais dérivée.** Une clé dérivée
//! (`HMAC(clé_tenant, cas)`) se re-dérive : la « broyer » ne détruirait rien du tout, puisque
//! quiconque détient la clé tenant la reconstruit en une ligne. Ce serait un effacement de façade —
//! le pire résultat possible, parce qu'il serait de bonne foi et documenté comme un effacement.
//!
//! [`CaseKey`] ne peut donc être construite que depuis des octets **fournis par l'appelant**, et
//! aucune fonction de ce module ne dérive de clé de cas. Un test le fige.
//!
//! ## Ce que ce module NE fait PAS, et qu'il faut dire
//!
//! * **Le nœud reste.** Son identifiant, son rôle, son horodatage, ses arêtes demeurent — c'est
//!   ce qui permet à la chaîne de tenir. « Disparaître » veut dire *le contenu*, pas *la trace*.
//!   Un auditeur voit toujours qu'un acte a eu lieu à telle date ; il ne peut plus lire ce qu'il
//!   disait. C'est exactement ce que la limitation de conservation demande, et rien de plus.
//! * **Rien ne protège le passé.** Un record scellé avant l'activation d'un trousseau porte son
//!   contenu **en clair, pour toujours** : le re-chiffrer changerait ses octets, donc son sceau,
//!   donc la chaîne. Le crypto-shredding protège ce qui est capturé **après** son activation.
//! * **Ce n'est pas une preuve de destruction.** Détruire une clé en mémoire ou dans un fichier
//!   n'est pas démontrable à un tiers ; sauvegardes et instantanés peuvent la conserver ailleurs.
//!   Ce module rend le contenu irrécupérable *par cette chaîne* — la discipline des copies reste
//!   une affaire d'exploitation.

use std::collections::BTreeMap;

use crate::hmac::hmac_sha256;

/// Le préfixe qui rend une enveloppe **reconnaissable**.
///
/// Sans lui, un champ broyé serait indistinguable d'un champ dont le contenu ressemble à du
/// hasard — et un lecteur conclurait à une corruption. Le préfixe permet de dire « ceci a été
/// chiffré », donc « ceci a été délibérément soustrait », ce qui est une information et non un
/// accident.
pub const ENVELOPPE: &str = "shred:v1:";

/// Le contexte de dérivation du flot de clé. Versionné : changer la construction changerait tous
/// les chiffrés, donc tous les sceaux, donc toutes les chaînes.
const CTX_FLOT: &[u8] = b"efe-prov-dag:shred:v1:keystream:";

/// La clé d'**un cas**. Trente-deux octets, **aléatoires**.
///
/// Aucune fonction de ce module ne la dérive : voir la note du module sur le piège de la clé
/// dérivée. L'appelant fournit l'entropie, et c'est à lui de garantir qu'elle en est.
#[derive(Clone, PartialEq, Eq)]
pub struct CaseKey([u8; 32]);

impl CaseKey {
    /// Depuis des octets **aléatoires** fournis par l'appelant.
    #[must_use]
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }
}

// Jamais de `Debug` qui imprime les octets : une clé de cas dans un journal, c'est un effacement
// annulé par une trace d'exploitation. Même discipline que `TenantKey`.
impl std::fmt::Debug for CaseKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CaseKey(…)")
    }
}

/// Le flot de clé : `HMAC(clé, CTX ‖ contexte ‖ compteur)`, par blocs de 32 octets.
///
/// Le **contexte** — en pratique `node_id:champ` — rend le flot unique par champ. C'est la seule
/// chose qui compte pour la sûreté de cette construction : réutiliser un flot sur deux clairs
/// différents les révélerait tous les deux par simple XOR. Les identifiants de nœud étant uniques
/// dans un DAG (`DuplicateId` le garantit à l'append), deux champs distincts ne peuvent pas
/// partager un contexte.
///
/// Le chiffrement est **déterministe**, et c'est voulu : rejouer une capture doit produire les
/// mêmes octets, sinon la chaîne ne serait pas re-dérivable.
fn flot(cle: &CaseKey, contexte: &str, longueur: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(longueur);
    let mut compteur: u32 = 0;
    while out.len() < longueur {
        let mut msg = Vec::with_capacity(CTX_FLOT.len() + contexte.len() + 4);
        msg.extend_from_slice(CTX_FLOT);
        msg.extend_from_slice(contexte.as_bytes());
        msg.extend_from_slice(&compteur.to_be_bytes());
        out.extend_from_slice(&hmac_sha256(&cle.0, &msg));
        compteur += 1;
    }
    out.truncate(longueur);
    out
}

fn en_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn de_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
}

/// Chiffre un champ. Le résultat est ce qui sera **scellé** dans le nœud.
///
/// Aucune authentification n'est ajoutée, et ce n'est pas un oubli : le chiffré est immédiatement
/// scellé par la chaîne HMAC du DAG, qui détecte toute altération. Ajouter un second tag
/// authentifierait deux fois la même chose et ferait croire à une garantie supplémentaire.
#[must_use]
pub fn sceller(cle: &CaseKey, contexte: &str, clair: &str) -> String {
    let ks = flot(cle, contexte, clair.len());
    let ct: Vec<u8> = clair.as_bytes().iter().zip(&ks).map(|(a, b)| a ^ b).collect();
    format!("{ENVELOPPE}{}", en_hex(&ct))
}

/// Ce champ est-il une enveloppe ? Se répond **sans clé**, ce qui permet à un lecteur qui n'a rien
/// de savoir qu'il regarde un contenu soustrait et non un contenu absurde.
#[must_use]
pub fn est_enveloppe(s: &str) -> bool {
    s.starts_with(ENVELOPPE)
}

/// Déchiffre, si l'entrée est bien une enveloppe et que la clé est la bonne.
///
/// Rend `None` quand ce n'est pas une enveloppe, quand le hex est malformé, ou quand le clair
/// obtenu n'est pas de l'UTF-8 — ce dernier cas signalant une **mauvaise clé**, puisque le flot est
/// pseudo-aléatoire et qu'un XOR avec le mauvais flot ne produit presque jamais du texte valide.
#[must_use]
pub fn ouvrir(cle: &CaseKey, contexte: &str, enveloppe: &str) -> Option<String> {
    let hex = enveloppe.strip_prefix(ENVELOPPE)?;
    let ct = de_hex(hex)?;
    let ks = flot(cle, contexte, ct.len());
    let clair: Vec<u8> = ct.iter().zip(&ks).map(|(a, b)| a ^ b).collect();
    String::from_utf8(clair).ok()
}

/// L'état d'un cas dans le trousseau. **Trois** états, pas deux.
///
/// « Broyé » et « inconnu » ne doivent jamais se confondre : le premier est un fait — ce cas a
/// existé et son contenu a été délibérément détruit à telle date — le second est une absence. Les
/// confondre ferait dire « ce cas n'a jamais existé » d'un cas qu'un régulateur a vu passer, ce qui
/// est exactement le défaut que la passerelle a corrigé en distinguant 410 de 404.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EtatCas {
    /// La clé est là : le contenu se lit.
    Vivant {
        /// Quand le cas a été ouvert (secondes d'époque) — la base du calcul d'expiration.
        depuis: u64,
    },
    /// La clé a été détruite, à cette date. Le contenu est irrécupérable.
    Broye { le: u64 },
    /// Ce trousseau ne connaît pas ce cas. **Pas** une affirmation d'inexistence.
    Inconnu,
}

/// Ce qu'une lecture de champ donne — et pourquoi, quand elle ne donne rien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lecture {
    /// Le champ n'était pas chiffré (capturé avant l'activation du trousseau, ou non effaçable).
    EnClair(String),
    /// Déchiffré.
    Ouvert(String),
    /// Enveloppe reconnue, clé **détruite** à cette date. Le contenu ne reviendra pas.
    Broye { le: u64 },
    /// Enveloppe reconnue, mais ce trousseau ne connaît pas ce cas — il manque peut-être une clé
    /// que quelqu'un d'autre détient. Distinct de [`Lecture::Broye`], qui est définitif.
    CleAbsente,
    /// Enveloppe malformée, ou clé qui ne convient pas.
    Illisible,
}

/// Le trousseau des clés de cas — et la mémoire de ce qui a été broyé.
///
/// Il retient les broyages **exprès**. Un trousseau qui se contenterait d'oublier ne saurait plus
/// distinguer « détruit » de « jamais vu », et perdrait la seule information qu'un régulateur
/// demande vraiment : *quand* la suppression a eu lieu.
#[derive(Debug, Default)]
pub struct Trousseau {
    vivants: BTreeMap<String, (CaseKey, u64)>,
    broyes: BTreeMap<String, u64>,
}

impl Trousseau {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Confie la clé d'un cas au trousseau. `depuis` = l'ouverture du cas (époque, secondes).
    ///
    /// **Un cas ne s'ouvre qu'une fois.** Reposer une clé est refusé dans les deux situations, pour
    /// deux raisons qui se ressemblent moins qu'il n'y paraît :
    ///
    /// * sur un cas **broyé**, ce serait prétendre ressusciter un contenu déclaré détruit — le
    ///   trousseau n'a aucun moyen de savoir si c'est la même clé, et la date de destruction, elle,
    ///   a déjà été communiquée ;
    /// * sur un cas **vivant**, une clé différente rendrait illisible tout ce qui a déjà été chiffré
    ///   avec la première — une destruction aussi complète qu'un broyage, mais **silencieuse et non
    ///   enregistrée**. Les champs se liraient alors `Illisible`, jamais `Broye` : une suppression
    ///   réelle déguisée en corruption, c'est-à-dire le pire des deux mondes.
    ///
    /// Rend `false` sans rien changer, plutôt que d'écraser.
    pub fn poser(&mut self, cas: &str, cle: CaseKey, depuis: u64) -> bool {
        if self.broyes.contains_key(cas) || self.vivants.contains_key(cas) {
            return false;
        }
        self.vivants.insert(cas.to_string(), (cle, depuis));
        true
    }

    /// **Broie** un cas : la clé est détruite, la date retenue. Irréversible.
    ///
    /// Rend `false` si le cas était inconnu ou déjà broyé — un broyage qui n'a rien détruit ne doit
    /// pas se déclarer réussi.
    pub fn broyer(&mut self, cas: &str, le: u64) -> bool {
        if self.vivants.remove(cas).is_none() {
            return false;
        }
        self.broyes.insert(cas.to_string(), le);
        true
    }

    #[must_use]
    pub fn etat(&self, cas: &str) -> EtatCas {
        if let Some((_, depuis)) = self.vivants.get(cas) {
            return EtatCas::Vivant { depuis: *depuis };
        }
        match self.broyes.get(cas) {
            Some(le) => EtatCas::Broye { le: *le },
            None => EtatCas::Inconnu,
        }
    }

    /// Lit un champ dans le contexte d'un cas, en disant **pourquoi** quand elle échoue.
    #[must_use]
    pub fn lire(&self, cas: &str, contexte: &str, champ: &str) -> Lecture {
        if !est_enveloppe(champ) {
            return Lecture::EnClair(champ.to_string());
        }
        match self.etat(cas) {
            EtatCas::Vivant { .. } => {
                let (cle, _) = self.vivants.get(cas).expect("vivant ⟹ présent");
                match ouvrir(cle, contexte, champ) {
                    Some(c) => Lecture::Ouvert(c),
                    None => Lecture::Illisible,
                }
            }
            EtatCas::Broye { le } => Lecture::Broye { le },
            EtatCas::Inconnu => Lecture::CleAbsente,
        }
    }

    /// Chiffre un champ pour un cas — **sans jamais rendre la clé**, qui ne sort pas du trousseau.
    ///
    /// `None` si le cas n'est pas vivant. On refuse d'enrober pour un cas broyé : ce serait produire
    /// un chiffré que plus personne ne pourra jamais ouvrir, c'est-à-dire une donnée perdue déguisée
    /// en donnée protégée. Et pour un cas inconnu, on ne peut pas inventer une clé sans entropie.
    #[must_use]
    pub fn sceller_pour(&self, cas: &str, contexte: &str, clair: &str) -> Option<String> {
        let (cle, _) = self.vivants.get(cas)?;
        Some(sceller(cle, contexte, clair))
    }

    /// Les cas vivants dont l'âge dépasse l'horizon — ceux qu'une politique de conservation doit
    /// broyer. Triés, donc déterministe.
    #[must_use]
    pub fn expires(&self, horizon_s: u64, maintenant: u64) -> Vec<String> {
        self.vivants
            .iter()
            .filter(|(_, (_, depuis))| maintenant.saturating_sub(*depuis) >= horizon_s)
            .map(|(c, _)| c.clone())
            .collect()
    }

    /// Broie tout ce qui a expiré. Rend les cas broyés, dans l'ordre.
    pub fn broyer_expires(&mut self, horizon_s: u64, maintenant: u64) -> Vec<String> {
        let cibles = self.expires(horizon_s, maintenant);
        for c in &cibles {
            self.broyer(c, maintenant);
        }
        cibles
    }

    /// Combien de cas vivants, combien de broyés — la grandeur d'exploitation.
    #[must_use]
    pub fn compte(&self) -> (usize, usize) {
        (self.vivants.len(), self.broyes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cle(n: u8) -> CaseKey {
        CaseKey::from_bytes([n; 32])
    }

    #[test]
    fn un_aller_retour_rend_le_clair() {
        let k = cle(7);
        let e = sceller(&k, "SUP-2024::sanction:output", "PASS");
        assert!(est_enveloppe(&e), "reconnaissable sans clé");
        assert_ne!(e, "PASS");
        assert_eq!(ouvrir(&k, "SUP-2024::sanction:output", &e).as_deref(), Some("PASS"));
    }

    #[test]
    fn le_chiffrement_est_deterministe() {
        // Rejouer une capture doit produire les MÊMES octets, sinon la chaîne ne serait pas
        // re-dérivable et le reçu cesserait d'être une fonction de ses entrées.
        let k = cle(3);
        assert_eq!(sceller(&k, "ctx", "décision"), sceller(&k, "ctx", "décision"));
    }

    #[test]
    fn deux_contextes_ne_partagent_jamais_leur_flot() {
        // La seule propriété qui compte pour cette construction : réutiliser un flot sur deux
        // clairs les révélerait tous les deux par simple XOR.
        let k = cle(11);
        let a = sceller(&k, "cas::etape1:output", "MEME TEXTE");
        let b = sceller(&k, "cas::etape2:output", "MEME TEXTE");
        assert_ne!(a, b, "même clair, contextes différents ⟹ chiffrés différents");
    }

    #[test]
    fn une_mauvaise_cle_ne_rend_pas_un_faux_clair() {
        let e = sceller(&cle(1), "ctx", "revue conforme");
        // Le flot est pseudo-aléatoire : XOR avec le mauvais flot ne produit presque jamais de
        // l'UTF-8 valide. On refuse plutôt que de rendre du texte inventé.
        assert!(ouvrir(&cle(2), "ctx", &e).is_none());
        assert!(ouvrir(&cle(1), "autre-ctx", &e).is_none());
    }

    #[test]
    fn ce_qui_nest_pas_une_enveloppe_se_lit_en_clair() {
        let mut t = Trousseau::new();
        t.poser("cas", cle(4), 0);
        assert_eq!(t.lire("cas", "ctx", "PASS"), Lecture::EnClair("PASS".into()));
    }

    /// **Le cœur de M6** : broyer détruit le contenu, et le trousseau sait DIRE qu'il l'a détruit.
    #[test]
    fn broyer_detruit_le_contenu_et_le_dit() {
        let mut t = Trousseau::new();
        t.poser("SUP-2024", cle(9), 100);
        let e = sceller(&cle(9), "SUP-2024::x:output", "COMPLIANT");

        assert_eq!(t.lire("SUP-2024", "SUP-2024::x:output", &e), Lecture::Ouvert("COMPLIANT".into()));
        assert!(t.broyer("SUP-2024", 500));
        assert_eq!(
            t.lire("SUP-2024", "SUP-2024::x:output", &e),
            Lecture::Broye { le: 500 },
            "le contenu est parti, et la DATE reste — c'est elle qu'un régulateur demande"
        );
    }

    /// « Broyé » et « inconnu » ne se confondent jamais — même distinction que 410 vs 404 dans la
    /// passerelle : l'un est un fait, l'autre une absence.
    #[test]
    fn broye_et_inconnu_ne_se_confondent_pas() {
        let mut t = Trousseau::new();
        t.poser("connu", cle(5), 0);
        t.broyer("connu", 42);
        let e = sceller(&cle(5), "ctx", "x");

        assert_eq!(t.lire("connu", "ctx", &e), Lecture::Broye { le: 42 });
        assert_eq!(t.lire("jamais-vu", "ctx", &e), Lecture::CleAbsente);
        assert_eq!(t.etat("connu"), EtatCas::Broye { le: 42 });
        assert_eq!(t.etat("jamais-vu"), EtatCas::Inconnu);
    }

    #[test]
    fn un_broyage_qui_ne_detruit_rien_ne_se_declare_pas_reussi() {
        let mut t = Trousseau::new();
        assert!(!t.broyer("absent", 1), "rien à détruire");
        t.poser("cas", cle(1), 0);
        assert!(t.broyer("cas", 2));
        assert!(!t.broyer("cas", 3), "déjà broyé");
    }

    /// Reposer une clé sur un cas VIVANT est refusé : une clé différente rendrait illisible tout ce
    /// qui a été chiffré avec la première — une destruction complète, silencieuse, et que rien
    /// n'enregistrerait. Les champs se liraient `Illisible`, jamais `Broye`.
    #[test]
    fn on_necrase_pas_la_cle_dun_cas_vivant() {
        let mut t = Trousseau::new();
        assert!(t.poser("cas", cle(1), 0));
        let e = sceller(&cle(1), "ctx", "COMPLIANT");

        assert!(!t.poser("cas", cle(2), 50), "une seconde clé écraserait la première en silence");
        assert_eq!(t.etat("cas"), EtatCas::Vivant { depuis: 0 }, "la date d'ouverture non plus ne bouge pas");
        assert_eq!(t.lire("cas", "ctx", &e), Lecture::Ouvert("COMPLIANT".into()), "le contenu se lit encore");
    }

    #[test]
    fn on_ne_ressuscite_pas_un_cas_broye() {
        let mut t = Trousseau::new();
        t.poser("cas", cle(1), 0);
        t.broyer("cas", 10);
        assert!(!t.poser("cas", cle(1), 20), "reposer une clé annulerait une suppression déclarée");
        assert_eq!(t.etat("cas"), EtatCas::Broye { le: 10 });
    }

    #[test]
    fn lhorizon_choisit_les_expires_et_eux_seuls() {
        let mut t = Trousseau::new();
        t.poser("vieux", cle(1), 0);
        t.poser("recent", cle(2), 900);
        // horizon 500 s, il est 1000 : « vieux » a 1000 s, « recent » en a 100.
        assert_eq!(t.expires(500, 1000), vec!["vieux".to_string()]);
        let broyes = t.broyer_expires(500, 1000);
        assert_eq!(broyes, vec!["vieux".to_string()]);
        assert_eq!(t.etat("vieux"), EtatCas::Broye { le: 1000 });
        assert!(matches!(t.etat("recent"), EtatCas::Vivant { .. }), "le récent survit");
        assert_eq!(t.compte(), (1, 1));
    }

    /// **Le piège qui rendrait M6 théâtral.** Une clé DÉRIVÉE se re-dérive : la broyer ne détruirait
    /// rien. Ce test ne vérifie pas du code — il fige une propriété de l'API : aucune fonction de ce
    /// module ne fabrique de clé de cas. La seule voie est `from_bytes`, où l'entropie vient de
    /// l'appelant.
    #[test]
    fn aucune_cle_de_cas_ne_se_derive_dans_ce_module() {
        let src = include_str!("shred.rs");
        let hors_tests = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];

        // Une clé ne se fabrique que par le constructeur tuple `Self(…)`. Il doit y en avoir
        // exactement UN : celui de `from_bytes`, dont les octets viennent de l'appelant.
        assert_eq!(
            hors_tests.matches("Self(").count(),
            1,
            "une seule construction de CaseKey doit exister (celle de `from_bytes`) : toute autre \
             serait une DÉRIVATION, et une clé dérivée se re-dérive — le broyage ne détruirait rien"
        );

        // Et aucun nom de fonction en forme de dérivation ne doit apparaître : c'est par là que la
        // commodité reviendrait (« juste un helper pour fabriquer la clé depuis le tenant »).
        for interdit in ["fn derive", "fn from_seed", "fn from_secret", "fn from_tenant"] {
            assert!(
                !hors_tests.contains(interdit),
                "`{interdit}` fabriquerait une clé de cas re-dérivable : le broyage deviendrait du théâtre"
            );
        }

        // Ce garde est TEXTUEL. Il attrape la régression, pas l'ingéniosité : rien n'empêche un
        // appelant de fournir à `from_bytes` des octets qu'il a lui-même dérivés. C'est pour cela
        // que l'exigence d'entropie est écrite dans la doc du type, à l'endroit où on la lit.
    }
}

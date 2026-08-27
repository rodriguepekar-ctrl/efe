// SPDX-License-Identifier: UNLICENSED
//! # `trousseau_fichier` — le trousseau de M6, **rendu durable**.
//!
//! ## Le trou que ce module comble
//!
//! M6 a livré le crypto-shredding : les champs effaçables sont scellés chiffrés, et broyer détruit
//! la clé du cas sans déplacer un octet du DAG. Il déclarait honnêtement sa limite — *« le trousseau
//! ne se recharge pas au redémarrage ; sa persistance est le geste du déployeur »*.
//!
//! Puis M7 a bâti un chemin de **réhydratation** (`records` dans la déclaration de déploiement). Les
//! deux mis ensemble donnent un service qui recharge des records enrobés **sans leurs clés** : tout
//! se lit `CleAbsente`, à chaque redémarrage, et rien ne le dit au démarrage. Aucun des deux modules
//! ne l'a vu, parce que chacun avait raison de son côté.
//!
//! ## Pourquoi ce module CHARGE mais ne CRÉE jamais
//!
//! [`provenance_dag::shred::CaseKey`] n'expose que `from_bytes` : **aucun accesseur d'octets**,
//! délibérément — la même discipline que `TenantKey`. On ne peut donc pas écrire une clé qu'on
//! détient, seulement lire celles qu'on nous donne. Ce n'est pas une gêne, c'est le dessin : une clé
//! de cas doit être **aléatoire**, et l'entropie appartient à l'exploitant. Ce module lit son
//! fichier ; il n'en fabrique jamais le contenu.
//!
//! ## Un broyage qui n'est pas durable est un MENSONGE sur une destruction
//!
//! C'est la décision centrale de ce module. Broyer en mémoire pendant que la clé reste sur le
//! disque, c'est déclarer détruit ce qui est intact — et le redémarrage suivant la ressusciterait.
//! Alors [`retirer`] réécrit le fichier **sans** les cas broyés, et si elle échoue, le balayage
//! **refuse** au lieu de broyer en mémoire et de se taire (règle de M3 : dégrader produit-il quelque
//! chose d'utile *et honnête* ? ici, non).
//!
//! La réécriture est **atomique** — fichier temporaire, puis `rename`. Une écriture en place
//! interrompue par une panne perdrait des clés que personne n'a demandé de détruire : un
//! effacement de masse par accident, dans le module dont tout le propos est l'effacement voulu.
//!
//! ## Le format
//!
//! Une ligne par cas, trois champs séparés par des blancs. `#` commente. Générable à la main :
//!
//! ```text
//! # cas        clé (64 hex) ou BROYE                                             ouvert_à / broyé_le
//! SUP-2024     3f9a...c1                                                         1735689600
//! SUP-2023     BROYE                                                             1742000000
//! ```
//!
//! Un broyage **remplace la clé par `BROYE`** au lieu de supprimer la ligne. La différence n'est pas
//! cosmétique : supprimer perdrait la **date de destruction**, et « détruit le 12 mars »
//! redeviendrait « jamais vu » au redémarrage suivant. Or c'est cette date qu'un régulateur demande,
//! et c'est la distinction que M6 existe pour tenir (`Broye{le}` ≠ `Inconnu`). Une destruction
//! durable qui efface son propre procès-verbal n'est pas durable.
//!
//! Le fichier **est un secret** : il porte les clés. Il se monte comme tel, et [`charger`] crie si
//! ses permissions le rendent lisible par n'importe qui.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use provenance_dag::shred::CaseKey;

/// Une entrée du fichier : un cas, sa clé, l'instant de son ouverture.
///
/// `Debug` est sûr ici — et il fallait le vérifier avant de le dériver : `CaseKey` a un `Debug`
/// écrit à la main qui rend `CaseKey(…)`, précisément pour qu'une clé de cas ne puisse pas se
/// retrouver dans un journal. Un effacement annulé par une trace d'exploitation resterait un
/// effacement annulé.
#[derive(Debug)]
pub struct Entree {
    pub cas: String,
    pub cle: CaseKey,
    pub depuis: u64,
}

/// Ce qui peut mal se passer — chaque cas nommé, parce que les gestes qui les corrigent diffèrent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Erreur {
    /// Le fichier n'est pas lisible.
    Illisible { chemin: String, cause: String },
    /// Une ligne ne suit pas le format.
    Ligne { chemin: String, n: usize, pourquoi: String },
    /// Une clé n'est pas 64 caractères hexadécimaux.
    CleInvalide { chemin: String, n: usize, cas: String },
    /// Deux lignes déclarent le même cas — laquelle fait foi ?
    CasDuplique { chemin: String, cas: String },
    /// La réécriture (donc la **destruction**) a échoué.
    EcritureImpossible { chemin: String, cause: String },
}

impl fmt::Display for Erreur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Erreur::Illisible { chemin, cause } => {
                write!(f, "trousseau « {chemin} » illisible : {cause}")
            }
            Erreur::Ligne { chemin, n, pourquoi } => {
                write!(f, "{chemin}:{n} — {pourquoi} (attendu : « cas  clé_hex64  ouvert_à »)")
            }
            Erreur::CleInvalide { chemin, n, cas } => write!(
                f,
                "{chemin}:{n} — la clé du cas « {cas} » doit faire 64 caractères hexadécimaux. \
                 Générer : openssl rand -hex 32"
            ),
            Erreur::CasDuplique { chemin, cas } => write!(
                f,
                "{chemin} — le cas « {cas} » est déclaré deux fois : la seconde clé rendrait \
                 illisible tout ce qui a été chiffré avec la première, en silence"
            ),
            Erreur::EcritureImpossible { chemin, cause } => write!(
                f,
                "IMPOSSIBLE DE RÉÉCRIRE « {chemin} » : {cause}. Le broyage est ANNULÉ — broyer en \
                 mémoire pendant que la clé reste sur le disque déclarerait détruit ce qui est \
                 intact, et le redémarrage suivant la ressusciterait"
            ),
        }
    }
}

/// Le marqueur d'une clé détruite, à la place de ses 64 caractères hexadécimaux.
pub const BROYE: &str = "BROYE";

/// Ce que [`charger`] rend — **nommé**. Le triplet anonyme d'avant perdait exactement ce qu'il
/// fallait garder : lequel des deux `Vec` porte les morts, et ce qu'affirmait un `bool` nu.
///
/// `Debug` est sûr pour la même raison que sur [`Entree`], et il fallait la revérifier ici : la
/// seule clé traversée est un `CaseKey`, dont le `Debug` écrit à la main rend `CaseKey(…)`.
#[derive(Debug)]
pub struct Chargement {
    /// Les cas **vivants** : ceux dont la clé est encore sur le disque, donc encore lisibles.
    pub vivantes: Vec<Entree>,
    /// Les **pierres tombales** — `(cas, date de destruction)`. Un cas broyé n'a plus de clé mais
    /// garde sa date : c'est la différence entre « détruit le 12 mars » et « jamais vu », et c'est
    /// cette date qu'un régulateur demande.
    pub broyes: Vec<(String, u64)>,
    /// Le fichier est **accessible au-delà de son propriétaire** : sous Unix, au moins un bit de
    /// permission groupe/autrui est posé (`mode & 0o077`, pas seulement la lecture).
    ///
    /// C'est un **avertissement que [`charger`] remonte sans refuser** — voir la raison sur
    /// [`charger`]. L'appelant crie ; le chargement, lui, a bien eu lieu.
    ///
    /// Vaut `false` quand on n'a pas pu regarder (hors Unix, ou `metadata` en échec) : c'est une
    /// absence de mesure, pas une preuve de permissions saines.
    pub permissions_trop_ouvertes: bool,
}

/// Charge un trousseau : ses cas vivants, ses pierres tombales et l'état de ses permissions —
/// chacun nommé dans [`Chargement`].
///
/// On ne refuse pas sur les permissions : un montage de secret peut avoir sa propre discipline, et
/// refuser de démarrer sur un mode de fichier casserait des déploiements corrects. Mais on le
/// **dit**, fort, au même titre qu'un secret laissé à son défaut.
pub fn charger(chemin: &Path) -> Result<Chargement, Erreur> {
    let brut = std::fs::read_to_string(chemin).map_err(|e| Erreur::Illisible {
        chemin: chemin.display().to_string(),
        cause: e.to_string(),
    })?;
    let permissions_trop_ouvertes = lisible_par_tous(chemin);

    let mut vivantes: Vec<Entree> = Vec::new();
    let mut broyes: Vec<(String, u64)> = Vec::new();
    let mut vus: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (i, ligne) in brut.lines().enumerate() {
        let n = i + 1;
        let l = ligne.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let champs: Vec<&str> = l.split_whitespace().collect();
        if champs.len() != 3 {
            return Err(Erreur::Ligne {
                chemin: chemin.display().to_string(),
                n,
                pourquoi: format!("{} champ(s) au lieu de 3", champs.len()),
            });
        }
        let cas = champs[0].to_string();
        let instant = champs[2].parse::<u64>().map_err(|_| Erreur::Ligne {
            chemin: chemin.display().to_string(),
            n,
            pourquoi: format!("« {} » n'est pas un instant (époque, secondes)", champs[2]),
        })?;
        if !vus.insert(cas.clone()) {
            return Err(Erreur::CasDuplique { chemin: chemin.display().to_string(), cas });
        }
        // Le marqueur se teste AVANT l'hexadécimal : `BROYE` n'en est pas, et le confondre avec une
        // clé invalide ferait refuser le démarrage d'un service dont tout va bien.
        if champs[1] == BROYE {
            broyes.push((cas, instant));
            continue;
        }
        let cle = match hex32(champs[1]) {
            Some(b) => CaseKey::from_bytes(b),
            None => {
                return Err(Erreur::CleInvalide {
                    chemin: chemin.display().to_string(),
                    n,
                    cas,
                })
            }
        };
        vivantes.push(Entree { cas, cle, depuis: instant });
    }
    Ok(Chargement { vivantes, broyes, permissions_trop_ouvertes })
}

/// **Broie durablement** des cas : la clé est remplacée par [`BROYE`] et la date retenue.
///
/// Écrit un fichier temporaire voisin puis `rename` : une écriture en place interrompue perdrait
/// des clés que personne n'a demandé de détruire, ce qui serait un effacement de masse par accident
/// dans le module dont tout le propos est l'effacement **voulu**.
///
/// Les commentaires et l'ordre des lignes sont **préservés** : le fichier reste celui de
/// l'exploitant, avec ses clés détruites et leurs dates inscrites à la place.
pub fn broyer(chemin: &Path, cas: &[String], le: u64) -> Result<usize, Erreur> {
    let brut = std::fs::read_to_string(chemin).map_err(|e| Erreur::Illisible {
        chemin: chemin.display().to_string(),
        cause: e.to_string(),
    })?;
    let a_retirer: std::collections::BTreeSet<&str> = cas.iter().map(String::as_str).collect();

    let mut garde = String::with_capacity(brut.len());
    let mut retires = 0usize;
    for ligne in brut.lines() {
        let l = ligne.trim();
        let champs: Vec<&str> = l.split_whitespace().collect();
        let vise = !l.is_empty()
            && !l.starts_with('#')
            && champs.first().map(|c| a_retirer.contains(*c)).unwrap_or(false)
            && champs.get(1).map(|k| *k != BROYE).unwrap_or(false);
        if vise {
            retires += 1;
            // La clé disparaît ; le CAS et la DATE restent. On ne supprime pas la ligne : ce serait
            // effacer le procès-verbal de l'effacement.
            garde.push_str(&format!("{}\t{BROYE}\t{le}", champs[0]));
            garde.push('\n');
            continue;
        }
        garde.push_str(ligne);
        garde.push('\n');
    }

    let temporaire = chemin_temporaire(chemin);
    let echec = |c: String| Erreur::EcritureImpossible { chemin: chemin.display().to_string(), cause: c };
    {
        let mut f = std::fs::File::create(&temporaire).map_err(|e| echec(e.to_string()))?;
        f.write_all(garde.as_bytes()).map_err(|e| echec(e.to_string()))?;
        // Sans `sync_all`, `rename` peut précéder l'arrivée des octets sur le support : on aurait
        // renommé un fichier vide par-dessus le trousseau. Le coût est une écriture par balayage.
        f.sync_all().map_err(|e| echec(e.to_string()))?;
    }
    std::fs::rename(&temporaire, chemin).map_err(|e| {
        let _ = std::fs::remove_file(&temporaire);
        echec(e.to_string())
    })?;
    Ok(retires)
}

fn chemin_temporaire(chemin: &Path) -> PathBuf {
    let mut nom = chemin.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    nom.push_str(".scella-tmp");
    chemin.with_file_name(nom)
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Le fichier est-il lisible par le groupe ou par tous ? (Unix seulement ; ailleurs, on ne prétend
/// rien plutôt que de rendre un `false` qui rassurerait à tort.)
#[cfg(unix)]
fn lisible_par_tous(chemin: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(chemin).map(|m| m.permissions().mode() & 0o077 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn lisible_par_tous(_chemin: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use provenance_dag::shred::{sceller, Lecture, Trousseau};

    fn temporaire(nom: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("scella-m6-{nom}-{}.keys", std::process::id()));
        p
    }

    fn ecrire(p: &Path, contenu: &str) {
        std::fs::write(p, contenu).expect("écriture");
    }

    const CLE_A: &str = "aa";
    const CLE_B: &str = "bb";

    fn fichier(nom: &str) -> PathBuf {
        let p = temporaire(nom);
        ecrire(
            &p,
            &format!(
                "# un commentaire que la réécriture doit PRÉSERVER\n\
                 SUP-A   {}   100\n\
                 \n\
                 SUP-B   {}   900\n",
                CLE_A.repeat(32),
                CLE_B.repeat(32)
            ),
        );
        p
    }

    #[test]
    fn un_trousseau_se_charge_et_ouvre_ce_quil_doit() {
        let p = fichier("charge");
        let Chargement { vivantes, broyes, .. } = charger(&p).expect("chargé");
        assert_eq!(vivantes.len(), 2, "les commentaires et les lignes vides sont ignorés");
        assert!(broyes.is_empty());
        assert_eq!(vivantes[0].cas, "SUP-A");
        assert_eq!(vivantes[0].depuis, 100);

        let mut t = Trousseau::new();
        for e in vivantes {
            t.poser(&e.cas, e.cle, e.depuis);
        }
        let cle = CaseKey::from_bytes(hex32(&CLE_A.repeat(32)).unwrap());
        let env = sceller(&cle, "SUP-A::x:output", "COMPLIANT");
        assert_eq!(t.lire("SUP-A", "SUP-A::x:output", &env), Lecture::Ouvert("COMPLIANT".into()));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn une_ligne_malformee_est_refusee_et_nommee() {
        let p = temporaire("malforme");
        ecrire(&p, "SUP-A   deux-champs\n");
        assert!(matches!(charger(&p), Err(Erreur::Ligne { n: 1, .. })));
        ecrire(&p, "SUP-A   pas-du-hex   100\n");
        assert!(matches!(charger(&p), Err(Erreur::CleInvalide { n: 1, .. })));
        ecrire(&p, &format!("SUP-A   {}   hier\n", CLE_A.repeat(32)));
        assert!(matches!(charger(&p), Err(Erreur::Ligne { n: 1, .. })));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn un_cas_declare_deux_fois_est_refuse() {
        let p = temporaire("duplique");
        ecrire(&p, &format!("SUP-A {} 1\nSUP-A {} 2\n", CLE_A.repeat(32), CLE_B.repeat(32)));
        let e = charger(&p).expect_err("doit refuser");
        assert!(matches!(e, Erreur::CasDuplique { .. }));
        assert!(e.to_string().contains("en silence"));
        let _ = std::fs::remove_file(&p);
    }

    /// ⭐ **La destruction est DURABLE, et elle garde son procès-verbal.** La clé disparaît du
    /// disque ; le cas et la DATE restent. Supprimer la ligne perdrait la date, et « détruit le
    /// 12 mars » redeviendrait « jamais vu » au redémarrage.
    #[test]
    fn broyer_detruit_la_cle_sur_le_disque_et_garde_la_date() {
        let p = fichier("broie");
        assert_eq!(broyer(&p, &["SUP-A".to_string()], 555).expect("réécrit"), 1);

        let restant = std::fs::read_to_string(&p).expect("relu");
        assert!(!restant.contains(&CLE_A.repeat(32)), "la clé n'est plus sur le disque");
        assert!(restant.contains("SUP-A"), "mais le cas demeure");
        assert!(restant.contains("un commentaire"), "et les commentaires sont PRÉSERVÉS");

        let Chargement { vivantes, broyes, .. } = charger(&p).expect("rechargé");
        assert_eq!(vivantes.len(), 1, "seul SUP-B reste vivant");
        assert_eq!(broyes, vec![("SUP-A".to_string(), 555)], "la pierre tombale porte SA date");
        let _ = std::fs::remove_file(&p);
    }

    /// Rebroyer un cas déjà broyé ne réécrit pas sa date : une destruction a lieu une fois.
    #[test]
    fn rebroyer_ne_reecrit_pas_une_date_de_destruction() {
        let p = fichier("rebroie");
        broyer(&p, &["SUP-A".to_string()], 100).expect("réécrit");
        assert_eq!(broyer(&p, &["SUP-A".to_string()], 999).expect("réécrit"), 0, "rien à détruire");
        assert_eq!(charger(&p).expect("rechargé").broyes, vec![("SUP-A".to_string(), 100)]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn broyer_un_cas_absent_ne_detruit_rien() {
        let p = fichier("absent");
        assert_eq!(broyer(&p, &["JAMAIS-VU".to_string()], 1).expect("réécrit"), 0);
        assert_eq!(charger(&p).expect("rechargé").vivantes.len(), 2, "les deux survivent");
        let _ = std::fs::remove_file(&p);
    }

    /// Un fichier qu'on ne peut pas réécrire ⟹ **erreur**, jamais un succès silencieux. C'est la
    /// différence entre « détruit » et « déclaré détruit ».
    #[test]
    fn une_reecriture_impossible_est_une_erreur_pas_un_silence() {
        let p = temporaire("inexistant/sous/dossier");
        let e = broyer(&p, &["X".to_string()], 1).expect_err("doit échouer");
        assert!(matches!(e, Erreur::Illisible { .. } | Erreur::EcritureImpossible { .. }));
    }

    #[test]
    fn le_fichier_temporaire_est_voisin_et_ne_traine_pas() {
        let p = fichier("temp");
        let t = chemin_temporaire(&p);
        assert_eq!(t.parent(), p.parent(), "voisin : un rename entre systèmes de fichiers échouerait");
        broyer(&p, &["SUP-A".to_string()], 1).expect("réécrit");
        assert!(!t.exists(), "le temporaire ne survit pas au succès");
        let _ = std::fs::remove_file(&p);
    }

    /// `BROYE` n'est pas de l'hexadécimal : le confondre avec une clé invalide ferait refuser le
    /// démarrage d'un service dont tout va bien.
    #[test]
    fn le_marqueur_de_broyage_nest_pas_une_cle_invalide() {
        let p = temporaire("marqueur");
        ecrire(&p, &format!("SUP-A {BROYE} 42\nSUP-B {} 900\n", CLE_B.repeat(32)));
        let Chargement { vivantes, broyes, .. } = charger(&p).expect("chargé sans erreur");
        assert_eq!(vivantes.len(), 1);
        assert_eq!(broyes, vec![("SUP-A".to_string(), 42)]);
        let _ = std::fs::remove_file(&p);
    }
}

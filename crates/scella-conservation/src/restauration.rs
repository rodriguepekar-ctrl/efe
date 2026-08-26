// SPDX-License-Identifier: UNLICENSED
//! **Restaurer** — et surtout, quand ça rate, dire *laquelle* des choses différentes vient de rater.
//!
//! ## Le vrai travail est dans le diagnostic
//!
//! Recharger un fichier est trivial. Ce qui ne l'est pas — et ce qui fait la différence entre une
//! sauvegarde utile et une sauvegarde décorative — c'est de répondre à la question que l'exploitant
//! pose à trois heures du matin : *pourquoi ça ne remonte pas ?* Un « échec de vérification » ne
//! répond pas. Pire, il **accuse** : il laisse entendre qu'on a altéré la preuve, alors que la cause
//! la plus fréquente sera la clé, ou une archive prise à un autre moment.
//!
//! La feuille de route parlait de cinq issues. En les dérivant honnêtement il y en a **neuf**, et
//! aucune n'est du remplissage : chacune correspond à un incident réel qu'un exploitant rencontrera,
//! et à une action différente.
//!
//! | issue | ce qui s'est passé | ce qu'on fait |
//! |---|---|---|
//! | [`Issue::Restauree`] | rien à signaler | on repart |
//! | [`Issue::GenerationDetruite`] | la clé a été détruite, à cette date | on cesse de chercher : c'est **définitif et voulu** |
//! | [`Issue::GenerationInconnue`] | ce trousseau-ci ne l'a pas | on va chercher le coffre qui l'a |
//! | [`Issue::MauvaiseGeneration`] | **prouvé** : une autre génération re-prouve l'archive | on corrige le manifeste |
//! | [`Issue::CleOuPremierRecord`] | la chaîne casse au tout premier maillon | ambigu — voir plus bas |
//! | [`Issue::ChaineRompue`] | elle casse plus loin | **là**, quelque chose a été altéré |
//! | [`Issue::Tronquee`] | la chaîne est valide mais plus courte que promis | il manque la fin |
//! | [`Issue::Etendue`] | plus longue | l'archive a vécu après sa prise |
//! | [`Issue::RacineDivergente`] | même longueur, autre racine | la chaîne a été **re-scellée** |
//!
//! ## L'ambiguïté du premier maillon, énoncée au lieu d'être masquée
//!
//! Une rupture au record 0 a **deux** causes possibles, et le calcul ne permet pas de les
//! distinguer : soit la clé n'est pas la bonne — auquel cas *tous* les sceaux sont faux, et le
//! premier l'est donc aussi — soit le premier record a été altéré. Rendre un verdict « mauvaise
//! clé » serait confortable et parfois faux.
//!
//! Alors ce module fait ce qu'il peut prouver, et dit le reste : si une **autre** génération du
//! trousseau re-prouve l'archive, ce n'est plus une hypothèse, c'est un fait — [`Issue::MauvaiseGeneration`]
//! nomme la vraie. Sinon, [`Issue::CleOuPremierRecord`] énonce les deux causes sans choisir.
//!
//! ## Pourquoi `RacineDivergente` mérite d'exister
//!
//! Le cas paraît impossible : si le contenu changeait, la chaîne ne se re-prouverait plus. Il ne
//! l'est pas — **pour qui détient la clé**. Modifier un record puis re-sceller toute la chaîne
//! produit une archive parfaitement re-prouvable, de même longueur, et de racine différente. C'est
//! la réécriture de l'histoire par le détenteur légitime, et la seule chose qui l'attrape est la
//! racine écrite ailleurs — au manifeste, et mieux encore dans une ancre publiée hors de sa portée.

use std::fmt;
use std::path::Path;

use provenance_dag::{reprove_dag, Record, SealedDag, TenantKey};

use crate::generations::{EtatGeneration, TrousseauMaitre};
use crate::manifeste::{Manifeste, NOM_MANIFESTE, NOM_RECORDS};

/// Ce qu'une tentative de restauration a établi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// La chaîne se re-prouve, la longueur et la racine sont celles du manifeste.
    Restauree {
        /// La racine re-dérivée (égale à celle du manifeste).
        racine: String,
        /// Le nombre de records.
        noeuds: usize,
    },
    /// La génération annoncée a été détruite à cette date. Irrécupérable **par dessein**.
    GenerationDetruite {
        /// Le numéro annoncé par le manifeste.
        generation: u32,
        /// Quand la clé a été détruite.
        le: u64,
    },
    /// Ce trousseau ne connaît pas la génération annoncée. Ce n'est pas une affirmation d'inexistence.
    GenerationInconnue {
        /// Le numéro annoncé.
        generation: u32,
    },
    /// **Prouvé** : le manifeste annonce une génération, une autre re-prouve réellement l'archive.
    MauvaiseGeneration {
        /// Celle qu'annonce le manifeste.
        annoncee: u32,
        /// Celle qui re-prouve.
        reelle: u32,
    },
    /// La chaîne casse au premier maillon : mauvaise clé, ou premier record altéré. Indécidable ici.
    CleOuPremierRecord,
    /// La chaîne casse au-delà du premier maillon — un record a été altéré sans re-scellement.
    ChaineRompue {
        /// L'index du premier record cassé.
        a: usize,
    },
    /// La chaîne est valide, mais plus courte que ce que le manifeste promettait.
    Tronquee {
        /// Ce que le manifeste annonçait.
        attendus: usize,
        /// Ce qui a été trouvé.
        trouves: usize,
    },
    /// La chaîne est valide, mais plus longue : des records ont été ajoutés après la prise.
    Etendue {
        /// Ce que le manifeste annonçait.
        attendus: usize,
        /// Ce qui a été trouvé.
        trouves: usize,
    },
    /// Même longueur, autre racine : la chaîne a été re-scellée par un détenteur de la clé.
    RacineDivergente {
        /// La racine du manifeste.
        attendue: String,
        /// La racine re-dérivée.
        obtenue: String,
    },
    /// Le manifeste est absent ou illisible.
    ManifesteIllisible {
        /// Ce qui n'a pas marché.
        cause: String,
    },
    /// Les records sont absents ou illisibles.
    RecordsIllisibles {
        /// Ce qui n'a pas marché.
        cause: String,
    },
}

impl Issue {
    /// La seule issue qui autorise à repartir.
    #[must_use]
    pub fn est_restauree(&self) -> bool {
        matches!(self, Issue::Restauree { .. })
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Restauree { racine, noeuds } => {
                write!(f, "restaurée : {noeuds} records, racine {racine}")
            }
            Self::GenerationDetruite { generation, le } => write!(
                f,
                "génération {generation} DÉTRUITE le {le} — cette archive ne se re-prouvera plus, et \
                 c'est voulu. Ce n'est pas une altération."
            ),
            Self::GenerationInconnue { generation } => write!(
                f,
                "génération {generation} inconnue de ce trousseau — un autre coffre en détient \
                 peut-être la clé. Ce n'est PAS une affirmation d'inexistence."
            ),
            Self::MauvaiseGeneration { annoncee, reelle } => write!(
                f,
                "le manifeste annonce la génération {annoncee}, mais c'est la {reelle} qui re-prouve \
                 cette archive — c'est le manifeste qu'il faut corriger, pas la chaîne"
            ),
            Self::CleOuPremierRecord => write!(
                f,
                "rupture au premier maillon : la clé n'est pas la bonne, OU le premier record a été \
                 altéré. Le calcul ne permet pas de trancher, et aucune autre génération connue ne \
                 re-prouve cette archive."
            ),
            Self::ChaineRompue { a } => write!(
                f,
                "chaîne rompue au record {a} — au-delà du premier maillon, donc ce n'est pas la clé : \
                 quelque chose a été altéré"
            ),
            Self::Tronquee { attendus, trouves } => write!(
                f,
                "tronquée : {trouves} records pour {attendus} annoncés. La chaîne restante est valide — \
                 un préfixe de chaîne valide l'est toujours — c'est le COMPTE qui la trahit."
            ),
            Self::Etendue { attendus, trouves } => write!(
                f,
                "étendue : {trouves} records pour {attendus} annoncés — l'archive a vécu après sa prise, \
                 son manifeste est périmé"
            ),
            Self::RacineDivergente { attendue, obtenue } => write!(
                f,
                "même longueur, racine différente ({obtenue} au lieu de {attendue}) : la chaîne a été \
                 RE-SCELLÉE. Seul un détenteur de la clé peut faire cela."
            ),
            Self::ManifesteIllisible { cause } => write!(f, "manifeste illisible : {cause}"),
            Self::RecordsIllisibles { cause } => write!(f, "records illisibles : {cause}"),
        }
    }
}

/// Le résultat complet d'une tentative — ce qu'on a pu lire, et ce qu'on en conclut.
#[derive(Debug, Clone)]
pub struct Restauration {
    /// Le manifeste, s'il a pu être lu.
    pub manifeste: Option<Manifeste>,
    /// Les records, s'ils ont pu être lus. Présents même quand l'issue est un échec : un exploitant
    /// qui diagnostique a besoin de les regarder.
    pub records: Vec<Record>,
    /// Ce qui a été établi.
    pub issue: Issue,
}

impl Restauration {
    fn echec(manifeste: Option<Manifeste>, records: Vec<Record>, issue: Issue) -> Self {
        Self { manifeste, records, issue }
    }
}

/// Lit les deux fichiers d'une archive. Séparé du diagnostic pour que l'ordre des refus soit lisible.
// L'échec est mis en boîte : sans cela, chaque appel — y compris les réussites — paierait sur la
// pile la taille du plus gros diagnostic possible. `clippy::result_large_err` a raison de le dire.
fn lire_archive(dossier: &Path) -> Result<(Manifeste, Vec<Record>), Box<Restauration>> {
    let texte = std::fs::read_to_string(dossier.join(NOM_MANIFESTE)).map_err(|e| {
        Box::new(Restauration::echec(None, Vec::new(), Issue::ManifesteIllisible { cause: e.to_string() }))
    })?;
    let manifeste = Manifeste::lire(&texte).map_err(|e| {
        Box::new(Restauration::echec(None, Vec::new(), Issue::ManifesteIllisible { cause: e.to_string() }))
    })?;
    let brut = std::fs::read_to_string(dossier.join(NOM_RECORDS)).map_err(|e| {
        Box::new(Restauration::echec(
            Some(manifeste.clone()),
            Vec::new(),
            Issue::RecordsIllisibles { cause: e.to_string() },
        ))
    })?;
    let records = SealedDag::records_from_jsonl(&brut).map_err(|e| {
        Box::new(Restauration::echec(
            Some(manifeste.clone()),
            Vec::new(),
            Issue::RecordsIllisibles { cause: format!("{e:?}") },
        ))
    })?;
    Ok((manifeste, records))
}

/// Confronte une chaîne lue à ce que son manifeste promettait. Le cœur du diagnostic.
///
/// `autres` sert au seul cas où l'on peut lever l'ambiguïté du premier maillon : si une autre clé
/// connue re-prouve l'archive, la mauvaise génération n'est plus une hypothèse.
fn diagnostiquer(
    manifeste: &Manifeste,
    records: &[Record],
    cle: &TenantKey,
    autres: &[(u32, &TenantKey)],
) -> Issue {
    let v = reprove_dag(records, cle);
    if !v.ok {
        let a = v.broken_at.unwrap_or(0);
        if a == 0 {
            for (numero, autre) in autres {
                if *numero != manifeste.generation && reprove_dag(records, autre).ok {
                    return Issue::MauvaiseGeneration { annoncee: manifeste.generation, reelle: *numero };
                }
            }
            return Issue::CleOuPremierRecord;
        }
        return Issue::ChaineRompue { a };
    }

    // La chaîne est authentique. Reste à savoir si c'est bien CELLE que le manifeste décrivait.
    // Le compte d'abord : il se compare d'un coup d'œil, là où deux racines de 64 caractères se
    // comparent mal. Deux gardes, et le plus lisible passe en premier.
    if records.len() < manifeste.noeuds {
        return Issue::Tronquee { attendus: manifeste.noeuds, trouves: records.len() };
    }
    if records.len() > manifeste.noeuds {
        return Issue::Etendue { attendus: manifeste.noeuds, trouves: records.len() };
    }
    let obtenue = v.manifest_root.unwrap_or_default();
    if obtenue != manifeste.racine {
        return Issue::RacineDivergente { attendue: manifeste.racine.clone(), obtenue };
    }
    Issue::Restauree { racine: obtenue, noeuds: records.len() }
}

/// Restaure avec **une** clé — le cas simple, quand on sait déjà laquelle.
///
/// Ne peut pas prouver une mauvaise génération : sans trousseau, il n'y a rien à quoi comparer.
pub fn restaurer(dossier: &Path, cle: &TenantKey) -> Restauration {
    let (manifeste, records) = match lire_archive(dossier) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let issue = diagnostiquer(&manifeste, &records, cle, &[]);
    Restauration { manifeste: Some(manifeste), records, issue }
}

/// Restaure via le **trousseau des générations** — la voie normale d'un service.
///
/// C'est ici que le bénéfice de la rotation se constate : une archive scellée sous une génération
/// simplement *retirée* se restaure sans que rien de particulier ne soit à faire.
pub fn restaurer_avec(dossier: &Path, trousseau: &TrousseauMaitre) -> Restauration {
    let (manifeste, records) = match lire_archive(dossier) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let generation = manifeste.generation;

    // L'état d'abord : une génération détruite ne se « vérifie » pas — on ne va pas produire un
    // verdict d'altération sur une destruction volontaire et documentée.
    match trousseau.etat(generation) {
        EtatGeneration::Detruite { le } => {
            let issue = Issue::GenerationDetruite { generation, le };
            return Restauration::echec(Some(manifeste), records, issue);
        }
        EtatGeneration::Inconnue => {
            let issue = Issue::GenerationInconnue { generation };
            return Restauration::echec(Some(manifeste), records, issue);
        }
        EtatGeneration::Vivante { .. } | EtatGeneration::Retiree { .. } => {}
    }

    let cle = match trousseau.cle_de_lecture(generation) {
        Some(c) => c,
        // Inatteignable : les deux états restants portent une clé. On refuse plutôt que d'unwrap —
        // un état ajouté plus tard passerait ici, et un panic serait la pire façon de l'apprendre.
        None => {
            let issue = Issue::GenerationInconnue { generation };
            return Restauration::echec(Some(manifeste), records, issue);
        }
    };

    let inventaire = trousseau.inventaire();
    let autres: Vec<(u32, &TenantKey)> = inventaire
        .iter()
        .filter_map(|g| trousseau.cle_de_lecture(g.numero).map(|c| (g.numero, c)))
        .collect();

    let issue = diagnostiquer(&manifeste, &records, cle, &autres);
    Restauration { manifeste: Some(manifeste), records, issue }
}

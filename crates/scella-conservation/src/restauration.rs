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
//! ## Pourquoi `RacineDivergente` mérite d'exister — et ce qu'elle n'attrape PAS
//!
//! Le cas paraît impossible : si le contenu changeait, la chaîne ne se re-prouverait plus. Il ne
//! l'est pas — **pour qui détient la clé**. Modifier un record puis re-sceller toute la chaîne
//! produit une archive parfaitement re-prouvable, de même longueur, et de racine différente.
//!
//! Mais il faut aller au bout du raisonnement, ce que la première version de ce module n'avait pas
//! fait : **celui qui détient l'archive détient aussi son manifeste**. Un adversaire qui re-scelle
//! réécrit la racine du manifeste dans le même geste, et `RacineDivergente` ne se déclenche jamais.
//! Elle attrape la corruption accidentelle et l'adversaire négligent. Pas l'adversaire décidé.
//!
//! ## L'ancre — la seule chose que le porteur de l'archive ne peut pas réécrire
//!
//! Le remède est l'**ancre** : un engagement `(run_id, racine, compte)` publié dans un journal que
//! le porteur de l'archive ne contrôle pas. `provenance-dag` — la seule dépendance de ce crate —
//! sait déjà l'indexer et la vérifier (`AnchorIndex`, `verify_against_anchor`). Le manifeste ne
//! porte donc pas une URL décorative mais le **`run_id`** qui indexe ce journal, et [`Controle`]
//! permet de l'exiger.
//!
//! L'ancre est confrontée **avant** le manifeste et son verdict l'emporte, pour une raison qui tient
//! en une phrase : le manifeste voyage avec l'archive, l'ancre non.

use std::fmt;
use std::path::Path;

use provenance_dag::{reprove_dag, verify_against_anchor, AnchorIndex, Record, SealedDag, TenantKey};

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
    /// **Le verdict qui compte** : l'ancre publiée contredit ce que l'archive contient.
    ///
    /// Contrairement à [`Issue::RacineDivergente`], celle-ci attrape l'adversaire décidé : il a pu
    /// réécrire les records ET le manifeste, il n'a pas pu réécrire un journal qu'il ne détient pas.
    AncreDivergente {
        /// Le `run_id` que le manifeste désigne.
        run_id: String,
        /// Ce que l'ancre publiée engage.
        motif: String,
    },
    /// Le manifeste désigne une ancre que le journal fourni ne contient pas.
    ///
    /// Pas une accusation : le journal peut être partiel, ou l'ancre pas encore publiée.
    AncreAbsente {
        /// Le `run_id` cherché.
        run_id: String,
    },
    /// Le journal d'ancres fourni ne s'analyse pas.
    AncreIllisible {
        /// Ce qui n'a pas marché.
        cause: String,
    },
    /// Le trousseau des clés de cas présenté n'est pas celui qui était en vigueur à la prise.
    ///
    /// La chaîne se re-prouve — le scellement ne dépend pas de ces clés — mais les champs effaçables
    /// se liraient `CleAbsente` sans que rien ne le dise. On le dit.
    TrousseauDivergent {
        /// L'empreinte inscrite au manifeste.
        attendue: String,
        /// Celle du fichier présenté.
        obtenue: String,
    },
    /// Le manifeste porte une empreinte de trousseau, et aucun trousseau n'a été présenté.
    TrousseauNonPresente {
        /// L'empreinte que le manifeste attend.
        attendue: String,
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
            Self::AncreDivergente { run_id, motif } => write!(
                f,
                "L'ANCRE PUBLIÉE CONTREDIT CETTE ARCHIVE (run {run_id}) : {motif}. Le manifeste voyage \
                 avec l'archive, l'ancre non — c'est donc l'ancre qui a raison."
            ),
            Self::AncreAbsente { run_id } => write!(
                f,
                "le manifeste désigne l'ancre « {run_id} », absente du journal fourni — journal partiel \
                 ou ancre non publiée. Ce n'est pas une accusation."
            ),
            Self::AncreIllisible { cause } => write!(f, "journal d'ancres illisible : {cause}"),
            Self::TrousseauDivergent { attendue, obtenue } => write!(
                f,
                "le trousseau présenté ({obtenue}) n'est pas celui de la prise ({attendue}) : la chaîne \
                 se re-prouve, mais les champs effaçables se liront « clé absente » sans autre \
                 explication. Restaurez le bon trousseau."
            ),
            Self::TrousseauNonPresente { attendue } => write!(
                f,
                "le manifeste attend le trousseau d'empreinte {attendue} et aucun n'a été présenté — \
                 les champs effaçables resteront illisibles"
            ),
            Self::ManifesteIllisible { cause } => write!(f, "manifeste illisible : {cause}"),
            Self::RecordsIllisibles { cause } => write!(f, "records illisibles : {cause}"),
        }
    }
}

/// Ce que l'appelant accepte de **confronter** à l'archive, au-delà de la clé.
///
/// Vide par défaut, et c'est volontaire : une restauration hors-ligne, sur une machine de secours
/// qui n'a que l'archive et la clé, doit rester possible. Chaque pièce ajoutée ici resserre le
/// verdict d'un cran — mais ne pas l'avoir n'est pas une faute, c'est une situation.
#[derive(Debug, Default, Clone)]
pub struct Controle<'a> {
    /// Le **texte** du journal d'ancres. Fourni par l'appelant plutôt que lu par ce module : le
    /// journal peut venir d'un fichier, d'un objet distant ou d'un dépôt tiers, et c'est
    /// précisément parce qu'il ne vient pas de l'archive qu'il a de la valeur.
    pub journal_ancres: Option<&'a str>,
    /// Le fichier de trousseau des clés de cas dont on veut confronter l'empreinte.
    pub trousseau_cas: Option<&'a Path>,
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
    controle: &Controle<'_>,
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

    // ── L'ANCRE D'ABORD, quand elle est fournie ──────────────────────────────────────────────
    // Elle passe avant le manifeste, et son verdict l'emporte, pour une raison qui tient en une
    // phrase : le manifeste voyage avec l'archive, l'ancre non. Un adversaire qui re-scelle réécrit
    // le premier dans le même geste ; il ne peut rien contre le second.
    if let Some(journal) = controle.journal_ancres {
        if let Some(run_id) = manifeste.ancre.as_deref() {
            match AnchorIndex::parse(journal) {
                Err(e) => return Issue::AncreIllisible { cause: format!("{e:?}") },
                Ok(index) => match index.get(run_id) {
                    None => return Issue::AncreAbsente { run_id: run_id.to_string() },
                    Some(ancre) => {
                        let v = verify_against_anchor(records, cle, ancre);
                        if !v.ok {
                            return Issue::AncreDivergente {
                                run_id: run_id.to_string(),
                                motif: v.reason.unwrap_or_else(|| "sans motif".into()),
                            };
                        }
                    }
                },
            }
        }
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

    // ── Le trousseau des cas ────────────────────────────────────────────────────────────────
    // Il ne participe PAS au scellement : la chaîne se re-prouve sans lui. Mais restaurer avec le
    // mauvais trousseau donne un registre dont les champs effaçables se lisent tous « clé absente »,
    // à chaque consultation, sans que rien n'en explique la cause. C'est le défaut que M6 et M7
    // avaient produit ensemble, chacun ayant raison de son côté. On le dit ici, une fois.
    if let Some(attendue) = manifeste.empreinte_trousseau.as_deref() {
        match controle.trousseau_cas {
            None => return Issue::TrousseauNonPresente { attendue: attendue.to_string() },
            Some(chemin) => match crate::archive::empreinte_fichier(chemin) {
                Err(e) => {
                    return Issue::TrousseauDivergent {
                        attendue: attendue.to_string(),
                        obtenue: format!("illisible : {e}"),
                    }
                }
                Ok(obtenue_emp) if obtenue_emp != attendue => {
                    return Issue::TrousseauDivergent {
                        attendue: attendue.to_string(),
                        obtenue: obtenue_emp,
                    }
                }
                Ok(_) => {}
            },
        }
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
    let issue = diagnostiquer(&manifeste, &records, cle, &[], &Controle::default());
    Restauration { manifeste: Some(manifeste), records, issue }
}

/// Restaure via le **trousseau des générations** — la voie normale d'un service.
///
/// C'est ici que le bénéfice de la rotation se constate : une archive scellée sous une génération
/// simplement *retirée* se restaure sans que rien de particulier ne soit à faire.
///
/// Ne confronte que l'archive à elle-même. Pour opposer une ancre publiée ou un trousseau de cas,
/// voir [`restaurer_controle`].
pub fn restaurer_avec(dossier: &Path, trousseau: &TrousseauMaitre) -> Restauration {
    restaurer_controle(dossier, trousseau, &Controle::default())
}

/// Restaure **et confronte** à ce que l'appelant apporte de l'extérieur de l'archive.
///
/// La différence avec [`restaurer_avec`] n'est pas de degré. Sans [`Controle`], on vérifie qu'une
/// archive est cohérente **avec elle-même** — ce qui laisse passer quiconque a réécrit les deux
/// fichiers ensemble. Avec une ancre, on vérifie qu'elle est cohérente avec un engagement pris
/// ailleurs, à un moment où l'adversaire n'avait pas encore décidé de mentir.
pub fn restaurer_controle(
    dossier: &Path,
    trousseau: &TrousseauMaitre,
    controle: &Controle<'_>,
) -> Restauration {
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

    let issue = diagnostiquer(&manifeste, &records, cle, &autres, controle);
    Restauration { manifeste: Some(manifeste), records, issue }
}

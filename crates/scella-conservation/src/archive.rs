// SPDX-License-Identifier: UNLICENSED
//! **Sauvegarder** — assembler une archive qu'une restauration pourra affirmer, et non seulement lire.
//!
//! ## Une sauvegarde n'est pas une copie
//!
//! `CaptureStore::save_local` existait déjà : elle écrit les records en JSONL, atomiquement. C'est
//! une **copie**, et une copie ne permet à personne d'affirmer quoi que ce soit. Elle ne dit ni de
//! qui sont ces records, ni ce que leur chaîne doit redonner, ni combien il y en avait, ni quelle
//! clé les re-prouve. Restaurer depuis une copie, c'est charger des octets et espérer.
//!
//! Une archive ajoute le peu de contexte sans lequel la restauration ne peut rien conclure — et
//! **rien d'autre** : la clé n'y est jamais (le langage l'interdit, cf. [`crate::generations`]), et
//! le trousseau des cas non plus, dont seule l'empreinte voyage.
//!
//! ## On re-prouve AVANT d'archiver
//!
//! `forensic-export` refuse d'exporter un store cassé plutôt que de faire confiance à un drapeau.
//! Ce module applique la même règle, et pour une raison plus dure encore : archiver une chaîne
//! rompue **grave la rupture dans la sauvegarde**. Le jour de la restauration, on ne saurait plus si
//! la casse date de l'incident qu'on répare ou d'avant — et la sauvegarde, censée être le recours,
//! serait devenue le doute.
//!
//! La racine écrite au manifeste est donc celle que la **re-dérivation vient de calculer**, jamais
//! une valeur fournie par l'appelant. Elle ne peut pas mentir sur son propre contenu.
//!
//! ## L'écriture est atomique, et dans cet ordre
//!
//! Fichier temporaire voisin, `fsync`, puis `rename` — la discipline de `save_local` et de
//! `trousseau_fichier`. Les records sont écrits **avant** le manifeste : une archive interrompue
//! entre les deux n'a pas de manifeste, donc [`crate::restauration`] la refuse franchement. L'ordre
//! inverse produirait un manifeste qui promet des records absents — une archive qui *ment* plutôt
//! qu'une archive *incomplète*.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use provenance_dag::{reprove_dag, sha256, Record, SealedDag, TenantKey};

use crate::manifeste::{Manifeste, ManifesteError, NOM_MANIFESTE, NOM_RECORDS};

/// Ce que l'appelant apporte et que les records ne portent pas.
#[derive(Debug, Clone)]
pub struct Prise {
    /// Le tenant propriétaire.
    pub tenant: String,
    /// L'instant de la prise (secondes d'époque). Ce crate n'a pas d'horloge : l'appelant la fournit,
    /// ce qui rend aussi les tests déterministes.
    pub cree_a: u64,
    /// La génération de clé qui a scellé ces records.
    pub generation: u32,
    /// L'empreinte du trousseau de cas en vigueur, si un trousseau existait. Voir [`empreinte_fichier`].
    pub empreinte_trousseau: Option<String>,
    /// La référence de l'ancre publiée ailleurs, si elle existe.
    pub ancre: Option<String>,
}

/// Ce qu'une sauvegarde réussie laisse derrière elle.
#[derive(Debug, Clone)]
pub struct Sauvegarde {
    /// Le dossier de l'archive.
    pub dossier: PathBuf,
    /// Le manifeste tel qu'il a été écrit.
    pub manifeste: Manifeste,
}

/// Pourquoi une sauvegarde a été refusée.
#[derive(Debug)]
pub enum ArchiveError {
    /// La chaîne ne se re-prouve pas : on refuse de graver la rupture dans la sauvegarde.
    ChaineRompue {
        /// L'index du premier record cassé, s'il est connu.
        a: Option<usize>,
        /// Le motif rendu par S0.
        motif: String,
    },
    /// La re-preuve a réussi mais n'a pas rendu de racine — S0 a changé de contrat sous nos pieds.
    RacineAbsente,
    /// Le manifeste n'est pas écrivable sans ambiguïté.
    Manifeste(ManifesteError),
    /// Une écriture a échoué.
    Io(std::io::Error),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChaineRompue { a, motif } => match a {
                Some(i) => write!(
                    f,
                    "refus d'archiver : la chaîne est rompue au record {i} ({motif}). Archiver graverait \
                     la rupture dans la sauvegarde."
                ),
                None => write!(f, "refus d'archiver : la chaîne ne se re-prouve pas ({motif})"),
            },
            Self::RacineAbsente => write!(f, "re-preuve réussie sans racine — contrat S0 inattendu"),
            Self::Manifeste(e) => write!(f, "manifeste inécrivable : {e}"),
            Self::Io(e) => write!(f, "écriture impossible : {e}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<ManifesteError> for ArchiveError {
    fn from(e: ManifesteError) -> Self {
        Self::Manifeste(e)
    }
}

impl From<std::io::Error> for ArchiveError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// L'empreinte d'un fichier — en pratique, celle du trousseau des clés de cas.
///
/// **Une empreinte, pas un contenu.** Le trousseau est un secret ; l'archive ne l'est pas. Ce qui
/// voyage suffit à dire « vous restaurez avec un autre trousseau que celui d'origine » et ne révèle
/// rien de ce qu'il contient.
pub fn empreinte_fichier(chemin: &Path) -> std::io::Result<String> {
    let octets = std::fs::read(chemin)?;
    Ok(hex(&sha256(&octets)))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|o| format!("{o:02x}")).collect()
}

/// Écrit un fichier de façon atomique et durable : temporaire voisin, `fsync`, puis `rename`.
fn ecrire_atomique(chemin: &Path, contenu: &str) -> std::io::Result<()> {
    if let Some(parent) = chemin.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut tmp_os = chemin.as_os_str().to_owned();
    tmp_os.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp_os);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contenu.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, chemin)
}

/// Archive une chaîne scellée dans `dossier`.
///
/// Re-prouve d'abord ; refuse si la chaîne est rompue. Écrit les records, puis le manifeste — dans
/// cet ordre, cf. la note du module.
pub fn sauvegarder(
    records: &[Record],
    cle: &TenantKey,
    prise: &Prise,
    dossier: &Path,
) -> Result<Sauvegarde, ArchiveError> {
    let verdict = reprove_dag(records, cle);
    if !verdict.ok {
        return Err(ArchiveError::ChaineRompue {
            a: verdict.broken_at,
            motif: verdict.reason.unwrap_or_else(|| "sans motif".into()),
        });
    }
    let racine = verdict.manifest_root.ok_or(ArchiveError::RacineAbsente)?;

    let manifeste = Manifeste {
        tenant: prise.tenant.clone(),
        racine,
        noeuds: records.len(),
        cree_a: prise.cree_a,
        generation: prise.generation,
        empreinte_trousseau: prise.empreinte_trousseau.clone(),
        ancre: prise.ancre.clone(),
    };
    // Rendu AVANT toute écriture : un manifeste inécrivable ne doit pas laisser d'archive à moitié faite.
    let texte_manifeste = manifeste.rendre()?;

    // Le JSONL passe par `SealedDag::to_jsonl` et non par un sérialiseur local. Ce n'est pas une
    // économie de dépendance (elle en est une, ce crate n'en garde qu'UNE) : c'est une garantie de
    // **fidélité**. Une archive doit produire exactement les octets que `verify-mini` sait relire ;
    // écrire ici un second sérialiseur, c'est créer un second endroit où la divergence peut naître.
    let mut jsonl = SealedDag::from_records(records.to_vec()).to_jsonl();
    if !jsonl.is_empty() {
        jsonl.push('\n');
    }

    ecrire_atomique(&dossier.join(NOM_RECORDS), &jsonl)?;
    ecrire_atomique(&dossier.join(NOM_MANIFESTE), &texte_manifeste)?;

    Ok(Sauvegarde { dossier: dossier.to_path_buf(), manifeste })
}

// SPDX-License-Identifier: UNLICENSED
//! **La publication de l'ancre** — P.4 : écrire la racine attendue **ailleurs** que dans le bundle.
//!
//! Le *format* de l'ancre et sa *vérification* vivent dans la pile B (`provenance_dag::anchor`) :
//! ce sont des primitives critiques pour la confiance, et elles doivent rester lisibles par un
//! vérifieur sans dépendances. Ce module-ci n'apporte que l'**écriture** — l'entrée-sortie, que le
//! cœur de confiance n'a pas à porter.
//!
//! # Le contrat, et ce que le fil ne tranche pas
//!
//! L'architecture (§11.2) laisse volontairement **le support ouvert** : journal append-only
//! interne, horodatage RFC-3161, journal de transparence. Elle n'impose que trois choses, et ce
//! sont elles qui font le travail :
//!
//! 1. l'ancre est écrite **hors** du bundle ;
//! 2. le journal est **append-only** — on n'y réécrit jamais une ligne ;
//! 3. l'auditeur la reçoit par un **canal distinct** de celui qui lui remet le bundle.
//!
//! [`AnchorSink`] est ce contrat, réduit à une méthode. [`FileAnchorSink`] en est la mise en œuvre
//! la plus simple et la plus auditable : un fichier texte que l'on peut lire avec `cat`. Un autre
//! support s'y substitue en implémentant le trait, sans que rien d'autre ne bouge.
//!
//! # Pourquoi l'append-only n'est pas une commodité
//!
//! Un journal réinscriptible ne prouve rien : qui peut réécrire une ancre peut la faire coïncider
//! après coup avec un bundle tronqué. C'est pourquoi [`FileAnchorSink`] ouvre en **ajout** et
//! n'expose **aucun** moyen de modifier l'existant. La détection d'un ré-ancrage contradictoire,
//! elle, appartient au lecteur : `AnchorIndex::parse` rend `Conflict` plutôt que d'arbitrer.
//!
//! # La limite, dite
//!
//! Un producteur malveillant peut tronquer un run *puis* publier l'ancre du bundle tronqué :
//! l'ancre sera cohérente avec le mensonge. Ce que l'ancre empêche, c'est la falsification
//! **postérieure** à sa publication. Elle ne rend pas le producteur honnête ; elle le rend
//! **engagé**.

use std::io::Write;
use std::path::{Path, PathBuf};

use provenance_dag::AnchorRecord;
use uuid::Uuid;

use crate::seal::Bundle;

/// Ce qui peut échouer à la publication.
#[derive(Debug)]
pub enum PublishError {
    /// L'ancre elle-même est inconstructible (champ non sérialisable).
    Anchor(provenance_dag::AnchorError),
    /// Le support a refusé l'écriture.
    Io(std::io::Error),
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PublishError::Anchor(e) => write!(f, "ancre inconstructible : {e}"),
            PublishError::Io(e) => write!(f, "publication de l'ancre refusée : {e}"),
        }
    }
}

impl std::error::Error for PublishError {}

impl From<provenance_dag::AnchorError> for PublishError {
    fn from(e: provenance_dag::AnchorError) -> Self {
        PublishError::Anchor(e)
    }
}
impl From<std::io::Error> for PublishError {
    fn from(e: std::io::Error) -> Self {
        PublishError::Io(e)
    }
}

/// **Le contrat de publication** — une seule méthode, pour que le support reste interchangeable.
///
/// `Send + Sync` : un sink est partagé par un serveur multi-tâches.
pub trait AnchorSink: Send + Sync {
    /// Publie une ancre. **Doit** être append-only : jamais de réécriture.
    fn publish(&self, anchor: &AnchorRecord) -> Result<(), PublishError>;

    /// Un nom lisible du support, pour les journaux d'exploitation. Ne doit contenir aucun secret.
    fn describe(&self) -> String;
}

/// Fabrique l'ancre d'un bundle.
///
/// `derived_at` est **fourni par l'appelant** : le pont ne lit pas d'horloge, et un test peut
/// produire une ancre reproductible à l'octet.
pub fn anchor_of(
    run_id: Uuid,
    tenant: &str,
    bundle: &Bundle,
    derived_at: u64,
) -> Result<AnchorRecord, provenance_dag::AnchorError> {
    AnchorRecord::new(
        run_id.to_string(),
        tenant,
        bundle.manifest_root.clone(),
        bundle.node_count,
        derived_at,
    )
}

/// Le support le plus simple : **un fichier texte append-only**.
///
/// Une ligne par ancre, lisible avec `cat`, relisible par `AnchorIndex::parse`. Le fichier est
/// ouvert en **ajout** à chaque écriture puis refermé : aucun descripteur n'est conservé, ce qui
/// laisse un opérateur faire tourner le journal (rotation) sans coordonner avec le serveur.
#[derive(Debug, Clone)]
pub struct FileAnchorSink {
    path: PathBuf,
}

impl FileAnchorSink {
    /// Ouvre (ou prépare) un journal à ce chemin. Ne crée pas le fichier tant que rien n'est publié.
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self { path: path.as_ref().to_path_buf() }
    }

    /// Le chemin du journal.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AnchorSink for FileAnchorSink {
    fn publish(&self, anchor: &AnchorRecord) -> Result<(), PublishError> {
        // `append(true)` : le noyau positionne l'écriture en fin de fichier à CHAQUE write, ce qui
        // rend l'ajout sûr même si plusieurs processus écrivent. Aucun mode d'écrasement n'est
        // proposé — ni ici, ni ailleurs dans ce type.
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        // Une ligne, terminée : un journal tronqué en cours d'écriture perd la ligne entière
        // plutôt que d'en produire une moitié qui se relirait mal.
        writeln!(f, "{}", anchor.to_line())?;
        f.flush()?;
        Ok(())
    }

    fn describe(&self) -> String {
        format!("fichier append-only « {} »", self.path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use provenance_dag::{AnchorIndex, AnchorError};

    fn bundle(root: &str, n: u32) -> Bundle {
        Bundle { jsonl: String::new(), manifest_root: root.into(), node_count: n }
    }

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("efe-filp-p4-{name}-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn an_anchor_carries_exactly_what_the_bundle_commits() {
        let run = Uuid::from_u128(0xABCD);
        let a = anchor_of(run, "acme", &bundle(&"aa".repeat(32), 7), 1_700).unwrap();
        assert_eq!(a.run_id, run.to_string());
        assert_eq!(a.tenant_id, "acme");
        assert_eq!(a.manifest_root, "aa".repeat(32));
        assert_eq!(a.node_count, 7, "le second garde contre la troncature voyage avec");
        assert_eq!(a.derived_at, 1_700, "horloge fournie par l'appelant — le pont reste pur");
    }

    #[test]
    fn publishing_appends_and_never_rewrites() {
        let p = tmp("append");
        let sink = FileAnchorSink::new(&p);
        let r1 = Uuid::from_u128(1);
        let r2 = Uuid::from_u128(2);
        sink.publish(&anchor_of(r1, "t", &bundle(&"aa".repeat(32), 3), 1).unwrap()).unwrap();
        sink.publish(&anchor_of(r2, "t", &bundle(&"bb".repeat(32), 5), 2).unwrap()).unwrap();

        let journal = std::fs::read_to_string(&p).unwrap();
        assert_eq!(journal.lines().count(), 2, "deux ancres, deux lignes — rien n'est écrasé");

        // …et le journal se relit par la pile B, sans le pont.
        let idx = AnchorIndex::parse(&journal).unwrap();
        assert_eq!(idx.len(), 2);
        assert_eq!(idx.get(&r1.to_string()).unwrap().node_count, 3);
        assert_eq!(idx.get(&r2.to_string()).unwrap().manifest_root, "bb".repeat(32));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn republishing_the_same_run_identically_stays_readable() {
        // Un opérateur prudent republie ; le journal doit rester exploitable.
        let p = tmp("idem");
        let sink = FileAnchorSink::new(&p);
        let a = anchor_of(Uuid::from_u128(9), "t", &bundle(&"cc".repeat(32), 4), 7).unwrap();
        sink.publish(&a).unwrap();
        sink.publish(&a).unwrap();
        let journal = std::fs::read_to_string(&p).unwrap();
        assert_eq!(journal.lines().count(), 2, "append-only : la ligne est bien ajoutée deux fois");
        assert_eq!(AnchorIndex::parse(&journal).unwrap().len(), 1, "mais l'index en voit un seul run");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_contradictory_republication_is_surfaced_by_the_reader() {
        // Le sink ÉCRIT sans juger (append-only) ; c'est le LECTEUR qui refuse d'arbitrer.
        let p = tmp("conflit");
        let sink = FileAnchorSink::new(&p);
        let run = Uuid::from_u128(11);
        sink.publish(&anchor_of(run, "t", &bundle(&"aa".repeat(32), 3), 1).unwrap()).unwrap();
        sink.publish(&anchor_of(run, "t", &bundle(&"bb".repeat(32), 3), 2).unwrap()).unwrap();
        let journal = std::fs::read_to_string(&p).unwrap();
        assert!(
            matches!(AnchorIndex::parse(&journal), Err(AnchorError::Conflict { .. })),
            "un ré-ancrage contradictoire est une PREUVE, pas un détail à arbitrer"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn the_sink_describes_itself_without_leaking_anything() {
        let d = FileAnchorSink::new("/var/log/ancres.log").describe();
        assert!(d.contains("append-only"));
        assert!(d.contains("ancres.log"));
    }
}

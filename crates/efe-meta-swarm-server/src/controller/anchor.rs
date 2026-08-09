// SPDX-License-Identifier: UNLICENSED
//! # `controller::anchor` — Couche 3 · 3a : publier la racine, pour qu'une troncature se voie **sans la clé**.
//!
//! ## Le trou que ce module ferme
//!
//! Le reçu de consensus est *tamper-evident pour un détenteur de clé* : re-prouver la chaîne exige
//! la clé du tenant. Cela suffit à détecter une falsification **de contenu** — mais pas une
//! **troncature**, et c'est là que se cache le cas vicieux : le préfixe d'une chaîne HMAC valide
//! est lui-même une chaîne HMAC parfaitement valide. Couper les cinq derniers pas d'une ronde
//! produit un bundle qui se re-prouve *impeccablement*, sous une racine différente que personne
//! n'a de raison de trouver suspecte.
//!
//! Le manifeste porte bien `n_steps`, mais il voyage AVEC le bundle : celui qui tronque le corrige.
//! Il faut donc une revendication publiée **avant** et **ailleurs** — c'est l'ancre.
//!
//! ## Ce que l'ancre apporte, et ce qu'elle n'apporte pas
//!
//! Elle engage deux valeurs, `root` et `n_steps`, dans un journal **append-only**. Les recouper
//! avec un bundle ne demande **aucun secret** : comparer deux chaînes et deux entiers. C'est la
//! marche que l'ancre fait gagner — un tiers qui n'a pas la clé, et ne peut donc pas re-prouver la
//! chaîne, peut malgré tout constater qu'on lui présente une ronde tronquée ou une racine autre
//! que celle publiée.
//!
//! Elle ne rend pas le reçu *trustless* pour autant. Le détenteur du journal est le même que celui
//! de la clé : il peut publier ce qu'il veut, au moment où il veut. Ce que l'ancre supprime, c'est
//! la réécriture **rétroactive** — publier tôt et ne plus pouvoir revenir dessus. Le passage à une
//! garantie tierce (horodatage RFC 3161, témoin externe) est le jalon suivant, et il n'est pas fait.
//!
//! ## Un seul journal pour les deux couches
//!
//! Le format et le fichier sont ceux du Fil P (`EFE_SWARM_ANCHOR_LOG`) : un auditeur n'a qu'un
//! endroit à consulter pour les runs de la passerelle **et** les rondes de consensus. La
//! séparation des deux natures est assurée par le préfixe `meta:` sur la clé
//! ([`provenance_dag::META_ANCHOR_PREFIX`]) — `AnchorIndex` indexant par `run_id` seul, sans lui
//! une collision d'identifiants mêlerait deux artefacts sans que rien ne le signale.

use std::io::Write;
use std::path::{Path, PathBuf};

use provenance_dag::AnchorRecord;

/// La variable qui désigne le journal. **Délibérément la même que celle du Fil P** : c'est ce qui
/// fait qu'il n'y a qu'un journal, et non deux à corréler.
pub const ANCHOR_LOG_ENV: &str = "EFE_SWARM_ANCHOR_LOG";

/// Un journal d'ancres en **ajout seul**.
///
/// Volontairement minuscule et local plutôt qu'emprunté à `provenance-bridge` : ce dernier possède
/// bien un `FileAnchorSink`, mais en dépendre tirerait blake3, ed25519, chrono et les deux piles de
/// provenance dans le contrôleur. Ce qui doit être partagé — **le format de ligne** — l'est
/// réellement : il n'existe qu'une définition, `AnchorRecord::to_line`, dans `provenance-dag`. Ce
/// qui est recopié ici, ce sont quinze lignes d'ouverture de fichier.
#[derive(Debug, Clone)]
pub struct FileAnchorLog {
    path: PathBuf,
}

impl FileAnchorLog {
    /// Nouveau journal sur `path`.
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self { path: path.as_ref().to_path_buf() }
    }

    /// Le chemin, pour les journaux d'exploitation.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Ajoute une ancre.
    ///
    /// `append(true)` : le noyau repositionne l'écriture en fin de fichier à **chaque** write, ce
    /// qui rend l'ajout sûr même si plusieurs processus écrivent — le contrôleur et la passerelle
    /// partagent précisément ce fichier. Aucun mode d'écrasement n'est ouvert : un journal qu'on
    /// pourrait réécrire ne servirait à rien, puisque c'est l'impossibilité de revenir en arrière
    /// qui fait toute sa valeur.
    ///
    /// # Errors
    /// Toute erreur d'E/S. L'appelant décide quoi en faire — et il en fait un échec, cf.
    /// `run_iterate` : une ancre silencieusement non publiée laisserait un auditeur en chercher
    /// une qui n'existe pas.
    pub fn publish(&self, anchor: &AnchorRecord) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        f.write_all(anchor.to_line().as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()
    }
}

/// Résout le journal depuis l'environnement. `None` ⟹ ancrage inactif, ce qui est un état
/// **honnête** et doit être dit au boot, pas deviné.
#[must_use]
pub fn log_from_env() -> Option<FileAnchorLog> {
    std::env::var(ANCHOR_LOG_ENV).ok().filter(|p| !p.trim().is_empty()).map(FileAnchorLog::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(run: &str, root: &str, n: u32) -> AnchorRecord {
        AnchorRecord::new(run.to_string(), "t1", root.to_string(), n, 1_700_000_000_000).unwrap()
    }

    /// Le journal **ajoute**, il ne réécrit jamais : c'est l'impossibilité du retour en arrière qui
    /// fait toute la valeur d'une ancre. Un `publish` qui tronquerait le fichier rendrait le
    /// dispositif décoratif.
    #[test]
    fn publishing_appends_and_never_rewrites() {
        let dir = std::env::temp_dir().join(format!("efe-anchor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("anchors.log");
        let _ = std::fs::remove_file(&path);
        let log = FileAnchorLog::new(&path);

        log.publish(&rec("meta:run-1", "aa", 3)).unwrap();
        log.publish(&rec("meta:run-2", "bb", 5)).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 2, "la seconde ancre s'AJOUTE : {content}");
        assert!(content.contains("meta:run-1"), "la première survit : {content}");

        // Et le journal se relit avec le lecteur du Fil P — c'est le même format, pas un cousin.
        let idx = provenance_dag::AnchorIndex::parse(&content).expect("journal relisible");
        assert!(idx.get("meta:run-1").is_some(), "l'ancre est retrouvée sous sa clé préfixée");
        std::fs::remove_file(&path).ok();
    }

    /// Le préfixe `meta:` sépare les deux natures d'artefact dans un journal partagé. Sans lui,
    /// `AnchorIndex` — qui indexe par `run_id` SEUL — mêlerait une ronde de consensus et un run de
    /// passerelle portant le même identifiant.
    #[test]
    fn the_meta_prefix_keeps_both_layers_apart_in_one_journal() {
        let a = provenance_dag::meta_anchor_of("r1", "t1", "aa", 3, 0).unwrap();
        let b = rec("r1", "bb", 7); // un run de passerelle, MÊME identifiant nu
        assert_ne!(a.run_id, b.run_id, "le préfixe doit les distinguer");

        let journal = format!("{}\n{}\n", a.to_line(), b.to_line());
        let idx = provenance_dag::AnchorIndex::parse(&journal)
            .expect("deux natures cohabitent SANS conflit fantôme");
        assert_eq!(idx.get("meta:r1").unwrap().manifest_root, "aa");
        assert_eq!(idx.get("r1").unwrap().manifest_root, "bb");
    }
}

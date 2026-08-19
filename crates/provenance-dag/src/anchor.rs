// SPDX-License-Identifier: UNLICENSED
//! **L'ancre publiée** — Fil P / P.4 : la racine de référence, tenue **hors** du bundle.
//!
//! # Pourquoi un bundle ne peut pas se garder lui-même
//!
//! Un DAG scellé prouve que ce qu'il contient n'a pas bougé. Il ne prouve **pas** que tout y est
//! encore : un **préfixe** d'une chaîne de sceaux valide est lui-même une chaîne valide. Retirer
//! les derniers records d'un bundle le laisse donc parfaitement re-prouvable — seule la
//! **manifest-root** change. Et comme la racine voyage *dans* le bundle qu'on tend à l'auditeur,
//! il n'a rien à quoi la confronter : le bundle ne prouve sa cohérence que **contre lui-même**.
//!
//! L'ancre est la sortie de ce cercle. C'est un enregistrement **minuscule**, publié **ailleurs**,
//! qui dit : *pour ce run, la racine attendue est celle-ci, et elle engage N nœuds*. L'auditeur la
//! reçoit par un **canal distinct** de celui du bundle, et c'est cette distinction — pas la
//! cryptographie — qui fait tout le travail.
//!
//! # Le compte de nœuds n'est pas décoratif
//!
//! Il borne l'attaque par troncature une **seconde fois**, indépendamment de la racine. Un bundle
//! amputé trahit alors deux fois : par sa racine *et* par son compte. Deux gardes valent mieux
//! qu'un lorsque l'un d'eux repose sur une comparaison de chaînes que l'on pourrait oublier.
//!
//! # Le format de ligne, et pourquoi il est écrit à la main
//!
//! Une ancre est **une ligne**, à champs séparés par des tabulations. Ce choix suit la philosophie
//! de cette crate : ce qui est critique pour la confiance s'écrit à la main, se relit sans
//! bibliothèque, et n'admet **qu'une seule** écriture par valeur. Un auditeur doit pouvoir lire un
//! journal d'ancres avec `cat`, et un vérifieur sans dépendances doit pouvoir le parser en vingt
//! lignes. Tout champ contenant une tabulation ou un saut de ligne est **refusé à l'écriture** :
//! le format reste ainsi injectif.
//!
//! # Ce que l'ancre ne fait pas
//!
//! Elle ne dit pas que la décision était **correcte**, ni que le producteur était **honnête au
//! moment d'ancrer**. Un producteur malveillant peut tronquer un run *puis* en publier l'ancre :
//! l'ancre sera cohérente avec le mensonge. Ce que l'ancre empêche, c'est la falsification
//! **postérieure** à la publication — et c'est déjà ce qui distingue un journal d'un récit.

use std::collections::HashMap;

use crate::dag::{verify_against_root, Record, Verdict};
use crate::key::TenantKey;

/// Version du format d'ancre. Committée dans la ligne : un lecteur sait ce qu'il lit.
pub const ANCHOR_SCHEMA_V1: u16 = 1;

/// Le séparateur de champs. La tabulation est retenue parce qu'elle n'apparaît ni dans un UUID,
/// ni dans un hexadécimal, ni dans un nombre — et qu'elle se voit dans un éditeur.
const SEP: char = '\t';

/// **L'ancre d'un run** : la racine attendue, publiée hors du bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorRecord {
    /// Version du format.
    pub schema: u16,
    /// Le run ancré. Chaîne et non `Uuid` : cette crate n'a **aucune** dépendance à laquelle
    /// confier un type, et le vérifieur doit rester relisible sans bibliothèque.
    pub run_id: String,
    /// Le tenant propriétaire.
    pub tenant_id: String,
    /// **Le seul champ qui compte vraiment** : la racine attendue (hex).
    pub manifest_root: String,
    /// Le nombre de nœuds engagés — le second garde contre la troncature.
    pub node_count: u32,
    /// Millisecondes depuis l'époque, **informatif**. Ne participe à aucune décision de
    /// vérification : une ancre n'est pas plus vraie parce qu'elle est récente.
    pub derived_at: u64,
}

/// Le préfixe qui **isole** les ancres de consensus méta-essaim dans un journal partagé.
///
/// `AnchorIndex` indexe par `run_id` SEUL. Sans préfixe, une ronde de méta-essaim et un run de
/// passerelle qui porteraient le même identifiant se contrediraient dans l'index — au mieux une
/// `Conflict` fantôme, au pire une ancre de la mauvaise nature rendue pour une recherche. Les
/// collisions sont improbables en pratique (UUID d'un côté, `iter-<hex>` de l'autre) ; « improbable »
/// n'est pas une garantie, et un index de provenance n'est pas l'endroit où s'en contenter.
///
/// Le préfixe rend donc la séparation **structurelle**, et il permet ce qu'on voulait : **UN SEUL
/// journal** couvrant la Couche 2 (runs de la passerelle) et la Couche 3 (rondes de consensus).
/// Un auditeur n'a qu'un fichier à consulter, et sait à quoi il a affaire en lisant la clé.
pub const META_ANCHOR_PREFIX: &str = "meta:";

/// Fabrique l'ancre d'une **ronde de consensus méta-essaim**.
///
/// Prend des valeurs nues plutôt qu'un type de la Couche 3 : ce crate n'a aucune raison de dépendre
/// du moteur de méta-essaim pour construire six champs de texte.
///
/// **Placé ici et non dans `provenance-bridge`** — où vivait d'abord son jumeau `anchor_of` — parce
/// que le pont tire tout un arbre (blake3, ed25519, chrono, les deux piles de provenance). Ancrer
/// une ronde de consensus n'a pas à coûter cela : le contrôleur méta-essaim ne dépend que de ce
/// crate-ci, quatorze crates dont l'essentiel est serde. Le vocabulaire d'ancrage reste avec le
/// TYPE qu'il fabrique, ce qui est de toute façon sa place.
///
/// Ce que l'ancre engage : la **racine** de la chaîne de consensus et le **nombre de pas**. Le
/// second n'est pas décoratif — c'est lui qui attrape la TRONCATURE, dont le préfixe restant est
/// une chaîne parfaitement intègre. Et le tout est vérifiable **SANS LA CLÉ** : comparer deux
/// chaînes et deux entiers ne demande aucun secret, là où re-prouver la chaîne l'exige. C'est
/// précisément la marche que l'ancre fait gagner.
///
/// # Errors
/// Les champs ne doivent contenir ni tabulation ni saut de ligne (le format cesserait d'être
/// injectif) — [`AnchorRecord::new`] le refuse.
pub fn meta_anchor_of(
    meta_id: &str,
    tenant: &str,
    root: &str,
    n_steps: usize,
    derived_at: u64,
) -> Result<AnchorRecord, AnchorError> {
    AnchorRecord::new(
        format!("{META_ANCHOR_PREFIX}{meta_id}"),
        tenant,
        root.to_string(),
        u32::try_from(n_steps).unwrap_or(u32::MAX),
        derived_at,
    )
}

/// Ce qui peut mal se passer avec une ancre ou un journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorError {
    /// Un champ contient une tabulation ou un saut de ligne — le format cesserait d'être injectif.
    FieldNotSerialisable {
        /// Le champ fautif.
        field: &'static str,
    },
    /// La ligne n'a pas le bon nombre de champs.
    MalformedLine {
        /// Nombre de champs trouvés.
        got: usize,
    },
    /// Un champ numérique n'est pas un nombre.
    MalformedField {
        /// Le champ fautif.
        field: &'static str,
    },
    /// Version de format inconnue de ce lecteur.
    SchemaUnsupported(u16),
    /// **Deux ancres du même run se contredisent.**
    ///
    /// Dans un journal append-only, ce n'est pas une erreur de parsing : c'est une **preuve** que
    /// quelque chose ne va pas — soit le producteur a ré-ancré après coup, soit le journal a été
    /// manipulé. On refuse de choisir un gagnant : « la dernière gagne » transformerait une
    /// contradiction visible en un silence.
    Conflict {
        /// Le run concerné.
        run_id: String,
        /// La racine déjà publiée.
        first_root: String,
        /// La racine contradictoire.
        second_root: String,
    },
}

impl std::fmt::Display for AnchorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnchorError::FieldNotSerialisable { field } => {
                write!(f, "champ « {field} » contient une tabulation ou un saut de ligne")
            }
            AnchorError::MalformedLine { got } => {
                write!(f, "ligne d'ancre malformée : {got} champs (6 attendus)")
            }
            AnchorError::MalformedField { field } => write!(f, "champ « {field} » illisible"),
            AnchorError::SchemaUnsupported(v) => write!(f, "schéma d'ancre non supporté : {v}"),
            AnchorError::Conflict { run_id, first_root, second_root } => write!(
                f,
                "ANCRES CONTRADICTOIRES pour le run {run_id} : « {first_root} » puis « {second_root} » — \
                 ré-ancrage après coup, ou journal manipulé"
            ),
        }
    }
}

impl std::error::Error for AnchorError {}

fn clean(v: &str, field: &'static str) -> Result<(), AnchorError> {
    if v.contains(SEP) || v.contains('\n') || v.contains('\r') {
        return Err(AnchorError::FieldNotSerialisable { field });
    }
    Ok(())
}

impl AnchorRecord {
    /// Construit une ancre, en refusant tout champ qui casserait l'injectivité du format.
    pub fn new(
        run_id: impl Into<String>,
        tenant_id: impl Into<String>,
        manifest_root: impl Into<String>,
        node_count: u32,
        derived_at: u64,
    ) -> Result<Self, AnchorError> {
        let (run_id, tenant_id, manifest_root) =
            (run_id.into(), tenant_id.into(), manifest_root.into());
        clean(&run_id, "run_id")?;
        clean(&tenant_id, "tenant_id")?;
        clean(&manifest_root, "manifest_root")?;
        Ok(Self {
            schema: ANCHOR_SCHEMA_V1,
            run_id,
            tenant_id,
            manifest_root,
            node_count,
            derived_at,
        })
    }

    /// La ligne canonique — **sans** saut de ligne final (l'appelant l'ajoute en écrivant).
    #[must_use]
    pub fn to_line(&self) -> String {
        format!(
            "{}{SEP}{}{SEP}{}{SEP}{}{SEP}{}{SEP}{}",
            self.schema,
            self.run_id,
            self.tenant_id,
            self.manifest_root,
            self.node_count,
            self.derived_at
        )
    }

    /// Relit une ligne canonique. Strict : six champs, pas un de plus.
    pub fn from_line(line: &str) -> Result<Self, AnchorError> {
        let f: Vec<&str> = line.split(SEP).collect();
        if f.len() != 6 {
            return Err(AnchorError::MalformedLine { got: f.len() });
        }
        let schema: u16 =
            f[0].parse().map_err(|_| AnchorError::MalformedField { field: "schema" })?;
        if schema != ANCHOR_SCHEMA_V1 {
            return Err(AnchorError::SchemaUnsupported(schema));
        }
        Ok(Self {
            schema,
            run_id: f[1].to_string(),
            tenant_id: f[2].to_string(),
            manifest_root: f[3].to_string(),
            node_count: f[4]
                .parse()
                .map_err(|_| AnchorError::MalformedField { field: "node_count" })?,
            derived_at: f[5]
                .parse()
                .map_err(|_| AnchorError::MalformedField { field: "derived_at" })?,
        })
    }
}

/// Un **journal d'ancres** relu, indexé par run.
///
/// Les lignes vides et celles commençant par `#` sont ignorées : un journal doit pouvoir porter des
/// commentaires d'exploitation sans cesser d'être lisible par une machine.
#[derive(Debug, Clone, Default)]
pub struct AnchorIndex {
    by_run: HashMap<String, AnchorRecord>,
}

impl AnchorIndex {
    /// Relit un journal entier.
    ///
    /// Une **contradiction** (même run, racine ou compte différents) arrête la lecture avec
    /// [`AnchorError::Conflict`] : dans un journal append-only, deux ancres qui se contredisent
    /// sont une information, pas un détail à arbitrer. Une **répétition à l'identique** est en
    /// revanche acceptée : republier la même ancre est idempotent, et un opérateur prudent le fera.
    pub fn parse(journal: &str) -> Result<Self, AnchorError> {
        let mut by_run: HashMap<String, AnchorRecord> = HashMap::new();
        for line in journal.lines() {
            let l = line.trim_end_matches('\r');
            if l.trim().is_empty() || l.starts_with('#') {
                continue;
            }
            let a = AnchorRecord::from_line(l)?;
            match by_run.get(&a.run_id) {
                Some(prev) if prev.manifest_root != a.manifest_root || prev.node_count != a.node_count => {
                    return Err(AnchorError::Conflict {
                        run_id: a.run_id.clone(),
                        first_root: prev.manifest_root.clone(),
                        second_root: a.manifest_root.clone(),
                    });
                }
                _ => {
                    by_run.insert(a.run_id.clone(), a);
                }
            }
        }
        Ok(Self { by_run })
    }

    /// L'ancre d'un run, si elle a été publiée.
    #[must_use]
    pub fn get(&self, run_id: &str) -> Option<&AnchorRecord> {
        self.by_run.get(run_id)
    }

    /// Le nombre de runs ancrés.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_run.len()
    }

    /// Le journal est-il vide ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_run.is_empty()
    }
}

/// **La vérification que P.4 achète** : re-prouver un bundle *contre son ancre publiée*.
///
/// Contrôle la racine **et** le compte de nœuds. Le second n'est pas redondant : il attrape une
/// troncature même si l'on venait à relâcher la comparaison de racine, et il rend le refus plus
/// lisible pour un humain (« il manque des nœuds » plutôt que « la racine diffère »).
#[must_use]
pub fn verify_against_anchor(
    records: &[Record],
    key: &TenantKey,
    anchor: &AnchorRecord,
) -> Verdict {
    let n = records.len();
    if n as u64 != u64::from(anchor.node_count) {
        return Verdict {
            ok: false,
            n_records: n,
            manifest_root: None,
            broken_at: None,
            reason: Some(format!(
                "compte de nœuds NON concordant avec l'ancre publiée — attendu {}, obtenu {n} \
                 (bundle tronqué ou complété)",
                anchor.node_count
            )),
        };
    }
    verify_against_root(records, key, &anchor.manifest_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;
    use crate::dag::SealedDag;

    fn key() -> TenantKey {
        TenantKey::derive(b"racine", "acme")
    }

    fn dag(n: usize) -> SealedDag {
        let k = key();
        let mut d = SealedDag::new();
        for i in 0..n {
            d.append(
                &k,
                Node::AgentAct {
                    node_id: format!("run::{i:03}"),
                    agent_role: "generator".into(),
                    inputs_hash: "h".into(),
                    model: "mode_a".into(),
                    version: "v1".into(),
                    output: "o".into(),
                    evidence_refs: vec![],
                    timestamp: i as u64,
                },
            )
            .unwrap();
        }
        d
    }

    fn anchor_of(d: &SealedDag, n: u32) -> AnchorRecord {
        AnchorRecord::new("run-1", "acme", d.manifest_root(), n, 1_700_000_000_000).unwrap()
    }

    // ── Le format ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_line_roundtrips_exactly() {
        let a = AnchorRecord::new("r1", "t1", "ab".repeat(32), 7, 42).unwrap();
        assert_eq!(AnchorRecord::from_line(&a.to_line()).unwrap(), a);
    }

    #[test]
    fn the_format_stays_injective() {
        // Un champ porteur d'un séparateur casserait la relecture : on refuse à l'ÉCRITURE.
        assert_eq!(
            AnchorRecord::new("r\t1", "t", "root", 1, 0).unwrap_err(),
            AnchorError::FieldNotSerialisable { field: "run_id" }
        );
        assert_eq!(
            AnchorRecord::new("r", "t\nx", "root", 1, 0).unwrap_err(),
            AnchorError::FieldNotSerialisable { field: "tenant_id" }
        );
    }

    #[test]
    fn malformed_lines_are_refused_not_guessed() {
        assert!(matches!(
            AnchorRecord::from_line("1\tr\tt").unwrap_err(),
            AnchorError::MalformedLine { got: 3 }
        ));
        assert!(matches!(
            AnchorRecord::from_line("1\tr\tt\troot\tNaN\t0").unwrap_err(),
            AnchorError::MalformedField { field: "node_count" }
        ));
        assert!(matches!(
            AnchorRecord::from_line("99\tr\tt\troot\t1\t0").unwrap_err(),
            AnchorError::SchemaUnsupported(99)
        ));
    }

    // ── Le journal ────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_journal_indexes_runs_and_tolerates_comments() {
        let a = AnchorRecord::new("r1", "t", "aa".repeat(32), 3, 1).unwrap();
        let b = AnchorRecord::new("r2", "t", "bb".repeat(32), 5, 2).unwrap();
        let j = format!("# journal d'ancres\n{}\n\n{}\n", a.to_line(), b.to_line());
        let idx = AnchorIndex::parse(&j).unwrap();
        assert_eq!(idx.len(), 2);
        assert_eq!(idx.get("r1").unwrap().node_count, 3);
        assert!(idx.get("inconnu").is_none());
    }

    #[test]
    fn republishing_the_same_anchor_is_idempotent() {
        let a = AnchorRecord::new("r1", "t", "aa".repeat(32), 3, 1).unwrap();
        let j = format!("{}\n{}\n", a.to_line(), a.to_line());
        assert_eq!(AnchorIndex::parse(&j).unwrap().len(), 1, "republier à l'identique est sûr");
    }

    #[test]
    fn contradictory_anchors_are_evidence_not_a_detail_to_arbitrate() {
        // « La dernière gagne » transformerait une contradiction VISIBLE en un silence.
        let a = AnchorRecord::new("r1", "t", "aa".repeat(32), 3, 1).unwrap();
        let b = AnchorRecord::new("r1", "t", "bb".repeat(32), 3, 2).unwrap();
        let j = format!("{}\n{}\n", a.to_line(), b.to_line());
        match AnchorIndex::parse(&j).unwrap_err() {
            AnchorError::Conflict { run_id, .. } => assert_eq!(run_id, "r1"),
            other => panic!("attendu Conflict, obtenu {other:?}"),
        }
    }

    #[test]
    fn a_changed_node_count_alone_is_already_a_conflict() {
        let a = AnchorRecord::new("r1", "t", "aa".repeat(32), 3, 1).unwrap();
        let b = AnchorRecord::new("r1", "t", "aa".repeat(32), 4, 2).unwrap();
        assert!(AnchorIndex::parse(&format!("{}\n{}\n", a.to_line(), b.to_line())).is_err());
    }

    // ── La vérification contre l'ancre ────────────────────────────────────────────────────

    #[test]
    fn a_bundle_verifies_against_its_own_anchor() {
        let d = dag(4);
        let a = anchor_of(&d, 4);
        assert!(verify_against_anchor(d.records(), &key(), &a).ok);
    }

    #[test]
    fn truncation_fails_on_both_the_count_and_the_root() {
        // LE test de P.4 : le bundle tronqué reste INTÈGRE, et l'ancre l'attrape quand même —
        // deux fois.
        let d = dag(4);
        let a = anchor_of(&d, 4);
        let short = &d.records()[..3];

        // Intègre en lui-même : c'est tout le piège.
        assert!(crate::dag::reprove_dag(short, &key()).ok, "un préfixe de chaîne valide l'est");

        // Garde n°1 : le compte.
        let v = verify_against_anchor(short, &key(), &a);
        assert!(!v.ok);
        assert!(v.reason.unwrap().contains("compte de nœuds"), "le compte trahit en premier");

        // Garde n°2 : la racine, indépendamment du compte.
        let a_same_count = AnchorRecord::new("run-1", "acme", d.manifest_root(), 3, 0).unwrap();
        assert!(!verify_against_anchor(short, &key(), &a_same_count).ok, "la racine trahit aussi");
    }

    #[test]
    fn appending_a_forged_node_also_fails() {
        // La troncature n'est pas la seule attaque : ajouter aussi.
        let d = dag(3);
        let a = anchor_of(&d, 3);
        let longer = dag(4);
        assert!(!verify_against_anchor(longer.records(), &key(), &a).ok, "un nœud de plus trahit");
    }
}

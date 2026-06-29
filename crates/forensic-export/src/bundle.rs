// SPDX-License-Identifier: UNLICENSED
//! **L'assembleur** — `assemble` produit un bundle forensic autonome depuis le seam d'export de S2.
//!
//! Discipline (archi S4 §11) : **re-prouver** d'abord (défense en profondeur — on ne fait PAS confiance à
//! un flag `integrity_ok` ; on re-dérive la chaîne), **refuser** un store cassé, **vérifier** que le root
//! annoncé concorde, puis écrire les fichiers + **vendorer** la source du vérifieur + le **MANIFEST** + le
//! **README** du sceptique. La **clé** sert à re-prouver mais n'est **JAMAIS** écrite (I-S4-SANS-CLÉ,
//! structurel : aucun chemin de code n'écrit la clé).

use crate::anchor::{self, AnchorProof};
use crate::{manifest, readme, vendor};
use provenance_dag::{reprove_dag, verify_against_root, Record, SealedDag, TenantKey};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// L'entrée de l'assembleur — le **contenu** d'export (mappé depuis l'`ExportContent` de S2 par l'appelant ;
/// forensic-export reste découplé de l'audit-api).
#[derive(Debug, Clone)]
pub struct BundleInput {
    /// Le DAG scellé en JSONL (la PREUVE).
    pub records_jsonl: String,
    /// Le manifest-root annoncé (l'ENGAGEMENT) — re-vérifié contre la re-preuve.
    pub manifest_root: String,
    /// Le récit rendu (une VUE — chaque ligne cite son node_id).
    pub report: String,
    /// La portée scellée (ex. « full-tenant-dag … ») — pour le README.
    pub sealed_scope: String,
    /// Les cas que le récit raconte — pour le README.
    pub report_cases: Vec<String>,
}

/// Les options d'assemblage.
#[derive(Debug, Clone, Default)]
pub struct BundleOpts {
    /// **Preuve d'ancrage** STRUCTURÉE (§6) — obtenue par le déploiement (forensic-export ne fait PAS
    /// l'appel réseau). On la **vérifie** (liaison de root) avant de l'écrire ; un ancrage qui commet un
    /// AUTRE root est refusé. `None` = pas d'ancrage. Cf. [`crate::anchor`].
    pub anchor: Option<AnchorProof>,
    /// Hash du binaire de référence (build reproductible côté déploiement) — optionnel ; le tiers re-compile.
    pub expected_binary_sha256: Option<String>,
    /// Étiquette du tenant pour le README (PAS la clé).
    pub tenant_label: String,
}

/// Erreur d'assemblage.
#[derive(Debug)]
pub enum ExportError {
    /// E/S sur le disque.
    Io(std::io::Error),
    /// Le JSONL des records est invalide.
    BadJsonl(String),
    /// Le store **ne re-prouve pas** — on refuse d'empaqueter du non-prouvé.
    BrokenStore { broken_at: Option<usize>, reason: Option<String> },
    /// Le `manifest_root` annoncé ne concorde pas avec celui re-dérivé.
    RootMismatch { expected: String, got: String },
    /// Le dossier de sortie existe déjà et n'est **pas vide** (on refuse de polluer le bundle).
    DirNotEmpty(PathBuf),
    /// Le récit cite un `node_id` **absent** des records re-prouvés (récit non adossé — RÉCIT=VUE imposé
    /// à l'assemblage : aucune citation fantôme ne part dans un bundle).
    ReportCitation(String),
    /// La preuve d'ancrage commet un **autre** root que le DAG empaqueté (ancrage non probant — refusé).
    AnchorRootMismatch { bound: String, expected: String },
    /// La preuve d'ancrage est **mal formée** (émetteur vide, caractère de contrôle, reçu manquant pour
    /// une méthode cryptographique…) — on n'empaquette pas un ancrage non vérifiable.
    AnchorMalformed(String),
}
impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io(e) => write!(f, "E/S : {e}"),
            ExportError::BadJsonl(e) => write!(f, "JSONL invalide : {e}"),
            ExportError::BrokenStore { broken_at, reason } => write!(
                f,
                "store altéré (record {:?}) — export refusé : {}",
                broken_at,
                reason.clone().unwrap_or_default()
            ),
            ExportError::RootMismatch { expected, got } => {
                write!(f, "manifest-root annoncé « {expected} » ≠ re-dérivé « {got} »")
            }
            ExportError::DirNotEmpty(p) => write!(f, "dossier de sortie non vide : {}", p.display()),
            ExportError::ReportCitation(node) => write!(f, "le récit cite un nœud absent des records : « {node} »"),
            ExportError::AnchorRootMismatch { bound, expected } => {
                write!(f, "ancrage non probant : commet « {bound} » ≠ root du DAG « {expected} »")
            }
            ExportError::AnchorMalformed(m) => write!(f, "ancrage mal formé : {m}"),
        }
    }
}
impl std::error::Error for ExportError {}
impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        ExportError::Io(e)
    }
}

/// Le résumé de ce qui a été écrit (les fichiers + leur sha256, l'engagement).
#[derive(Debug, Clone)]
pub struct BundleManifest {
    pub dir: PathBuf,
    pub files: Vec<(String, String)>,
    pub manifest_root: String,
    pub n_records: usize,
}

/// **Assemble** un bundle forensic sous `out_dir`. `key` re-prouve les records (et n'est JAMAIS écrite).
pub fn assemble(
    input: &BundleInput,
    key: &TenantKey,
    opts: &BundleOpts,
    out_dir: &Path,
) -> Result<BundleManifest, ExportError> {
    // ── 1. parser ──
    let records = SealedDag::records_from_jsonl(&input.records_jsonl).map_err(|e| ExportError::BadJsonl(e.to_string()))?;

    // ── 2. RE-PROUVER (défense en profondeur — pas de confiance en un flag) → refuser si cassé ──
    let v = reprove_dag(&records, key);
    if !v.ok {
        return Err(ExportError::BrokenStore { broken_at: v.broken_at, reason: v.reason });
    }

    // ── 3. le root annoncé doit concorder avec le re-dérivé ──
    if v.manifest_root.as_deref() != Some(input.manifest_root.as_str()) {
        return Err(ExportError::RootMismatch {
            expected: input.manifest_root.clone(),
            got: v.manifest_root.unwrap_or_default(),
        });
    }
    // (cohérence belt-and-suspenders : verify_against_root rend le même verdict)
    debug_assert!(verify_against_root(&records, key, &input.manifest_root).ok);

    // ── 3bis. Garde ANTI-CITATION-FANTÔME : aucune citation vers un nœud absent ne part dans un bundle
    //    (les records sont re-prouvés, donc l'ensemble des node_id est de confiance). N.B. : ceci ne
    //    vérifie PAS la justesse des affirmations du récit — la régénération exacte « récit = vue » est
    //    la propriété du rendeur S2 (cf. tests/recit_is_a_view.rs), obtenu séparément. ──
    validate_report_citations(&records, &input.report)?;

    // ── 3ter. ANCRAGE (§6) : s'il est fourni, il doit être PROBANT pour CE DAG — on refuse un ancrage
    //    qui commet un autre root (RootMismatch) ET un ancrage mal formé (Malformed). Match exhaustif :
    //    une future variante d'AnchorOutcome casserait la compilation plutôt que de filer en douce. ──
    if let Some(proof) = &opts.anchor {
        match anchor::verify_anchor(proof, &input.manifest_root) {
            anchor::AnchorOutcome::Bound { .. } => {}
            anchor::AnchorOutcome::RootMismatch { bound, expected } => return Err(ExportError::AnchorRootMismatch { bound, expected }),
            anchor::AnchorOutcome::Malformed(m) => return Err(ExportError::AnchorMalformed(m)),
        }
    }

    // ── 4. garde : out_dir doit être absent ou VIDE. On **propage** l'erreur de `read_dir` (un échec
    //    de lecture ne vaut PAS « vide » — sinon on écrirait par-dessus un dossier illisible). ──
    if out_dir.exists() && out_dir.read_dir()?.next().is_some() {
        return Err(ExportError::DirNotEmpty(out_dir.to_path_buf()));
    }

    // ── 5. écriture ATOMIQUE : tout dans un staging voisin → `rename` final. Sur la moindre erreur, on
    //    efface le staging — **jamais** de demi-bundle « valide » laissé sur le disque (REFUSE-CASSÉ). ──
    let staging = staging_dir(out_dir);
    let _ = std::fs::remove_dir_all(&staging); // efface un staging résiduel d'un run précédent
    let files = match write_all(&staging, input, opts, records.len()) {
        Ok(files) => files,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    // Finalise : remplace out_dir (absent ou vide) par le staging, atomiquement (même système de fichiers).
    let finalize = || -> std::io::Result<()> {
        if out_dir.exists() {
            std::fs::remove_dir(out_dir)?; // vide (garanti par la garde) → remove_dir OK
        }
        std::fs::rename(&staging, out_dir)
    };
    if let Err(e) = finalize() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(ExportError::Io(e));
    }

    Ok(BundleManifest {
        dir: out_dir.to_path_buf(),
        files,
        manifest_root: input.manifest_root.clone(),
        n_records: records.len(),
    })
}

/// Le dossier de staging voisin (même parent → `rename` atomique), propre au processus.
fn staging_dir(out_dir: &Path) -> PathBuf {
    let name = out_dir.file_name().and_then(|n| n.to_str()).unwrap_or("bundle");
    out_dir.with_file_name(format!(".{name}.staging-{}", std::process::id()))
}

/// Écrit TOUS les fichiers du bundle (+ le MANIFEST en dernier) sous `dir`. La clé n'apparaît **nulle part**.
fn write_all(dir: &Path, input: &BundleInput, opts: &BundleOpts, n_records: usize) -> Result<Vec<(String, String)>, ExportError> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("records.sealed.jsonl"), &input.records_jsonl)?;
    std::fs::write(dir.join("manifest.root"), format!("{}\n", input.manifest_root))?;
    std::fs::write(dir.join("report.md"), &input.report)?;
    vendor::write_verify_src(&dir.join("verify"))?;
    if let Some(proof) = &opts.anchor {
        // (assemble a déjà validé l'ancrage ; encode peut néanmoins refuser un champ de contrôle.)
        let encoded = proof.encode().map_err(|e| ExportError::AnchorMalformed(e.to_string()))?;
        std::fs::write(dir.join("anchor.proof"), encoded)?; // preuve STRUCTURÉE (cf. anchor.rs)
    }
    if let Some(hash) = &opts.expected_binary_sha256 {
        std::fs::write(dir.join("EXPECTED_BINARY.sha256"), format!("{hash}\n"))?;
    }
    let label = if opts.tenant_label.is_empty() { "tenant" } else { opts.tenant_label.as_str() };
    let readme = readme::render(&readme::ReadmeCtx {
        tenant_label: label,
        manifest_root: &input.manifest_root,
        n_records,
        sealed_scope: &input.sealed_scope,
        report_cases: &input.report_cases,
        vendored_lines: vendor::vendored_line_count(),
        anchor: opts.anchor.as_ref(),
    });
    std::fs::write(dir.join("README.md"), readme)?;
    manifest::write_manifest(dir).map_err(ExportError::Io)
}

/// Extrait les `node_id` cités dans le récit : les tokens entre backticks contenant « :: » (la convention
/// de citation de S2, ex. `` `SUP-A::sanction` ``). On ne filtre **pas** sur l'espace — un `node_id` PEUT
/// en contenir (S1 n'interdit que « :: » et le vide dans un segment), donc une citation à espace doit être
/// validée, pas ignorée.
fn extract_cited_node_ids(report: &str) -> impl Iterator<Item = &str> {
    report.split('`').filter(|t| t.contains("::"))
}

/// **Garde anti-citation-fantôme** : tout `node_id` cité par le récit doit exister dans les records
/// re-prouvés — aucune citation vers un nœud absent ne part dans un bundle. (Ceci NE vérifie PAS la
/// justesse des affirmations ; la régénération exacte « récit = vue » est la propriété du rendeur S2,
/// obtenu séparément — cf. `tests/recit_is_a_view.rs` et le README.)
fn validate_report_citations(records: &[Record], report: &str) -> Result<(), ExportError> {
    let ids: HashSet<&str> = records.iter().map(|r| r.node.id()).collect();
    for cited in extract_cited_node_ids(report) {
        if !ids.contains(cited) {
            return Err(ExportError::ReportCitation(cited.to_string()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use provenance_dag::{Node, SealedDag};

    /// Octets de clé FIXES connus (pour le test « sans clé » : on sait quel hex chercher).
    const KEY_BYTES: [u8; 32] = [0x2au8; 32];
    fn key() -> TenantKey {
        TenantKey::from_bytes(KEY_BYTES)
    }
    fn key_hex() -> String {
        KEY_BYTES.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn good_input() -> BundleInput {
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "SUP-A::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        BundleInput {
            records_jsonl: d.to_jsonl(),
            manifest_root: d.manifest_root(),
            report: "# Dossier SUP-A — PENDING\n- sanction_check : PASS _[`SUP-A::sanction`]_\n".into(),
            sealed_scope: "full-tenant-dag".into(),
            report_cases: vec!["SUP-A".into()],
        }
    }

    fn fresh_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("s4-bundle-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn assembles_a_complete_self_contained_bundle() {
        let dir = fresh_dir("ok");
        let m = assemble(&good_input(), &key(), &BundleOpts { tenant_label: "acme".into(), ..Default::default() }, &dir).unwrap();
        assert_eq!(m.n_records, 1);
        for f in ["records.sealed.jsonl", "manifest.root", "report.md", "README.md", "MANIFEST.sha256", "verify/provenance-verify/src/main.rs", "verify/provenance-dag/src/dag.rs", "verify/build.sh"] {
            assert!(dir.join(f).exists(), "manquant : {f}");
        }
        // le MANIFEST couvre les fichiers (récursivement) et pas lui-même.
        let man = std::fs::read_to_string(dir.join("MANIFEST.sha256")).unwrap();
        assert!(man.contains("records.sealed.jsonl"));
        assert!(man.contains("verify/provenance-dag/src/dag.rs"), "le MANIFEST couvre la source vendorée");
        assert!(!man.contains("MANIFEST.sha256"), "le MANIFEST ne se hache pas lui-même");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundle_never_contains_the_key() {
        // I-S4-SANS-CLÉ : aucun fichier du bundle ne contient les octets (hex) de la clé.
        let dir = fresh_dir("nokey");
        assemble(&good_input(), &key(), &BundleOpts::default(), &dir).unwrap();
        let kh = key_hex();
        let files = crate::manifest::list_files(&dir).unwrap();
        for rel in files {
            let bytes = std::fs::read(dir.join(&rel)).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains(&kh), "la clé NE doit JAMAIS apparaître ({rel})");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_a_broken_store() {
        // I-S4-REFUSE-CASSÉ : on n'empaquette jamais du non-prouvé.
        let dir = fresh_dir("broken");
        let mut bad = good_input();
        bad.records_jsonl = bad.records_jsonl.replace("PASS", "FAIL"); // altère sans re-sceller
        let r = assemble(&bad, &key(), &BundleOpts::default(), &dir);
        assert!(matches!(r, Err(ExportError::BrokenStore { .. })), "store altéré → refusé");
        assert!(!dir.exists() || std::fs::read_dir(&dir).map(|mut d| d.next().is_none()).unwrap_or(true), "rien d'écrit");
    }

    #[test]
    fn refuses_a_wrong_announced_root() {
        let dir = fresh_dir("root");
        let mut bad = good_input();
        bad.manifest_root = "ff".repeat(32); // root annoncé faux
        assert!(matches!(assemble(&bad, &key(), &BundleOpts::default(), &dir), Err(ExportError::RootMismatch { .. })));
    }

    #[test]
    fn refuses_a_nonempty_output_dir() {
        let dir = fresh_dir("nonempty");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stray.txt"), b"parasite").unwrap();
        assert!(matches!(assemble(&good_input(), &key(), &BundleOpts::default(), &dir), Err(ExportError::DirNotEmpty(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn structured_anchor_is_carried_and_root_bound() {
        let dir = fresh_dir("anchor");
        let input = good_input();
        let proof = AnchorProof {
            method: anchor::AnchorMethod::Rfc3161,
            issuer: "acme-tsa".into(),
            bound_root: input.manifest_root.clone(), // ancre CE root
            timestamp: 1_700_000_000,
            receipt: vec![0xde, 0xad, 0xbe, 0xef],
        };
        let opts = BundleOpts { anchor: Some(proof.clone()), ..Default::default() };
        assemble(&input, &key(), &opts, &dir).unwrap();
        // anchor.proof est la preuve STRUCTURÉE, re-décodable à l'identique.
        let written = std::fs::read(dir.join("anchor.proof")).unwrap();
        assert_eq!(AnchorProof::decode(&written).unwrap(), proof);
        assert!(std::fs::read_to_string(dir.join("README.md")).unwrap().contains("rfc3161"), "le README décrit l'ancrage");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_an_anchor_committing_a_different_root() {
        // Cœur de l'attaque « ancrer un autre DAG » : un ancrage qui ne commet pas CE root est refusé.
        let dir = fresh_dir("anchor-bad");
        let proof = AnchorProof { method: anchor::AnchorMethod::Declared, issuer: "x".into(), bound_root: "00".repeat(32), timestamp: 1, receipt: vec![] };
        let opts = BundleOpts { anchor: Some(proof), ..Default::default() };
        assert!(matches!(assemble(&good_input(), &key(), &opts, &dir), Err(ExportError::AnchorRootMismatch { .. })));
    }

    #[test]
    fn refuses_a_malformed_anchor() {
        // Pas seulement RootMismatch : un ancrage MAL FORMÉ (méthode crypto sans reçu) est aussi refusé.
        let dir = fresh_dir("anchor-malformed");
        let input = good_input();
        let proof = AnchorProof { method: anchor::AnchorMethod::Rfc3161, issuer: "tsa".into(), bound_root: input.manifest_root.clone(), timestamp: 1, receipt: vec![] };
        let opts = BundleOpts { anchor: Some(proof), ..Default::default() };
        assert!(matches!(assemble(&input, &key(), &opts, &dir), Err(ExportError::AnchorMalformed(_))), "rfc3161 sans reçu → refusé");
    }

    #[test]
    fn refuses_a_report_citing_a_phantom_node() {
        // RÉCIT=VUE à l'assemblage : un récit qui cite un nœud absent des records est refusé.
        let dir = fresh_dir("phantom");
        let mut input = good_input();
        input.report = "- contrôle bidon _[`SUP-A::fantome`]_\n".into();
        assert!(matches!(assemble(&input, &key(), &BundleOpts::default(), &dir), Err(ExportError::ReportCitation(ref n)) if n == "SUP-A::fantome"));
    }
}

//! # `meta_provenance` — ÉMET un bundle de provenance du consensus de méta-essaim,
//! re-prouvable **hors-ligne, sans nous** par le binaire autonome `verify_meta`.
//!
//! Ce prototype ferme le seul trou du couplage « nano + essaim + méta-essaim » à la
//! couche de confiance : le méta-essaim avait une enveloppe de sûreté et un transport
//! mTLS, mais **aucune re-preuve de son consensus**. Ici, chaque pas de convergence
//! (indice, résidu, méta-consensus *après projection de sûreté*) est scellé dans une
//! chaîne HMAC via la crypte **partagée** [`efe_audit_core`] (`chain_hmac` + HKDF),
//! **byte-identique** à `efe-nano` et `efe-swarm-provenance` (vecteur golden gelé,
//! vérifié ci-dessous par `assert`). On écrit ensuite un **bundle** de deux fichiers
//! + une variante altérée, que le binaire séparé `verify_meta` re-prouve avec la seule
//! clé tenant (jamais écrite dans le bundle).
//!
//! Additif STRICT : dev-only ; `efe-meta-swarm` reste hors du binaire de prod (les deps
//! sont en `[dev-dependencies]` → zéro impact sur le graphe par défaut).
//!
//! ```text
//! cargo run --release -p efe-meta-swarm --example meta_provenance            # → ~/Downloads/meta-provenance-bundle
//! cargo run -p efe-meta-verify --bin verify_meta -- <dir>/meta_steps.jsonl <dir>/meta_manifest.json --key-hex <hex>
//! ```

use efe_audit_core::HmacKeySource;
use efe_meta_swarm::prelude::*;
use efe_meta_verify::{seal_in_place, to_hex, MetaManifest, SealedMetaStep};
use efe_nano_sdk::TenantId;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    // ── 1. Le MÊME méta-essaim Kovant (3 essaims), tracing activé → trace par-pas ──
    let meta = MetaSwarm::builder()
        .id("kovant-procurement-v1")
        .meta_space_dim(256)
        .add_participant(MockParticipant::with_contraction("supplier-scoring", 384, 0.6, 11)?.boxed())
        .add_participant(MockParticipant::with_contraction("contract-risk", 512, 0.7, 22)?.boxed())
        .add_participant(MockParticipant::with_contraction("spend-categorization", 384, 0.5, 33)?.boxed())
        .lift_for("supplier-scoring", LinearLift::identity(384, 256)?.boxed())
        .lift_for("contract-risk", LinearLift::identity(512, 256)?.boxed())
        .lift_for("spend-categorization", LinearLift::identity(384, 256)?.boxed())
        .pull_for("supplier-scoring", LinearPull::identity(256, 384)?.boxed())
        .pull_for("contract-risk", LinearPull::identity(256, 512)?.boxed())
        .pull_for("spend-categorization", LinearPull::identity(256, 384)?.boxed())
        .meta_operator(UniformMetaOp::new(256).boxed())
        .meta_safety(BoxSafety::symmetric(256, 10.0).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(1e-4)
        .max_iters(1000)
        .trace(true)
        .build()?;

    let result = meta.iterate()?;
    let trace = result.trace.expect("trace activée (.trace(true)) → présente");
    let src = trace.steps();
    println!("méta-essaim « {} » : {} pas de consensus, résidu final {:.3e}, arrêt {:?}", meta.id(), src.len(), result.final_residual, result.stop_reason);

    // ── 2. Construire les pas scellables (contenu PUBLIC — la clé n'y entre jamais) ──
    let mut steps: Vec<SealedMetaStep> = src
        .iter()
        .map(|s| SealedMetaStep {
            meta_id: meta.id().to_string(),
            step: s.step as u64,
            residual: s.residual,
            consensus: s.meta_consensus.as_slice().iter().map(|&v| f64::from(v)).collect(),
            seal: String::new(),
        })
        .collect();

    // ── 3. Clé tenant (crypto PARTAGÉE, HKDF byte-identique nano) + sceller la chaîne ──
    let key = HmacKeySource::test_key().derive(TenantId::system()).expect("clé tenant dérivée");
    let key_hex = to_hex(&key);
    assert_eq!(key_hex, "4f849020850db9409ad503b209412186e588114247a023ad70b128b0acfb560d", "clé golden byte-identique efe-nano");
    let root = seal_in_place(&mut steps, &key);

    // ── 4. Écrire le bundle : steps.jsonl + manifest.json + une variante altérée (D2) ──
    let dir = out_dir();
    write_jsonl(&dir, "meta_steps.jsonl", &steps)?;
    let manifest = MetaManifest { meta_id: meta.id().to_string(), n_steps: steps.len(), root: root.clone() };
    write(&dir, "meta_manifest.json", &(serde_json::to_string_pretty(&manifest).expect("manifeste sérialisable") + "\n"))?;

    // Altération : on modifie une valeur du CONTENU d'un pas médian ; son sceau embarqué
    // devient périmé → verify_meta recompute un hash différent et casse À ce pas.
    let k = steps.len() / 2;
    let mut tampered = steps.clone();
    tampered[k].consensus[0] += 1.0;
    write_jsonl(&dir, "meta_steps.TAMPERED.jsonl", &tampered)?;

    // ── 5. Le mode d'emploi (clé + root + les 2 commandes verify_meta) ──
    print_guide(&dir, &key_hex, &root, steps.len(), k);
    Ok(())
}

/// Écrit une suite de pas en JSONL (une `SealedMetaStep` par ligne).
fn write_jsonl(dir: &Path, name: &str, steps: &[SealedMetaStep]) -> std::io::Result<()> {
    let mut body = String::new();
    for s in steps {
        body.push_str(&serde_json::to_string(s).expect("pas sérialisable"));
        body.push('\n');
    }
    write(dir, name, &body)
}

/// Écrit `dir/name` en créant les dossiers parents au besoin.
fn write(dir: &Path, name: &str, content: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(name), content)
}

/// Dossier de sortie : `argv[1]` s'il est fourni, sinon `~/Downloads/meta-provenance-bundle`.
fn out_dir() -> PathBuf {
    if let Some(arg) = std::env::args().nth(1) {
        return PathBuf::from(arg);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Downloads").join("meta-provenance-bundle")
}

fn print_guide(dir: &Path, key_hex: &str, root: &str, n: usize, k: usize) {
    let d = dir.display();
    println!("\n═══════════════════════════════════════════════════════════════════════════");
    println!("  BUNDLE PROVENANCE MÉTA-ESSAIM écrit dans :\n    {d}");
    println!("───────────────────────────────────────────────────────────────────────────");
    println!("  clé tenant (hex, golden)  : {key_hex}");
    println!("  meta_provenance_root      : {root}");
    println!("  pas de consensus scellés  : {n}");
    println!("───────────────────────────────────────────────────────────────────────────");
    println!("  Re-preuve AUTONOME (sans nous, hors-ligne) — 2 fichiers + 1 clé :");
    println!("  ✓ intègre (exit 0) :");
    println!("      cargo run -q -p efe-meta-verify --bin verify_meta -- \"{d}/meta_steps.jsonl\" \"{d}/meta_manifest.json\" --key-hex {key_hex}");
    println!("  ✗ altéré (exit 1, cassé au pas {k}) :");
    println!("      cargo run -q -p efe-meta-verify --bin verify_meta -- \"{d}/meta_steps.TAMPERED.jsonl\" \"{d}/meta_manifest.json\" --key-hex {key_hex}");
    println!("═══════════════════════════════════════════════════════════════════════════\n");
}

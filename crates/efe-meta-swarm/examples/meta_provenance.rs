//! # `meta_provenance` — la provenance du consensus **scellée PAR LE MOTEUR** (v1 câblée).
//!
//! Contrairement au prototype (qui scellait la *trace* après coup), ici chaque pas de consensus est
//! scellé **au fil de l'itération**, dans le moteur, via un `StepObserver` : `iterate_with_provenance`
//! rend le résultat **et** un [`MetaProvenance`] (root + pas scellés, **sans clé**) — la provenance est
//! une sortie de première classe. Noter qu'on n'active **pas** la trace : la provenance en est
//! indépendante. Le bundle produit est **identique** à P1.1/P1.2 → re-prouvable par `verify_meta` ET
//! `verify_meta_mini` (zéro-dép), hors-ligne, sans nous.
//!
//! ```text
//! cargo run --release -p efe-meta-swarm --example meta_provenance --features provenance
//! ```

use efe_meta_swarm::prelude::*;
use efe_meta_swarm::provenance::{HmacKeySource, TenantId};
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    // ── 1. Le méta-essaim Kovant (3 essaims). Pas de .trace() : la provenance est indépendante. ──
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
        // Enveloppe SERRÉE (±0.4) : le garde-fou agit réellement → `safety_delta` non nul, pour
        // montrer que l'activation de l'enveloppe de sûreté devient re-prouvable (v2).
        .meta_safety(BoxSafety::symmetric(256, 0.4).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(1e-4)
        .max_iters(1000)
        .build()?;

    // ── 2. Itérer EN SCELLANT chaque pas (câblage moteur natif). La clé est dérivée, tenue le temps
    //       du run, puis droppée : elle n'entre jamais dans `prov`. ──
    let (result, prov) = meta.iterate_with_provenance(&HmacKeySource::test_key(), TenantId::system())?;
    println!("=== Méta-essaim « {} » — provenance scellée PAR LE MOTEUR (v1) ===", meta.id());
    println!("→ {} pas · résidu final {:.3e} · arrêt {:?}", result.iterations, result.final_residual, result.stop_reason);
    println!("→ meta_provenance_root : {}", prov.root);
    let max_delta = prov.steps.iter().filter_map(|s| s.safety_delta).fold(0.0_f64, f64::max);
    let n_active = prov.steps.iter().filter(|s| s.safety_delta.unwrap_or(0.0) > 0.0).count();
    println!("→ enveloppe de sûreté (v2, scellée) : max ‖z−z̃‖ = {max_delta:.4} · active à {n_active}/{} pas", prov.steps.len());
    let n_contrib = prov.steps.first().and_then(|s| s.vote_hashes.as_ref()).map_or(0, Vec::len);
    println!("→ contributions scellées (v3) : {n_contrib} hash/pas (1 par essaim, « qui a contribué quoi ») · schéma v{}", prov.manifest().schema_version);

    // ── 3. La clé tenant (pour la re-preuve — re-dérivée à part, jamais dans le bundle). ──
    let key = HmacKeySource::test_key().derive(TenantId::system()).expect("clé tenant dérivée");
    let key_hex: String = key.iter().map(|b| format!("{b:02x}")).collect();

    // ── 4. Écrire le bundle (meta_steps.jsonl + meta_manifest.json) + une variante altérée (D2). ──
    let dir = out_dir();
    prov.write_bundle(&dir).expect("écriture du bundle");
    let k = prov.steps.len() / 2;
    let mut tampered = prov.steps.clone();
    tampered[k].consensus[0] += 1.0; // un octet du contenu du pas médian
    let mut jsonl = String::new();
    for s in &tampered {
        jsonl.push_str(&serde_json::to_string(s).expect("SealedMetaStep sérialisable"));
        jsonl.push('\n');
    }
    std::fs::write(dir.join("meta_steps.TAMPERED.jsonl"), jsonl).expect("écriture altérée");

    // ── 5. Le mode d'emploi : les DEUX vérifieurs (pratique + zéro-dép). ──
    print_guide(&dir, &key_hex, &prov.root, prov.steps.len(), k);
    Ok(())
}

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
    println!("  BUNDLE (scellé par le moteur) écrit dans :\n    {d}");
    println!("───────────────────────────────────────────────────────────────────────────");
    println!("  clé tenant (hex)      : {key_hex}");
    println!("  meta_provenance_root  : {root}");
    println!("  pas de consensus      : {n}");
    println!("───────────────────────────────────────────────────────────────────────────");
    println!("  Re-preuve hors-ligne, sans nous — DEUX vérifieurs (bundle identique) :");
    println!("  ✓ pratique (P1.1)        : cargo run -q -p efe-meta-verify --bin verify_meta -- \"{d}/meta_steps.jsonl\" \"{d}/meta_manifest.json\" --key-hex {key_hex}");
    println!("  ✓ zéro-dép (P1.2)        : cargo run -q -p verify-meta-mini --bin verify_meta_mini -- \"{d}/meta_steps.jsonl\" \"{d}/meta_manifest.json\" --key-hex {key_hex}");
    println!("  ✗ altéré (exit 1, pas {k}) : cargo run -q -p verify-meta-mini --bin verify_meta_mini -- \"{d}/meta_steps.TAMPERED.jsonl\" \"{d}/meta_manifest.json\" --key-hex {key_hex}");
    println!("═══════════════════════════════════════════════════════════════════════════\n");
}

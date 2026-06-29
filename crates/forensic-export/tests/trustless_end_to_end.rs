// SPDX-License-Identifier: UNLICENSED
//! **I-S4-TRUSTLESS / RECOMPILABLE / AUTO-CONTENU** (test d'intégration, hors-process) — le cœur du fossé,
//! désormais un test qui **casse le build** plutôt qu'une démo manuelle. On assemble un vrai bundle, puis,
//! **comme le ferait un tiers**, on **re-compile la source vendorée** du vérifieur (hors du dépôt) et on la
//! **fait tourner** sur les records : exit **0** sur l'authentique, exit **1** sur l'altéré.
//!
//! Caveat de cache honnête : la build standalone tire `serde` du cache cargo. On compile en `--offline`
//! (cache chaud) ; si le cache est **froid** (CI air-gappée sans cache), on **saute** proprement le test
//! (sans le faire échouer). Mais une **erreur de compilation** de la source vendorée (régression réelle du
//! fossé) fait, elle, **échouer** le test — c'est exactement la garde qui manquait.

use forensic_export::{assemble, BundleInput, BundleOpts};
use provenance_dag::{Node, SealedDag, TenantKey};
use std::path::Path;
use std::process::Command;

const KEY_BYTES: [u8; 32] = [0x2au8; 32];
fn key_hex() -> String {
    KEY_BYTES.iter().map(|b| format!("{b:02x}")).collect()
}

/// Assemble un bundle réel sous `dir` ; rend le manifest-root.
fn build_bundle(dir: &Path) -> String {
    let k = TenantKey::from_bytes(KEY_BYTES);
    let mut d = SealedDag::new();
    d.append(&k, Node::AgentAct { node_id: "SUP-A::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
    let input = BundleInput {
        records_jsonl: d.to_jsonl(),
        manifest_root: d.manifest_root(),
        report: "# Dossier SUP-A\n- sanction_check : PASS  _[`SUP-A::sanction`]_\n".into(),
        sealed_scope: "full-tenant-dag".into(),
        report_cases: vec!["SUP-A".into()],
    };
    assemble(&input, &k, &BundleOpts { tenant_label: "acme".into(), ..Default::default() }, dir).expect("assemblage").manifest_root
}

#[test]
fn third_party_recompiles_vendored_verify_and_reproves_offline() {
    let base = std::env::temp_dir().join(format!("s4-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let bundle = base.join("bundle");
    let root = build_bundle(&bundle);

    // 1. RE-COMPILER la source vendorée, hors-process, hors workspace (cible temporaire dédiée).
    let pv_manifest = bundle.join("verify/provenance-verify/Cargo.toml");
    let target = base.join("target");
    let build = Command::new("cargo")
        .args(["build", "--release", "--offline", "--manifest-path"])
        .arg(&pv_manifest)
        .env("CARGO_TARGET_DIR", &target)
        .output();
    let build = match build {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP trustless-e2e : cargo introuvable ({e})");
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
    };
    if !build.status.success() {
        let err = String::from_utf8_lossy(&build.stderr);
        // Une vraie erreur de COMPILATION (diagnostic rustc) est une régression du fossé → ÉCHEC DUR,
        // **prioritaire** sur le heuristique de cache (qui ne doit jamais masquer un code qui ne compile plus).
        let real_compile_error = err.contains("error[E") || err.contains("could not compile") || err.contains("error: expected") || err.contains("cannot find");
        let cache_miss = ["offline", "download", "no matching package", "registry index", "not available", "failed to get"]
            .iter()
            .any(|m| err.contains(m));
        if cache_miss && !real_compile_error {
            eprintln!("SKIP trustless-e2e : build offline indisponible (cache froid) — {}", err.lines().next().unwrap_or(""));
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        panic!("RÉGRESSION DU FOSSÉ : la source vendorée NE COMPILE PAS hors du dépôt :\n{err}");
    }
    let bin = target.join("release/provenance-verify");
    assert!(bin.exists(), "le vérifieur re-compilé est produit");

    // 2. RE-PROUVER le bundle authentique → exit 0.
    let records = bundle.join("records.sealed.jsonl");
    let ok = Command::new(&bin).arg(&records).args(["--key-hex", &key_hex(), "--root", &root]).output().unwrap();
    assert_eq!(ok.status.code(), Some(0), "bundle authentique → PASS (exit 0)\nstderr: {}", String::from_utf8_lossy(&ok.stderr));

    // 3. ALTÉRER un octet → exit 1, ET le verdict --json **localise** la falsification au record altéré
    //    (broken_at = 0) — on distingue ainsi un vrai tamper d'un échec clé/root/usage.
    let tampered = base.join("tampered.jsonl");
    std::fs::write(&tampered, std::fs::read_to_string(&records).unwrap().replacen("PASS", "FAIL", 1)).unwrap();
    let bad = Command::new(&bin).arg(&tampered).args(["--key-hex", &key_hex(), "--root", &root, "--json"]).output().unwrap();
    assert_eq!(bad.status.code(), Some(1), "bundle altéré → FAIL (exit 1)");
    let json = String::from_utf8_lossy(&bad.stdout);
    assert!(json.contains("\"ok\":false"), "verdict json ok:false : {json}");
    assert!(json.contains("\"broken_at\":0"), "la falsification est localisée au record 0 (pas un échec clé/root) : {json}");

    let _ = std::fs::remove_dir_all(&base);
}

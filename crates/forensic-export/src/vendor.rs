// SPDX-License-Identifier: UNLICENSED
//! **Le vendoring du vérifieur** — la SOURCE de `provenance-verify` + `provenance-dag` est **embarquée à
//! la compilation** (via [`include_str!`]) dans ce binaire, puis **écrite** dans le bundle. Ainsi le
//! bundle est **auto-contenu** : un tiers lit la source (~quelques centaines de lignes), la **re-compile**
//! lui-même, et n'a JAMAIS à faire confiance à un binaire livré (§5 de l'archi S4). On génère des
//! `Cargo.toml` **autonomes** (sans héritage de workspace) pour que le tout compile hors de notre dépôt.
//!
//! Honnêteté de dépendance : le vérifieur dépend de `provenance-dag` (cœur crypto écrit à la main, zéro
//! dépendance crypto externe) — qui pull `serde`/`serde_json` (standard, audité) pour le JSONL. Aucune
//! crate **crypto** externe, aucun code Kovant/EFE hors cette source. (Un `verify-mini` strictement
//! std-only, JSON fait main, serait le durcissement nommé pour un arbre de dépendances littéralement vide.)

use std::fs;
use std::path::Path;

// ── la source embarquée (recompilée dans forensic-export à chaque build → toujours à jour) ──
const DAG_LIB: &str = include_str!("../../provenance-dag/src/lib.rs");
const DAG_SHA256: &str = include_str!("../../provenance-dag/src/sha256.rs");
const DAG_HMAC: &str = include_str!("../../provenance-dag/src/hmac.rs");
const DAG_KEY: &str = include_str!("../../provenance-dag/src/key.rs");
const DAG_NODE: &str = include_str!("../../provenance-dag/src/node.rs");
const DAG_DAG: &str = include_str!("../../provenance-dag/src/dag.rs");
const VERIFY_MAIN: &str = include_str!("../../provenance-verify/src/main.rs");

/// `Cargo.toml` autonome pour le cœur (versions concrètes, pas d'héritage de workspace).
const DAG_CARGO: &str = "# Vérifieur forensic vendoré — cœur zéro-dépendance-crypto (SHA-256/HMAC écrits à la main).\n\
[package]\n\
name = \"provenance-dag\"\n\
version = \"0.0.1\"\n\
edition = \"2021\"\n\
\n\
[dependencies]\n\
serde = { version = \"1.0\", features = [\"derive\"] }\n\
serde_json = \"1.0\"\n";

/// `Cargo.toml` autonome pour le CLI vérifieur (dépend du cœur par chemin relatif).
const VERIFY_CARGO: &str = "# Vérifieur forensic vendoré — le CLI autonome. Dépend UNIQUEMENT du cœur ci-contre.\n\
[package]\n\
name = \"provenance-verify\"\n\
version = \"0.0.1\"\n\
edition = \"2021\"\n\
\n\
[[bin]]\n\
name = \"provenance-verify\"\n\
path = \"src/main.rs\"\n\
\n\
[dependencies]\n\
provenance-dag = { path = \"../provenance-dag\" }\n";

/// Script de build reproductible (lisez-le aussi : il ne fait que `cargo build`).
const BUILD_SH: &str = "#!/bin/sh\n\
# Re-compilez le vérifieur depuis CETTE source — ne nous faites pas confiance, lisez-la d'abord.\n\
# --offline d'abord (machine air-gappée avec un cache cargo chaud) ; sinon en ligne (récupère serde).\n\
set -e\n\
cd \"$(dirname \"$0\")/provenance-verify\"\n\
cargo build --release --offline 2>/dev/null || cargo build --release\n\
echo \"binaire : $(pwd)/target/release/provenance-verify\"\n\
echo \"usage   : ./provenance-verify ../../records.sealed.jsonl --key-hex <K_tenant> --root <manifest.root>\"\n";

/// **La spécification math**, langage-neutre, pour ré-implémenter le vérifieur dans N'IMPORTE quel langage
/// (et donc survivre même à Rust). Pure DATA — pas de parser, pas de crypto. Couvre la récurrence de
/// chaîne, l'encodage canonique BINAIRE des nœuds (la part qui DOIT être byte-exacte), et des vecteurs de
/// test SHA-256 / HMAC-SHA256 pour valider une ré-implémentation.
const SPEC_MD: &str = "# Spécification du vérifieur (langage-neutre)\n\n\
But : permettre de **ré-implémenter** la re-preuve dans tout langage et d'obtenir le **même** verdict —\n\
sans dépendre de Rust ni de nous. Tout ci-dessous est public et standard.\n\n\
## 1. Récurrence de chaîne\n\n\
```\n\
chain_0   = SHA256(\"efe-prov-dag:v1:genesis\")             (32 octets ; GENESIS = ces octets ASCII)\n\
seal_i    = HMAC-SHA256(K_tenant, chain_{i-1} || canonical(node_i))\n\
chain_i   = seal_i\n\
manifest_root = hex(chain_last)\n\
```\n\
La re-preuve recalcule chaque `seal_i` et le compare (temps constant) au sceau stocké (hex) du record i.\n\
Intégrité structurelle : les références d'un nœud doivent exister parmi les nœuds ANTÉRIEURS ; ids uniques.\n\n\
## 2. Encodage canonique BINAIRE d'un nœud (byte-exact — la part critique)\n\n\
Helpers : `put_str(s)` = `u32_be(longueur_octets(s)) || octets_utf8(s)` ; `put_u64(v)` = `v` en big-endian (8 octets).\n\
Chaque nœud = un octet de TYPE, puis ses champs dans CET ordre :\n\
```\n\
AgentAct (type 1) : put_str(node_id) put_str(agent_role) put_str(inputs_hash) put_str(model)\n\
                    put_str(version) put_str(output) u32_be(N) [put_str(evidence_ref) ×N] put_u64(timestamp)\n\
Edge     (type 2) : put_str(edge_id) put_str(from_node) put_str(to_node) put_str(artifact_hash)\n\
                    put_str(artifact_type) put_u64(timestamp)\n\
Evidence (type 3) : put_str(evidence_id) put_str(tool) put_str(query_hash) put_str(retrieved_ref)\n\
                    put_str(content_hash) put_str(consumed_by_node) put_u64(timestamp)\n\
Human    (type 4) : put_str(node_id) put_str(human_id) u8(action) put_str(target_node)\n\
                    put_str(justification) put_u64(timestamp)\n\
                    action : Approve=1 Override=2 Escalate=3 Comment=4\n\
```\n\
(Le JSONL ne sert qu'au stockage ; le sceau est calculé sur cet encodage binaire, non sur le JSON.)\n\n\
## 3. Vecteurs de test (validez votre ré-implémentation)\n\n\
SHA-256 (FIPS 180-4) :\n\
```\n\
SHA256(\"\")    = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n\
SHA256(\"abc\") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n\
```\n\
HMAC-SHA256 (RFC 4231, test 1 ; clé = 0x0b ×20, message = \"Hi There\") :\n\
```\n\
HMAC = b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7\n\
```\n\
Les modules de test du cœur (`sha256.rs`, `hmac.rs`) contiennent davantage de vecteurs (deux-blocs, million de 'a', RFC 4231 #4/#6).\n";

/// README du sous-dossier `verify/` (comment re-compiler &amp; exécuter).
const VERIFY_README: &str = "# Le vérifieur autonome (à re-compiler vous-même)\n\n\
Vous n'avez **rien** à nous faire confiance. La procédure :\n\n\
1. **Lisez** la source : `provenance-dag/src/*.rs` (le cœur : SHA-256 + HMAC écrits à la main, prouvés\n\
   par les vecteurs NIST/RFC inclus dans les tests) et `provenance-verify/src/main.rs` (le CLI). Quelques\n\
   centaines de lignes, lisibles en une heure.\n\
2. **Re-compilez** : `./build.sh` (ou `cd provenance-verify && cargo build --release`). Vous obtenez\n\
   VOTRE binaire — pas le nôtre.\n\
3. **Obtenez la clé tenant** auprès du responsable de traitement (elle n'est **PAS** dans ce bundle).\n\
4. **Vérifiez** : `./provenance-verify ../records.sealed.jsonl --key-hex <K_tenant> --root <manifest.root>`\n\
   → code de sortie **0** = record authentique et non-altéré ; **1** = altéré/réordonné/mauvaise clé.\n\n\
Dépendances : `provenance-dag` (cœur, zéro dépendance **crypto** externe) + `serde`/`serde_json`\n\
(standard, audité, pour le JSONL). Aucune crate crypto externe, aucun code Kovant/EFE.\n";

/// Écrit l'arbre source vendoré sous `verify_dir` (provenance-dag/, provenance-verify/, build.sh, README).
pub fn write_verify_src(verify_dir: &Path) -> std::io::Result<()> {
    // Le cœur.
    let dag_src = verify_dir.join("provenance-dag/src");
    fs::create_dir_all(&dag_src)?;
    fs::write(verify_dir.join("provenance-dag/Cargo.toml"), DAG_CARGO)?;
    fs::write(dag_src.join("lib.rs"), DAG_LIB)?;
    fs::write(dag_src.join("sha256.rs"), DAG_SHA256)?;
    fs::write(dag_src.join("hmac.rs"), DAG_HMAC)?;
    fs::write(dag_src.join("key.rs"), DAG_KEY)?;
    fs::write(dag_src.join("node.rs"), DAG_NODE)?;
    fs::write(dag_src.join("dag.rs"), DAG_DAG)?;
    // Le CLI.
    let pv_src = verify_dir.join("provenance-verify/src");
    fs::create_dir_all(&pv_src)?;
    fs::write(verify_dir.join("provenance-verify/Cargo.toml"), VERIFY_CARGO)?;
    fs::write(pv_src.join("main.rs"), VERIFY_MAIN)?;
    // Build + guide + spéc math (langage-neutre).
    fs::write(verify_dir.join("build.sh"), BUILD_SH)?;
    fs::write(verify_dir.join("README.md"), VERIFY_README)?;
    fs::write(verify_dir.join("SPEC.md"), SPEC_MD)?;
    Ok(())
}

/// Le nombre approximatif de lignes de source vendorée (pour le README : « lisible en une heure »).
pub fn vendored_line_count() -> usize {
    [DAG_LIB, DAG_SHA256, DAG_HMAC, DAG_KEY, DAG_NODE, DAG_DAG, VERIFY_MAIN]
        .iter()
        .map(|s| s.lines().count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_source_is_present_and_nonempty() {
        // Les 7 fichiers sont bien embarqués (sinon le bundle ne serait pas auto-contenu).
        for (name, src) in [
            ("dag/lib", DAG_LIB),
            ("dag/sha256", DAG_SHA256),
            ("dag/hmac", DAG_HMAC),
            ("dag/key", DAG_KEY),
            ("dag/node", DAG_NODE),
            ("dag/dag", DAG_DAG),
            ("verify/main", VERIFY_MAIN),
        ] {
            assert!(src.len() > 200, "source {name} embarquée non-vide");
        }
        // Sceau de cohérence : le cœur contient bien la primitive HMAC et la genèse.
        assert!(DAG_HMAC.contains("hmac_sha256"), "le cœur HMAC est embarqué");
        assert!(DAG_DAG.contains("reprove_dag"), "la re-preuve est embarquée");
        assert!(VERIFY_MAIN.contains("provenance_dag"), "le CLI référence le cœur");
        assert!(vendored_line_count() > 300, "quelques centaines de lignes");
    }

    #[test]
    fn writes_a_compilable_layout() {
        let dir = std::env::temp_dir().join(format!("s4-vendor-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_verify_src(&dir).unwrap();
        // La structure attendue est là.
        for p in [
            "provenance-dag/Cargo.toml",
            "provenance-dag/src/lib.rs",
            "provenance-dag/src/dag.rs",
            "provenance-verify/Cargo.toml",
            "provenance-verify/src/main.rs",
            "build.sh",
            "README.md",
        ] {
            assert!(dir.join(p).exists(), "manquant : {p}");
        }
        // Le Cargo.toml du CLI pointe le cœur par chemin (compilable hors workspace).
        let pv_cargo = std::fs::read_to_string(dir.join("provenance-verify/Cargo.toml")).unwrap();
        assert!(pv_cargo.contains("path = \"../provenance-dag\""));
        assert!(!pv_cargo.contains("workspace = true"), "Cargo.toml autonome (pas d'héritage workspace)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vendored_core_deps_are_exactly_serde() {
        // I-S4-ZÉRO-DÉP : le Cargo.toml du cœur vendoré ne déclare QUE serde/serde_json (aucune crate
        // crypto, aucune autre) — garde build-breaking contre une dérive de dépendance du fossé.
        let deps: std::collections::BTreeSet<&str> = DAG_CARGO
            .lines()
            .skip_while(|l| l.trim() != "[dependencies]")
            .skip(1)
            .take_while(|l| !l.trim_start().starts_with('['))
            .filter_map(|l| l.split('=').next().map(str::trim))
            .filter(|n| !n.is_empty())
            .collect();
        assert_eq!(deps, ["serde", "serde_json"].into_iter().collect(), "cœur = exactement {{serde, serde_json}} : {deps:?}");
        assert!(VERIFY_CARGO.contains("provenance-dag = { path = \"../provenance-dag\" }"));
        assert!(!VERIFY_CARGO.contains("serde"), "le CLI lui-même ne déclare pas serde directement");
    }

    #[test]
    fn spec_md_is_written_and_describes_the_recurrence() {
        let dir = std::env::temp_dir().join(format!("s4-spec-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_verify_src(&dir).unwrap();
        let spec = std::fs::read_to_string(dir.join("SPEC.md")).unwrap();
        assert!(spec.contains("efe-prov-dag:v1:genesis"), "la genèse est spécifiée");
        assert!(spec.contains("HMAC-SHA256"), "la récurrence HMAC est spécifiée");
        assert!(spec.contains("canonical"), "l'encodage canonique est spécifié");
        assert!(spec.contains("ba7816bf8f01cfea"), "vecteur SHA-256(abc) présent pour valider une ré-impl");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// SPDX-License-Identifier: UNLICENSED
//! **Le vendoring du vérifieur** — la SOURCE de `verify-mini` est **embarquée à la compilation** (via
//! [`include_str!`]) dans ce binaire, puis **écrite** dans le bundle. Le bundle est **auto-contenu** : un
//! tiers lit la source, la **re-compile** lui-même, et n'a JAMAIS à faire confiance à un binaire livré
//! (§5 de l'archi S4). On génère un `Cargo.toml` **autonome** pour que le tout compile hors de notre dépôt.
//!
//! Honnêteté de dépendance — **désormais SANS astérisque** : `verify-mini` a un arbre de dépendances
//! **littéralement vide** (SHA-256/HMAC écrits à la main, parseur JSONL fait main, pas de `serde`). Donc
//! `cargo tree` ne montre que `verify-mini`, et `cargo build --offline` réussit **sur une machine
//! air-gappée sans aucun cache** — il n'y a rien à télécharger. La fidélité de `verify-mini` au cœur S0
//! (manifest-root octet-à-octet) est verrouillée par son cross-check (`crates/verify-mini/tests/`).

use std::fs;
use std::path::Path;

// ── la source embarquée (recompilée dans forensic-export à chaque build → toujours à jour) ──
const VM_LIB: &str = include_str!("../../verify-mini/src/lib.rs");
const VM_SHA256: &str = include_str!("../../verify-mini/src/sha256.rs");
const VM_HMAC: &str = include_str!("../../verify-mini/src/hmac.rs");
const VM_JSON: &str = include_str!("../../verify-mini/src/json.rs");
const VM_RECORD: &str = include_str!("../../verify-mini/src/record.rs");
const VM_VERIFY: &str = include_str!("../../verify-mini/src/verify.rs");
const VM_MAIN: &str = include_str!("../../verify-mini/src/main.rs");

/// `Cargo.toml` autonome — **arbre de dépendances VIDE** (std-only), lib + bin.
const VM_CARGO: &str = "# Vérifieur forensic vendoré — ARBRE DE DÉPENDANCES LITTÉRALEMENT VIDE (std-only).\n\
# Aucune dépendance : SHA-256/HMAC écrits à la main, JSONL parsé à la main, encodage canonique ré-implémenté.\n\
[package]\n\
name = \"verify-mini\"\n\
version = \"0.0.1\"\n\
edition = \"2021\"\n\
\n\
[lib]\n\
path = \"src/lib.rs\"\n\
\n\
[[bin]]\n\
name = \"verify-mini\"\n\
path = \"src/main.rs\"\n\
\n\
[dependencies]\n";

/// Script de build reproductible (lisez-le aussi : il ne fait que `cargo build`). Arbre de dépendances
/// vide → `--offline` réussit **toujours** (rien à télécharger, même air-gappé).
const BUILD_SH: &str = "#!/bin/sh\n\
# Re-compilez le vérifieur depuis CETTE source — ne nous faites pas confiance, lisez-la d'abord.\n\
# Arbre de dépendances VIDE → compile hors-ligne, sans aucun téléchargement (même sur machine air-gappée).\n\
set -e\n\
cd \"$(dirname \"$0\")/verify-mini\"\n\
cargo build --release --offline\n\
echo \"binaire : $(pwd)/target/release/verify-mini\"\n\
echo \"usage   : ./verify-mini ../../records.sealed.jsonl --key-hex <K_tenant> --root <manifest.root>\"\n";

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
Les modules de test embarqués (`sha256.rs`, `hmac.rs`) contiennent davantage de vecteurs (deux-blocs, million de 'a', RFC 4231 #4/#6).\n";

/// README du sous-dossier `verify/` (comment re-compiler &amp; exécuter).
const VERIFY_README: &str = "# Le vérifieur autonome (à re-compiler vous-même) — ZÉRO dépendance\n\n\
Vous n'avez **rien** à nous faire confiance. La procédure :\n\n\
1. **Lisez** la source : `verify-mini/src/*.rs` — SHA-256 + HMAC écrits à la main (prouvés par les vecteurs\n\
   NIST/RFC dans les tests), le **parseur JSONL fait main**, l'**encodage canonique** ré-implémenté, la\n\
   **re-preuve**, et le CLI. ~1000 lignes, lisibles attentivement en une à deux heures.\n\
2. **Re-compilez** : `./build.sh` (ou `cd verify-mini && cargo build --release`). L'arbre de dépendances\n\
   est **VIDE** → aucun téléchargement, compile même **air-gappé** ; vous obtenez VOTRE binaire, pas le nôtre.\n\
3. **Obtenez la clé tenant** auprès du responsable de traitement (elle n'est **PAS** dans ce bundle).\n\
4. **Vérifiez** : `./verify-mini ../records.sealed.jsonl --key-hex <K_tenant> --root <manifest.root>`\n\
   → code de sortie **0** = record authentique et non-altéré ; **1** = altéré/réordonné/mauvaise clé.\n\n\
Dépendances : **AUCUNE** (std-only). `cargo tree` ne montre que `verify-mini` — vous ne faites confiance\n\
qu'aux mathématiques et à votre propre compilation. (La fidélité au cœur scellé est, elle, prouvée par un\n\
cross-check octet-à-octet contre l'implémentation de référence — cf. `SPEC.md` pour ré-implémenter ailleurs.)\n";

/// Écrit l'arbre source vendoré sous `verify_dir` (verify-mini/, build.sh, README, SPEC).
pub fn write_verify_src(verify_dir: &Path) -> std::io::Result<()> {
    let src = verify_dir.join("verify-mini/src");
    fs::create_dir_all(&src)?;
    fs::write(verify_dir.join("verify-mini/Cargo.toml"), VM_CARGO)?;
    fs::write(src.join("lib.rs"), VM_LIB)?;
    fs::write(src.join("sha256.rs"), VM_SHA256)?;
    fs::write(src.join("hmac.rs"), VM_HMAC)?;
    fs::write(src.join("json.rs"), VM_JSON)?;
    fs::write(src.join("record.rs"), VM_RECORD)?;
    fs::write(src.join("verify.rs"), VM_VERIFY)?;
    fs::write(src.join("main.rs"), VM_MAIN)?;
    // Build + guide + spéc math (langage-neutre).
    fs::write(verify_dir.join("build.sh"), BUILD_SH)?;
    fs::write(verify_dir.join("README.md"), VERIFY_README)?;
    fs::write(verify_dir.join("SPEC.md"), SPEC_MD)?;
    Ok(())
}

/// Le nombre approximatif de lignes de source vendorée (pour le README : « lisible en une à deux heures »).
pub fn vendored_line_count() -> usize {
    [VM_LIB, VM_SHA256, VM_HMAC, VM_JSON, VM_RECORD, VM_VERIFY, VM_MAIN]
        .iter()
        .map(|s| s.lines().count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_source_is_present_and_nonempty() {
        // Les 7 fichiers de verify-mini sont bien embarqués (sinon le bundle ne serait pas auto-contenu).
        for (name, src) in [
            ("lib", VM_LIB),
            ("sha256", VM_SHA256),
            ("hmac", VM_HMAC),
            ("json", VM_JSON),
            ("record", VM_RECORD),
            ("verify", VM_VERIFY),
            ("main", VM_MAIN),
        ] {
            assert!(src.len() > 200, "source {name} embarquée non-vide");
        }
        // Sceaux de cohérence : crypto, parseur, re-preuve, et CLI bien embarqués.
        assert!(VM_HMAC.contains("hmac_sha256"), "le cœur HMAC est embarqué");
        assert!(VM_VERIFY.contains("reprove"), "la re-preuve est embarquée");
        assert!(VM_JSON.contains("parse_object"), "le parseur JSONL fait main est embarqué");
        assert!(VM_RECORD.contains("canonical"), "l'encodage canonique est embarqué");
        assert!(VM_MAIN.contains("verify_mini"), "le CLI référence le lib");
        assert!(vendored_line_count() > 500, "plusieurs centaines de lignes");
    }

    #[test]
    fn writes_a_compilable_layout() {
        let dir = std::env::temp_dir().join(format!("s4-vendor-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_verify_src(&dir).unwrap();
        for p in [
            "verify-mini/Cargo.toml",
            "verify-mini/src/lib.rs",
            "verify-mini/src/sha256.rs",
            "verify-mini/src/json.rs",
            "verify-mini/src/record.rs",
            "verify-mini/src/verify.rs",
            "verify-mini/src/main.rs",
            "build.sh",
            "README.md",
            "SPEC.md",
        ] {
            assert!(dir.join(p).exists(), "manquant : {p}");
        }
        let cargo = std::fs::read_to_string(dir.join("verify-mini/Cargo.toml")).unwrap();
        assert!(!cargo.contains("workspace = true"), "Cargo.toml autonome (pas d'héritage workspace)");
        assert!(cargo.contains("path = \"src/main.rs\"") && cargo.contains("path = \"src/lib.rs\""), "lib + bin déclarés");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vendored_dep_tree_is_literally_empty() {
        // I-S4-ZÉRO-DÉP (durci) : le Cargo.toml vendoré ne déclare AUCUNE dépendance — `cargo tree` ne
        // montre que verify-mini. Garde build-breaking contre une dérive qui ré-introduirait serde & co.
        let deps: Vec<&str> = VM_CARGO
            .lines()
            .skip_while(|l| l.trim() != "[dependencies]")
            .skip(1)
            .take_while(|l| !l.trim_start().starts_with('['))
            .filter_map(|l| l.split('=').next().map(str::trim))
            .filter(|n| !n.is_empty() && !n.starts_with('#'))
            .collect();
        assert!(deps.is_empty(), "ZÉRO dépendance vendorée, trouvé : {deps:?}");
        assert!(!VM_CARGO.contains("serde"), "plus de serde dans le vérifieur vendoré (l'astérisque est levé)");
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

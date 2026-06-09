// SPDX-License-Identifier: UNLICENSED
//! **Garde-fous tenus — preuve continue** (architecture §7/§9/§11).
//!
//! Le mode phare promet *keyless · souverain · zéro réseau caché*. Ce test l'**applique en
//! continu** : toute régression (un crate réseau/TLS ajouté, une clé/auth introduite dans le
//! SDK) **casse la CI**. Le garde-fou n'est pas une intention — il est exécutable.

use std::fs;
use std::path::Path;

fn collect_rs(dir: &Path, out: &mut String) {
    for entry in fs::read_dir(dir).expect("lecture du dossier src") {
        let p = entry.expect("entrée").path();
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push_str(&fs::read_to_string(&p).expect("lecture .rs"));
            out.push('\n');
        }
    }
}

/// G9/G10 : **aucune dépendance réseau/TLS** dans le SDK (il shell-out `curl` local).
#[test]
fn sdk_has_no_network_or_tls_dependency() {
    let cargo = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    for forbidden in ["reqwest", "native-tls", "openssl", "rustls", "hyper-tls", "ureq", "isahc"] {
        assert!(
            !cargo.contains(forbidden),
            "garde-fou G10 violé : dépendance réseau/TLS interdite dans efe-swarm-sdk : « {forbidden} »"
        );
    }
}

/// G1 : **aucun secret ni en-tête d'auth** dans les sources du SDK (keyless par construction).
#[test]
fn sdk_source_reads_no_key_and_sends_no_auth() {
    let mut src = String::new();
    collect_rs(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut src);
    for forbidden in ["_API_KEY", "api_key", "Bearer ", "std::env::var", "env::var("] {
        assert!(
            !src.contains(forbidden),
            "garde-fou G1 violé : motif clé/auth/env interdit dans le SDK : « {forbidden} »"
        );
    }
}

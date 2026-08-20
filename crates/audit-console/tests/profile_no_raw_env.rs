// SPDX-License-Identifier: UNLICENSED
//! **T1 (M0) — console** : *aucune lecture d'environnement hors du registre*.
//!
//! Même invariant que côté passerelle, même raison : sans lui, `profile::REGISTRY` ne serait
//! qu'une documentation parallèle. La console est un binaire de 52 lignes — la tentation d'y
//! glisser un `env::var` « juste pour cette fois » est d'autant plus grande.

use std::path::{Path, PathBuf};

/// Doit rester vide : chaque entrée est un interrupteur que le rapport ne décrit pas.
const EXEMPTIONS: &[&str] = &[];

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

#[test]
fn t1_console_aucune_lecture_denv_hors_du_registre() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&src, &mut files);
    assert!(!files.is_empty(), "aucun source sous {}", src.display());

    let mut fautes = Vec::new();
    for path in &files {
        let rel = path.strip_prefix(&src).unwrap_or(path).to_string_lossy().replace('\\', "/");
        if rel == "profile.rs" || EXEMPTIONS.contains(&rel.as_str()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let test_from =
            text.lines().position(|l| l.contains("#[cfg(test)]")).map_or(usize::MAX, |i| i + 1);
        for (i, line) in text.lines().enumerate() {
            if i + 1 > test_from {
                continue;
            }
            let code = line.split("//").next().unwrap_or(line);
            if code.contains("env::var") {
                fautes.push(format!("  {rel}:{}  {}", i + 1, line.trim()));
            }
        }
    }
    assert!(
        fautes.is_empty(),
        "\n{} lecture(s) hors registre — router par `profile::read::…` et déclarer dans \
         `profile::REGISTRY`.\n\n{}\n",
        fautes.len(),
        fautes.join("\n")
    );
}

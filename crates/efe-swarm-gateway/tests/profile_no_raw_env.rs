// SPDX-License-Identifier: UNLICENSED
//! **T1 (M0)** — *aucune lecture d'environnement hors du registre*.
//!
//! C'est l'invariant qui rend `profile::REGISTRY` réellement unique. Sans lui, le registre ne
//! serait qu'une **documentation parallèle** — c'est-à-dire exactement le défaut que M0 existe
//! pour supprimer : un interrupteur qu'on découvre en production.
//!
//! ## La règle
//!
//! Toute occurrence de `std::env::var` dans `src/` doit être :
//!   1. dans `src/profile.rs` — le lecteur unique ; ou
//!   2. après un `#[cfg(test)]` dans le fichier — du code de test, qui a le droit de poser son
//!      propre environnement (et ne sert jamais rien).
//!
//! ## L'heuristique, et pourquoi elle est honnête
//!
//! « Après un `#[cfg(test)]` » est une approximation textuelle : elle suppose la convention du
//! dépôt, où `#[cfg(test)] mod tests` termine le fichier. Elle est **conservatrice dans le mauvais
//! sens** — un `#[cfg(test)]` en milieu de fichier exonérerait du code de production qui le suit.
//! On l'accepte parce que l'alternative (analyser l'AST) coûterait plus que l'invariant ne vaut,
//! et parce que la violation serait visible en relecture. Si la convention change, ce test doit
//! être renforcé, pas assoupli.

use std::path::{Path, PathBuf};

/// Fichiers autorisés à lire l'environnement en production.
///
/// **Cette liste est la dette, rendue visible.** Elle doit rester vide : tout ce qui y figure est
/// un interrupteur que le rapport de profil ne décrit pas, donc que l'opérateur ne verra pas.
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
fn t1_aucune_lecture_denv_hors_du_registre() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&src, &mut files);
    assert!(!files.is_empty(), "aucun fichier source trouvé sous {}", src.display());

    let mut fautes: Vec<String> = Vec::new();

    for path in &files {
        let rel = path.strip_prefix(&src).unwrap_or(path).to_string_lossy().replace('\\', "/");
        if rel == "profile.rs" || EXEMPTIONS.contains(&rel.as_str()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else { continue };

        // Ligne du premier `#[cfg(test)]` : tout ce qui suit est du code de test.
        let test_from = text
            .lines()
            .position(|l| l.contains("#[cfg(test)]"))
            .map_or(usize::MAX, |i| i + 1);

        for (i, line) in text.lines().enumerate() {
            let n = i + 1;
            if n > test_from {
                continue; // code de test
            }
            // On cherche l'appel, pas la mention : une ligne de commentaire qui *parle* de
            // `env::var` ne viole rien.
            let code = line.split("//").next().unwrap_or(line);
            if code.contains("env::var") {
                fautes.push(format!("  {rel}:{n}  {}", line.trim()));
            }
        }
    }

    assert!(
        fautes.is_empty(),
        "\n{} lecture(s) d'environnement hors du registre — chacune est un interrupteur que le \
         rapport de profil ne décrit pas, donc que l'opérateur découvrira en production.\n\
         Router la lecture par `crate::profile::read::…`, et déclarer l'interrupteur dans \
         `profile::REGISTRY`.\n\n{}\n",
        fautes.len(),
        fautes.join("\n")
    );
}

/// Le pendant du précédent : la liste d'exemptions doit rester **vide**. Si elle grossit, M0 s'est
/// défait sans que personne ne le voie.
/// La liste étant un `const` vide, `is_empty()` vaut `true` à la compilation —
/// et **c'est l'assertion**. Ce garde ne peut être constant que TANT QU'IL EST
/// SATISFAIT : la première entrée ajoutée le rend franchissable, et il tombe.
/// Même situation, même raison que `scella-authority/tests/un_seul_analyseur.rs`.
#[test]
fn t1b_la_liste_dexemptions_reste_vide() {
    #[allow(clippy::const_is_empty)] // la constance EST l'assertion (cf. supra)
    let vide = EXEMPTIONS.is_empty();
    assert!(
        vide,
        "EXEMPTIONS contient {} entrée(s) : {:?}. Chacune est une dette — un interrupteur non \
         décrit par le rapport de profil.",
        EXEMPTIONS.len(),
        EXEMPTIONS
    );
}

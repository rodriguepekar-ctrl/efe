// SPDX-License-Identifier: UNLICENSED
//! # M2 — aucune route ne lit le store en direct : l'absence a un seul point de décision.
//!
//! ## Le trou que ce test ferme
//!
//! Le premier passage de M2 avait corrigé `verify/:id` — un run servi mais non persisté y cessait
//! d'être un `404` muet pour devenir un `410` qui dit ce qui s'est passé. L'audit du module a
//! trouvé que **six autres routes** faisaient encore exactement ce qui venait d'être condamné :
//! `bundle`, `records`, `n17`, `replay` ×2, `llm/audit`, plus la branche finale de `get_run`.
//!
//! Toutes tenaient le même raisonnement, écrit sept fois. C'est la forme même du défaut que ce
//! dépôt combat depuis M0 : une règle appliquée à la main est une règle qui sera oubliée une fois.
//!
//! ## Pourquoi le motif est `store.get(`, et pas `NotFound`
//!
//! Interdire `GatewayError::NotFound` obligerait à exempter `events.rs`, dont le `404` est
//! parfaitement légitime : il cherche un **bus vivant**, pas un run persisté, et il n'existe pas
//! de brèche pour un bus. Une liste d'exemptions est une vigilance déguisée en mécanisme.
//!
//! La règle positive n'en a pas besoin : **une route ne lit jamais le store en direct**. Elle
//! passe par `SwarmGatewayState::charger_run`, seul endroit qui sait consulter le journal des
//! brèches avant de conclure. `events.rs` interroge `dashboards`, pas `store` — il sort donc du
//! filet par construction, sans exception à écrire.

use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn aucune_route_ne_lit_le_store_en_direct() {
    let racine = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes");
    let mut fichiers = Vec::new();
    sources(&racine, &mut fichiers);
    assert!(
        fichiers.len() >= 8,
        "seulement {} routes trouvées — le test se croirait vert à tort",
        fichiers.len()
    );

    let mut fautes = Vec::new();
    for f in &fichiers {
        let rel = f.strip_prefix(&racine).unwrap_or(f).display().to_string();
        let Ok(texte) = std::fs::read_to_string(f) else { continue };
        for (n, ligne) in texte.lines().enumerate() {
            let nu = ligne.trim_start();
            // Les mentions en commentaire expliquent la règle ; elles ne la violent pas.
            if nu.starts_with("//") {
                continue;
            }
            if ligne.contains("store.get(") {
                fautes.push(format!("  {rel}:{}\n      {}", n + 1, ligne.trim()));
            }
        }
    }

    // Une route qui REGARDE sans conclure (`peek_run`) doit conclure ailleurs, et le seul
    // endroit qui sache le faire est `absence_expliquee`. Sans cette exigence, le `404` muet
    // reviendrait par la porte du « peek », et le filet aurait un trou de la forme exacte de
    // celui qu'il vient de fermer.
    for f in &fichiers {
        let rel = f.strip_prefix(&racine).unwrap_or(f).display().to_string();
        let Ok(texte) = std::fs::read_to_string(f) else { continue };
        if texte.contains("peek_run(") && !texte.contains("absence_expliquee") {
            fautes.push(format!(
                "  {rel}\n      emploie `peek_run` sans conclure par `absence_expliquee`"
            ));
        }
    }

    assert!(
        fautes.is_empty(),
        "\nUne route lit le store en direct :\n{}\n\n\
         Utiliser `state.charger_run(id, tenant)`. Un `store.get(...).ok_or(NotFound)` répond\n\
         « cet identifiant n'existe pas », ce qui est FAUX pour un run servi mais non persisté :\n\
         cela EFFACE un fait au lieu de le documenter, exactement ce que la chaîne de provenance\n\
         existe pour empêcher. `charger_run` consulte le journal des brèches et rend 410.\n",
        fautes.join("\n")
    );
}

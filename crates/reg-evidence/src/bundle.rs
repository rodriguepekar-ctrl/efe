// SPDX-License-Identifier: UNLICENSED
//! **La composition avec le bundle forensic (S6 × S4).** L'artefact réglementaire **voyage** dans le
//! bundle de S4, et il y est **re-dérivable** — d'où « zéro confiance neuve ».
//!
//! Le sous-arbre `reg/` ajouté au bundle :
//! - `reg/<cadre>.md` — l'artefact **rendu** (la VUE) ;
//! - `reg/mapping.<version>.json` — le [`MappingSet`] **versionné** (la lecture déclarée) qui a produit
//!   l'artefact, pour que le tiers **re-dérive** (sans lui, il lui manquerait la lecture) ;
//! - `reg/REDERIVE.md` — le **mode d'emploi** de la re-dérivation (le geste trust-light, honnêtement cadré).
//!
//! [`reg_subtree`] est **pure** (filename → contenu) — testable et déterministe ; [`write_reg_subtree`]
//! n'est qu'une commodité d'I/O. La **clé n'apparaît jamais** (l'artefact n'en contient pas — souveraineté,
//! cohérent avec S4 : la clé re-prouve mais ne s'écrit pas dans un bundle).

use crate::artifact::RegulatoryArtifact;
use crate::mapping::MappingSet;
use crate::render::render_md;
use std::path::Path;

/// Les fichiers du sous-arbre `reg/` à déposer dans un bundle forensic. **Pur** (filename relatif →
/// contenu) — aucune I/O. Déterministe : mêmes (artefact, mapping) → mêmes octets.
pub fn reg_subtree(artifact: &RegulatoryArtifact, mapping: &MappingSet) -> Vec<(String, String)> {
    let fw = artifact.framework.slug();
    vec![
        (format!("reg/{fw}.md"), render_md(artifact)),
        (
            format!("reg/mapping.{}.json", mapping.version),
            serde_json::to_string_pretty(mapping).expect("MappingSet sérialisable") + "\n",
        ),
        ("reg/REDERIVE.md".to_string(), rederive_md(artifact, mapping)),
    ]
}

/// Le mode d'emploi de la re-dérivation — le geste trust-light, cadré honnêtement.
fn rederive_md(artifact: &RegulatoryArtifact, mapping: &MappingSet) -> String {
    let fw = artifact.framework.slug();
    format!(
        "# Re-dériver cet artefact VOUS-MÊME (trust-light — zéro confiance neuve)\n\
\n\
Cet artefact réglementaire (`reg/{fw}.md`) n'est **pas** une assertion qu'il faut nous croire : c'est une\n\
**fonction déterministe** de deux entrées que vous détenez déjà dans ce bundle. Vous le **re-calculez**.\n\
\n\
1. **Re-prouvez** d'abord les records (`records.sealed.jsonl`) avec la **clé tenant** (obtenue auprès du\n\
   responsable de traitement — elle n'est **pas** dans ce bundle), comme décrit dans le `README` du bundle.\n\
2. **Re-dérivez** : appliquez la lecture `reg/mapping.{}.json` (version `{}`, `set_hash` ci-dessous) aux\n\
   records re-prouvés, en sélectionnant pour chaque exigence les nœuds que son sélecteur désigne.\n\
3. **Comparez** : le résultat doit **égaler** `reg/{fw}.md`, octet pour octet.\n\
\n\
- **manifest-root** attendu : `{}`\n\
- **mapping `set_hash`** attendu : `{}`\n\
\n\
## Ce que cet artefact EST, et ce qu'il N'EST PAS\n\
\n\
- Il **fournit et mappe** l'évidence que l'exigence réclame, chaque ligne citant un `node_id` scellé que\n\
  vous **recoupez** directement (ne faites jamais confiance au texte — recoupez-le).\n\
- Il **n'est pas** un prononcé de conformité : **outillé ≠ conforme**. La conformité est une conclusion\n\
  juridique (classification du système, conseil du client, obligations organisationnelles que le DAG ne\n\
  produit pas — voir les exigences marquées **HORS-PÉRIMÈTRE-DAG**, qui sont **déclarées, jamais tues**).\n\
- La **lecture** (le mapping) est une **revendication versionnée** de Kovant, datée — pas une vérité.\n\
  Elle voyage ici pour que vous puissiez la **lire, la contester, et la re-jouer**.\n",
        mapping.version, mapping.version, artifact.manifest_root, artifact.mapping_set_hash
    )
}

/// Écrit le sous-arbre `reg/` sous `dir` (crée `dir/reg/`). Commodité d'I/O au-dessus de [`reg_subtree`].
/// La **clé n'est jamais écrite** (l'artefact n'en contient pas).
pub fn write_reg_subtree(dir: &Path, artifact: &RegulatoryArtifact, mapping: &MappingSet) -> std::io::Result<()> {
    for (rel, content) in reg_subtree(artifact, mapping) {
        let path = dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::derive;
    use crate::templates::eu_ai_act_2026_06;
    use crate::CaseScope;
    use crate::Framework;
    use provenance_dag::{Node, SealedDag, TenantKey};

    fn key() -> TenantKey {
        TenantKey::from_bytes([13u8; 32])
    }

    fn artifact_and_mapping() -> (RegulatoryArtifact, MappingSet) {
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "SUP-2024::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        let m = eu_ai_act_2026_06();
        let a = derive(d.records(), &k, &m, Framework::EuAiAct, &CaseScope::Case("SUP-2024".into())).unwrap();
        (a, m)
    }

    #[test]
    fn reg_subtree_is_pure_and_carries_view_mapping_and_rederive() {
        let (a, m) = artifact_and_mapping();
        let files = reg_subtree(&a, &m);
        assert_eq!(reg_subtree(&a, &m), files, "pur et déterministe");
        // Les trois fichiers attendus.
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"reg/eu_ai_act.md"), "la VUE rendue voyage");
        assert!(names.contains(&"reg/mapping.2026-06.json"), "le mapping versionné voyage (re-dérivable)");
        assert!(names.contains(&"reg/REDERIVE.md"), "le mode d'emploi voyage");
        // La clé n'apparaît dans AUCUN fichier (souveraineté).
        let key_hex = "0d".repeat(32); // [13u8;32] en hex
        for (_, content) in &files {
            assert!(!content.contains(&key_hex), "aucune clé tenant écrite dans le bundle");
        }
        // Le mapping embarqué re-désérialise et reproduit le même hash (re-dérivable).
        let mapping_json = &files.iter().find(|(n, _)| n == "reg/mapping.2026-06.json").unwrap().1;
        let back: MappingSet = serde_json::from_str(mapping_json).unwrap();
        assert_eq!(back.set_hash(), m.set_hash(), "le mapping embarqué reproduit le hash");
        // REDERIVE.md énonce le cadrage honnête.
        let rederive = &files.iter().find(|(n, _)| n == "reg/REDERIVE.md").unwrap().1;
        assert!(rederive.contains("outillé ≠ conforme"));
        assert!(rederive.contains(&a.manifest_root));
        assert!(rederive.contains(&m.set_hash()));
    }

    #[test]
    fn write_reg_subtree_emits_files() {
        let (a, m) = artifact_and_mapping();
        let dir = std::env::temp_dir().join(format!("reg-evidence-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_reg_subtree(&dir, &a, &m).unwrap();
        assert!(dir.join("reg/eu_ai_act.md").exists());
        assert!(dir.join("reg/mapping.2026-06.json").exists());
        assert!(dir.join("reg/REDERIVE.md").exists());
        // Le contenu écrit égale le rendu pur (write n'est qu'une I/O au-dessus de reg_subtree).
        let md = std::fs::read_to_string(dir.join("reg/eu_ai_act.md")).unwrap();
        assert_eq!(md, render_md(&a));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

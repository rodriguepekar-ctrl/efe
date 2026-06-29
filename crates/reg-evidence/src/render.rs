// SPDX-License-Identifier: UNLICENSED
//! **Le rendu** — la VUE lisible de l'artefact (Markdown & HTML). Déterministe (mêmes données → mêmes
//! octets), donc **re-dérivable** par un tiers (`I-S6-VUE-REDÉRIVABLE`).
//!
//! Le rendu **porte** l'honnêteté : il énonce `outillé ≠ conforme` (la conclusion juridique n'est pas la
//! nôtre) ET `intègre ≠ correct` (les décisions citées sont re-prouvées, pas garanties justes), montre la
//! couverture en **trois classes**, et fait **citer chaque ligne d'évidence par son `node_id`** scellé —
//! « recoupez-le, ne faites pas confiance à ce texte ». Aucun verdict de conformité n'apparaît.

use crate::artifact::RegulatoryArtifact;
use crate::framework::Coverage;

/// La phrase cardinale, réutilisée par les deux rendus (constante — pas du texte libre).
const OUTILLE_NEQ_CONFORME: &str = "Outillé ≠ conforme : ce document FOURNIT et MAPPE l'évidence que l'exigence réclame ; il n'est PAS un prononcé de conformité (une conclusion juridique qui dépend de la classification du système, du conseil du client, et d'obligations organisationnelles que le DAG ne produit pas). L'évidence citée est intègre (re-prouvée) mais pas nécessairement correcte : la justesse des décisions est une autre couche.";

/// Rend l'artefact en **Markdown** — la VUE lisible, citant les nœuds, portant l'honnêteté.
pub fn render_md(a: &RegulatoryArtifact) -> String {
    let mut s = String::new();
    s.push_str(&format!("# Évidence réglementaire — {}\n\n", a.framework.label()));
    s.push_str(&format!("- **Portée** : {}\n", a.case_scope.label()));
    s.push_str(&format!("- **Lecture (mapping)** : version `{}` · `set_hash = {}`\n", a.mapping_version, a.mapping_set_hash));
    s.push_str(&format!("- **manifest-root** (DAG re-prouvé) : `{}`\n", a.manifest_root));
    s.push_str(&format!("- **Records dans la portée** : {}\n\n", a.n_records_in_scope));

    // L'avertissement cardinal.
    s.push_str("> **");
    s.push_str(OUTILLE_NEQ_CONFORME);
    s.push_str("**\n\n");

    // La couverture — les trois couleurs, montrées côte à côte.
    s.push_str("## Couverture (honnête — les trois classes)\n\n");
    s.push_str(&format!("- **COUVERT** : {}\n", a.coverage_summary.covered));
    s.push_str(&format!("- **PARTIEL** : {}\n", a.coverage_summary.partial));
    s.push_str(&format!("- **HORS-PÉRIMÈTRE-DAG** : {}  _(trous déclarés — jamais tus)_\n\n", a.coverage_summary.out_of_dag_scope));

    // Par exigence.
    s.push_str("## Par exigence\n\n");
    for f in &a.findings {
        s.push_str(&format!("### {} — [{}]\n\n", f.citation, f.coverage.label()));
        s.push_str(&format!("_{}_\n\n", f.requirement_text));
        if !f.furnishes.is_empty() {
            s.push_str(&format!("- **Ce que le DAG fournit** : {}\n", f.furnishes));
        }
        s.push_str(&format!("- **Ce qu'il NE fournit PAS** : {}\n", f.does_not_furnish));
        if f.evidence.is_empty() {
            if f.coverage == Coverage::OutOfDagScope {
                s.push_str("- **Évidence** : _(aucune — hors périmètre du DAG, déclaré)_\n");
            } else {
                s.push_str("- **Évidence** : _(aucun nœud de la portée ne satisfait ce sélecteur)_\n");
            }
        } else {
            s.push_str(&format!("- **Évidence ({} nœud(s) scellé(s) — recoupez-les)** :\n", f.evidence.len()));
            for id in &f.evidence {
                s.push_str(&format!("  - `{id}`\n"));
            }
        }
        s.push('\n');
    }

    // Pied honnête.
    s.push_str("---\n\n");
    s.push_str("Chaque `node_id` cité existe dans les records re-prouvés — **recoupez-le**, ne faites pas confiance à ce texte. ");
    s.push_str("Cet artefact est une **VUE re-dérivable** : avec les records scellés, la clé tenant, et la version de mapping ci-dessus, vous régénérez ce document à l'identique (trust-light — aucune confiance neuve). ");
    s.push_str("**outillé ≠ conforme.**\n");
    s
}

/// Rend l'artefact en **HTML** minimal (pour la console S2 / une pièce du bundle). Déterministe et
/// échappé. Volontairement sobre (pas de CSS lourd) : c'est une VUE, pas la preuve.
pub fn render_html(a: &RegulatoryArtifact) -> String {
    let mut s = String::new();
    s.push_str("<!DOCTYPE html>\n<html lang=\"fr\"><head><meta charset=\"utf-8\"><title>");
    s.push_str(&esc(&format!("Évidence réglementaire — {}", a.framework.label())));
    s.push_str("</title></head><body>\n");
    s.push_str(&format!("<h1>Évidence réglementaire — {}</h1>\n", esc(a.framework.label())));
    s.push_str("<ul>\n");
    s.push_str(&format!("<li><b>Portée</b> : {}</li>\n", esc(&a.case_scope.label())));
    s.push_str(&format!("<li><b>Lecture (mapping)</b> : version <code>{}</code> · <code>set_hash = {}</code></li>\n", esc(&a.mapping_version), esc(&a.mapping_set_hash)));
    s.push_str(&format!("<li><b>manifest-root</b> : <code>{}</code></li>\n", esc(&a.manifest_root)));
    s.push_str(&format!("<li><b>Records dans la portée</b> : {}</li>\n", a.n_records_in_scope));
    s.push_str("</ul>\n");
    s.push_str(&format!("<blockquote><b>{}</b></blockquote>\n", esc(OUTILLE_NEQ_CONFORME)));
    s.push_str("<h2>Couverture (les trois classes)</h2>\n<ul>\n");
    s.push_str(&format!("<li><b>COUVERT</b> : {}</li>\n", a.coverage_summary.covered));
    s.push_str(&format!("<li><b>PARTIEL</b> : {}</li>\n", a.coverage_summary.partial));
    s.push_str(&format!("<li><b>HORS-PÉRIMÈTRE-DAG</b> : {} (trous déclarés)</li>\n", a.coverage_summary.out_of_dag_scope));
    s.push_str("</ul>\n<h2>Par exigence</h2>\n");
    for f in &a.findings {
        s.push_str(&format!("<h3>{} — [{}]</h3>\n", esc(&f.citation), esc(f.coverage.label())));
        s.push_str(&format!("<p><i>{}</i></p>\n<ul>\n", esc(&f.requirement_text)));
        if !f.furnishes.is_empty() {
            s.push_str(&format!("<li><b>Fournit</b> : {}</li>\n", esc(&f.furnishes)));
        }
        s.push_str(&format!("<li><b>NE fournit PAS</b> : {}</li>\n", esc(&f.does_not_furnish)));
        if f.evidence.is_empty() {
            s.push_str("<li><b>Évidence</b> : <i>(aucune)</i></li>\n");
        } else {
            s.push_str("<li><b>Évidence (node_id scellés — recoupez-les)</b> :<ul>\n");
            for id in &f.evidence {
                s.push_str(&format!("<li><code>{}</code></li>\n", esc(id)));
            }
            s.push_str("</ul></li>\n");
        }
        s.push_str("</ul>\n");
    }
    s.push_str("<hr>\n<p>Chaque <code>node_id</code> cité existe dans les records re-prouvés — recoupez-le. ");
    s.push_str("VUE re-dérivable (trust-light). <b>outillé ≠ conforme.</b></p>\n");
    s.push_str("</body></html>\n");
    s
}

/// Échappement HTML minimal (zéro dépendance).
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{CaseScope, CoverageSummary, HonestyEnvelope, RegulatoryArtifact, RequirementFinding};
    use crate::framework::Framework;

    fn artifact() -> RegulatoryArtifact {
        RegulatoryArtifact {
            framework: Framework::EuAiAct,
            case_scope: CaseScope::Case("SUP-2024".into()),
            mapping_version: "2026-06".into(),
            mapping_set_hash: "ab".repeat(32),
            manifest_root: "cd".repeat(32),
            n_records_in_scope: 6,
            findings: vec![
                RequirementFinding {
                    citation: "art. 12".into(),
                    requirement_text: "tenue de registres".into(),
                    coverage: Coverage::Covered,
                    evidence: vec!["SUP-2024::sanction".into(), "SUP-2024::rev".into()],
                    furnishes: "le DAG scellé EST le registre".into(),
                    does_not_furnish: "la rétention reste au déployeur".into(),
                },
                RequirementFinding {
                    citation: "art. 9".into(),
                    requirement_text: "système de gestion des risques".into(),
                    coverage: Coverage::OutOfDagScope,
                    evidence: vec![],
                    furnishes: String::new(),
                    does_not_furnish: "artefact organisationnel hors DAG".into(),
                },
            ],
            coverage_summary: CoverageSummary { covered: 1, partial: 0, out_of_dag_scope: 1 },
            honesty: HonestyEnvelope::default(),
        }
    }

    #[test]
    fn md_is_deterministic_carries_honesty_and_cites_nodes() {
        let a = artifact();
        let md = render_md(&a);
        assert_eq!(md, render_md(&a), "rendu déterministe (re-dérivable)");
        // I-S6-PAS-DE-VERDICT + I-S6-INTÈGRE-PROPAGÉ : l'avertissement est présent, aucun « compliant ».
        assert!(md.to_lowercase().contains("outillé ≠ conforme"), "outillé ≠ conforme énoncé");
        assert!(md.contains("pas nécessairement correcte"), "intègre ≠ correct propagé");
        assert!(!md.to_lowercase().contains("compliant"), "aucun verdict de conformité dans le récit");
        // Traçabilité visible : chaque nœud est cité.
        assert!(md.contains("`SUP-2024::sanction`"));
        assert!(md.contains("`SUP-2024::rev`"));
        // La couverture montre ses trois classes (dont le trou).
        assert!(md.contains("HORS-PÉRIMÈTRE-DAG"));
        assert!(md.contains("art. 9 — [HORS-PÉRIMÈTRE-DAG]"));
        // Le root et la version de mapping sont cités (datables, re-dérivables).
        assert!(md.contains(&a.manifest_root));
        assert!(md.contains("version `2026-06`"));
    }

    #[test]
    fn html_is_escaped_and_deterministic() {
        // Un libellé hostile doit être échappé (pas d'injection).
        let mut a = artifact();
        a.findings[0].furnishes = "x <script>alert(1)</script> & \"".into();
        let html = render_html(&a);
        assert_eq!(html, render_html(&a), "rendu HTML déterministe");
        assert!(!html.contains("<script>alert"), "le HTML hostile est échappé");
        assert!(html.contains("&lt;script&gt;"), "échappement effectif");
        assert!(html.to_lowercase().contains("outillé ≠ conforme"));
        assert!(!html.to_lowercase().contains("compliant"));
    }
}

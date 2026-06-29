// SPDX-License-Identifier: UNLICENSED
//! **Le récit** — un rendu Markdown **pur** d'une [`DecisionView`] + son enveloppe d'intégrité. C'est le
//! « pourquoi lisible » de la console et le `report` du bundle d'export (le contenu remis à S4). Pur (pas
//! d'horloge) : l'horodatage de fraîcheur vient de l'enveloppe (`as_of`), déjà fixé par le service.
//!
//! Discipline « intègre ≠ correct » : la ligne d'intégrité dit « authentique & non-altéré » et **n'emploie
//! jamais** le token « correct » (elle parle de « justesse » qu'elle nie) — le test-grep reste littéral.

use crate::envelope::IntegrityEnvelope;
use crate::project::{DecisionStatus, DecisionView};

/// Rend une décision en Markdown (le « récit » lisible par un non-technicien).
pub fn render_case(view: &DecisionView, env: &IntegrityEnvelope) -> String {
    let mut s = String::new();
    let status = match view.status {
        DecisionStatus::Compliant => "COMPLIANT",
        DecisionStatus::Fail => "FAIL",
        DecisionStatus::Exception => "EXCEPTION",
        DecisionStatus::Pending => "PENDING",
    };
    s.push_str(&format!("# Dossier {} — {status}\n\n", view.case_id));
    if let (Some(label), Some(node)) = (&view.verdict_label, &view.verdict_node) {
        s.push_str(&format!("**Verdict** : {label}  ·  _(nœud scellé `{node}`)_\n\n"));
    }
    if view.verdict_conflict {
        s.push_str("> ⚠ **Verdicts conflictuels** : plusieurs nœuds d'agrégation dans ce dossier (statut forcé à FAIL).\n\n");
    }

    if !view.checks.is_empty() {
        s.push_str("## Contrôles\n\n");
        for c in &view.checks {
            s.push_str(&format!(
                "- **{}** : {}  ·  agent `{}` ({} v{})  ·  entrées minimisées (hachées)  ·  _[`{}`]_\n",
                c.requirement, c.outcome, c.agent_role, c.model, c.version, c.backed_by_node
            ));
            for e in &c.evidence {
                s.push_str(&format!(
                    "    - évidence : `{}` (outil `{}`, content_hash `{}`)  ·  _[`{}`]_\n",
                    e.retrieved_ref, e.tool, short(&e.content_hash), e.backed_by_node
                ));
            }
        }
        s.push('\n');
    }

    if !view.flow.is_empty() {
        s.push_str("## Flux inter-agents\n\n");
        for f in &view.flow {
            s.push_str(&format!("- `{}` → `{}`  ·  {}  ·  _[`{}`]_\n", f.from, f.to, f.artifact_type, f.backed_by_node));
        }
        s.push('\n');
    }

    if !view.oversight.is_empty() {
        s.push_str("## Supervision humaine (EU AI Act art. 14)\n\n");
        for o in &view.oversight {
            s.push_str(&format!(
                "- {} **{:?}** — « {} »  ·  _[`{}`]_\n",
                o.human_id, o.action, o.justification, o.backed_by_node
            ));
        }
        s.push('\n');
    }

    s.push_str("---\n\n");
    s.push_str(&integrity_line(env));
    s
}

/// La ligne d'intégrité — sans jamais le token « correct ».
fn integrity_line(env: &IntegrityEnvelope) -> String {
    if env.ok() {
        format!(
            "**Intégrité** : ✓ VÉRIFIÉ — record authentique et non-altéré ({} records, re-prouvé froid à t={}).  \n_Atteste ce qui s'est passé — pas la justesse de la décision, ni une reproductibilité bit-à-bit._\n",
            env.verdict.n_records, env.verdict.as_of
        )
    } else {
        format!(
            "**Intégrité** : ✗ ÉCHEC — altération/réordonnancement détecté{}.  \n_Motif : {}_\n",
            env.verdict.broken_at.map(|i| format!(" au record {i}")).unwrap_or_default(),
            env.verdict.reason.clone().unwrap_or_default()
        )
    }
}

/// Rend un bundle multi-décisions (le récit d'export).
pub fn render_export(views: &[DecisionView], env: &IntegrityEnvelope) -> String {
    let mut s = String::new();
    s.push_str(&format!("# Bundle forensic — {} décision(s)\n\n", views.len()));
    s.push_str(&integrity_line(env));
    s.push_str("\n---\n\n");
    for v in views {
        s.push_str(&render_case(v, env));
        s.push_str("\n\n");
    }
    s
}

fn short(h: &str) -> String {
    // par caractères (jamais d'index d'octet → pas de panique sur frontière non-ASCII, même si les
    // content_hash sont hex).
    let head: String = h.chars().take(12).collect();
    if head.len() < h.len() {
        format!("{head}…")
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::IntegrityEnvelope;
    use crate::project::{project, ProjectionConfig};
    use provenance_dag::{reprove_dag, Node, Record, SealedDag, TenantKey};

    fn case_and_env() -> (DecisionView, IntegrityEnvelope) {
        let k = TenantKey::from_bytes([4u8; 32]);
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "C::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        d.append(&k, Node::AgentAct { node_id: "C::aggregate".into(), agent_role: "verdict_aggregate".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "COMPLIANT".into(), evidence_refs: vec![], timestamp: 2 }).unwrap();
        let recs: Vec<&Record> = d.records().iter().collect();
        let view = project(&recs, "C", &ProjectionConfig::default());
        let v = reprove_dag(d.records(), &k);
        (view, IntegrityEnvelope::from_verdict(&v, 42))
    }

    #[test]
    fn renders_markdown_without_correct_token() {
        let (view, env) = case_and_env();
        let md = render_case(&view, &env);
        assert!(md.contains("# Dossier C — COMPLIANT"));
        assert!(md.contains("sanction_check"));
        assert!(md.contains("C::sanction"), "le nœud adossé est cité");
        assert!(md.contains("VÉRIFIÉ"));
        assert!(!md.to_lowercase().contains("correct"), "jamais le token « correct » : {md}");
    }

    #[test]
    fn renders_broken_integrity_loudly() {
        // Un verdict cassé → la ligne d'intégrité CRIE l'échec (fail-loud), sans jamais le token « correct ».
        let k = TenantKey::from_bytes([4u8; 32]);
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "C::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        let recs: Vec<&Record> = d.records().iter().collect();
        let view = project(&recs, "C", &ProjectionConfig::default());
        // une re-preuve sur un JSONL altéré → verdict cassé
        let tampered = d.to_jsonl().replace("PASS", "FAIL");
        let trecs: Vec<Record> = SealedDag::records_from_jsonl(&tampered).unwrap();
        let v = reprove_dag(&trecs, &k);
        let env = IntegrityEnvelope::from_verdict(&v, 7);
        let md = render_case(&view, &env);
        assert!(md.contains("ÉCHEC"), "l'échec d'intégrité est rendu fort : {md}");
        assert!(!md.to_lowercase().contains("correct"));
    }
}

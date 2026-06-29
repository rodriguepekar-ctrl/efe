// SPDX-License-Identifier: UNLICENSED
//! **L'interface, rendue serveur en HTML** — des fonctions de rendu **pures** (mêmes données → mêmes
//! octets), sans crypto, sans clé. Elles ne font que **rendre** ce que S2 a calculé : le badge rend
//! l'`IntegrityEnvelope`, la vue décision rend le `DecisionView` (chaque élément citant son
//! `backed_by_node`). Trois honnêtetés sont **portées par le rendu** :
//!
//! - **Montré vérifié ≠ prouvé sans confiance** : un bandeau + le chemin d'export sur chaque vue.
//! - **Fraîcheur honnête** : le badge affiche `as_of` (la re-preuve froide qui l'étaye) + « vérifier maintenant ».
//! - **Fail-loud** : une enveloppe `ok:false` rend la vue **rouge** avec le record cassé localisé — jamais un faux vert.
//!
//! Le RBAC est **rendu** (boutons d'action/export cachés selon le rôle, valeurs déjà **rédigées** par S2),
//! mais ce masquage est **cosmétique** : c'est S2 qui refuse (cf. [`crate::handler`]).

use audit_api::{CaseResponse, CaseSummary, DecisionStatus, DecisionView, IntegrityEnvelope, ListResponse, Role};
use provenance_dag::HumanAction;

/// Échappement HTML (zéro dépendance) — toute donnée dynamique passe par ici.
pub fn esc(s: &str) -> String {
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

const STYLE: &str = "body{font-family:-apple-system,Segoe UI,Inter,Arial,sans-serif;color:#0f172a;max-width:1000px;margin:0 auto;padding:1rem 1.2rem;line-height:1.5}\
a{color:#4f46e5;text-decoration:none}a:hover{text-decoration:underline}\
header{border-bottom:2px solid #0d9488;padding-bottom:.5rem;margin-bottom:1rem}\
.nav a{margin-right:1rem;font-weight:600}\
.banner{background:#eef2ff;border-left:4px solid #4f46e5;padding:.5rem .8rem;border-radius:0 6px 6px 0;font-size:.85rem;margin:.6rem 0}\
.badge{border-radius:8px;padding:.7rem 1rem;margin:.8rem 0;border:1px solid}\
.badge.ok{background:#f0fdf4;border-color:#16a34a}.badge.bad{background:#fef2f2;border-color:#dc2626}\
.badge h3{margin:.1rem 0}.badge .ok-t{color:#166534}.badge .bad-t{color:#991b1b}\
.fresh{color:#475569;font-size:.82rem}.root{font-family:ui-monospace,Menlo,monospace;font-size:.72rem;color:#64748b}\
.chip{display:inline-block;font-size:.72rem;font-weight:700;padding:.1em .55em;border-radius:20px}\
.c-ok{background:#dcfce7;color:#166534}.c-fail{background:#fee2e2;color:#991b1b}.c-exc{background:#fef3c7;color:#92400e}.c-pend{background:#e0e7ff;color:#3730a3}\
table{border-collapse:collapse;width:100%;margin:.6rem 0;font-size:.86rem}th,td{border:1px solid #e2e8f0;padding:.35rem .55rem;text-align:left;vertical-align:top}th{background:#eef2f1}\
.node{font-family:ui-monospace,Menlo,monospace;font-size:.74rem;color:#64748b}\
.btn{display:inline-block;background:#0d9488;color:#fff;border:none;border-radius:6px;padding:.4rem .8rem;font-size:.85rem;cursor:pointer;font-weight:600}\
.btn.secondary{background:#475569}.btn.export{background:#d97706}\
form.panel{background:#f6fbfa;border:1px solid #e2e8f0;border-radius:8px;padding:.7rem .9rem;margin:.8rem 0}\
select,textarea{font:inherit;padding:.3rem;border:1px solid #cbd5e1;border-radius:5px;width:100%;box-sizing:border-box;margin:.25rem 0}\
.muted{color:#64748b;font-size:.82rem}footer{margin-top:1.5rem;border-top:1px solid #e2e8f0;padding-top:.6rem;font-size:.78rem;color:#64748b}";

/// L'avertissement cardinal de S3, porté sur chaque page (montré ≠ prouvé).
const HONESTY_BANNER: &str = "<div class=\"banner\"><b>Montré vérifié ≠ prouvé sans confiance.</b> Ce que cet écran affiche est rendu par le \
cœur d'audit (confiance opérationnelle, vous êtes dans le tenant) — ce n'est pas une preuve opposable. Pour cela, <b>exportez</b> le \
dossier : un tiers le re-prouve <b>sans nous</b>. « Intègre » n'est jamais « correct » (la justesse est une autre couche).</div>";

/// Le chrome d'une page (nav, bannière d'honnêteté, pied). `inner` est du HTML déjà échappé.
pub fn page(title: &str, tenant: &str, role: Role, inner: &str) -> String {
    let role_label = role_label(role);
    format!(
        "<!DOCTYPE html><html lang=\"fr\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{t} — console de conformité</title><style>{STYLE}</style></head><body>\
<header><div class=\"nav\"><a href=\"/\">Dossiers</a><a href=\"/exceptions\">File d'exceptions</a>\
<span class=\"muted\" style=\"float:right\">tenant <b>{tenant}</b> · {role_label}</span></div></header>\
{HONESTY_BANNER}\
<h1>{t}</h1>{inner}\
<footer>Console opérationnelle (S3) — une <b>VUE</b> sur le record scellé. La clé tenant ne quitte jamais le backend ; \
rien ne sort de la frontière. Pour une preuve <b>sans confiance</b>, exportez le dossier.</footer>\
</body></html>",
        t = esc(title),
        tenant = esc(tenant),
    )
}

fn role_label(role: Role) -> &'static str {
    match role {
        Role::Viewer => "lecteur (données rédigées)",
        Role::ComplianceOfficer => "agent de conformité",
        Role::Auditor => "auditeur (atteste, n'agit pas)",
        Role::Admin => "administration",
    }
}

/// Le **badge d'intégrité** — vert + fraîcheur, ou **rouge fail-loud** avec le record cassé localisé.
pub fn badge(env: &IntegrityEnvelope) -> String {
    let v = &env.verdict;
    if v.ok {
        let root = env.manifest_root.as_deref().unwrap_or("");
        format!(
            "<div class=\"badge ok\"><h3 class=\"ok-t\">🔒 Vérifié — record authentique &amp; non-altéré</h3>\
<div class=\"fresh\">à jour de la re-preuve froide · as_of = {as_of} · {n} records</div>\
<div class=\"root\">manifest-root {root}</div>\
<form method=\"post\" action=\"/reprove\" style=\"display:inline\"><button class=\"btn secondary\">vérifier maintenant</button></form></div>",
            as_of = v.as_of,
            n = v.n_records,
            root = esc(&short_root(root)),
        )
    } else {
        let broken = v.broken_at.map(|b| b.to_string()).unwrap_or_else(|| "?".into());
        let reason = v.reason.as_deref().unwrap_or("intégrité non vérifiée");
        format!(
            "<div class=\"badge bad\"><h3 class=\"bad-t\">⚠ Altération détectée — le record n'est PAS authentique</h3>\
<div class=\"fresh\">cassé au record {broken} · as_of = {as_of}</div>\
<div class=\"muted\">{reason}</div>\
<form method=\"post\" action=\"/reprove\" style=\"display:inline\"><button class=\"btn secondary\">re-vérifier</button></form></div>",
            as_of = v.as_of,
            reason = esc(reason),
        )
    }
}

fn short_root(root: &str) -> String {
    if root.len() > 20 {
        format!("{}…{}", &root[..10], &root[root.len() - 6..])
    } else {
        root.to_string()
    }
}

fn status_chip(s: DecisionStatus) -> &'static str {
    match s {
        DecisionStatus::Compliant => "<span class=\"chip c-ok\">CONFORME (record)</span>",
        DecisionStatus::Fail => "<span class=\"chip c-fail\">ÉCHEC</span>",
        DecisionStatus::Exception => "<span class=\"chip c-exc\">EXCEPTION</span>",
        DecisionStatus::Pending => "<span class=\"chip c-pend\">EN ATTENTE</span>",
    }
}

fn action_label(a: HumanAction) -> &'static str {
    match a {
        HumanAction::Approve => "APPROUVER",
        HumanAction::Override => "OUTREPASSER",
        HumanAction::Escalate => "ESCALADER",
        HumanAction::Comment => "COMMENTER",
    }
}

/// Les **filtres actifs** d'une liste (valeurs brutes, telles que saisies) — pour pré-remplir le
/// formulaire ET préserver les filtres dans le lien « page suivante » (§7).
#[derive(Debug, Default, Clone)]
pub struct Filters {
    pub status: Option<String>,
    pub role: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}
impl Filters {
    /// La query-string des filtres actifs (URL-encodée), terminée par « &amp; » si non vide — pour que la
    /// pagination ne perde JAMAIS le filtre courant.
    fn query_prefix(&self) -> String {
        let mut parts = Vec::new();
        if let Some(s) = &self.status {
            parts.push(format!("status={}", url_enc(s)));
        }
        if let Some(r) = &self.role {
            parts.push(format!("role={}", url_enc(r)));
        }
        if let Some(f) = &self.from {
            parts.push(format!("from={}", url_enc(f)));
        }
        if let Some(t) = &self.to {
            parts.push(format!("to={}", url_enc(t)));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{}&", parts.join("&"))
        }
    }
}

/// Percent-encodage minimal d'une valeur de query (alphabet sûr conservé, le reste en `%XX`).
fn url_enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Le **formulaire de recherche/filtre** (§7) — statut, exigence (rôle), période — pré-rempli.
fn filter_form(f: &Filters) -> String {
    let sel = |v: &str| if f.status.as_deref() == Some(v) { " selected" } else { "" };
    format!(
        "<form method=\"get\" action=\"/\" class=\"panel\"><h3 style=\"margin-top:0\">Recherche &amp; filtre</h3>\
<label>Statut <select name=\"status\"><option value=\"\">(tous)</option>\
<option value=\"COMPLIANT\"{sc}>CONFORME (record)</option><option value=\"FAIL\"{sf}>ÉCHEC</option>\
<option value=\"EXCEPTION\"{se}>EXCEPTION</option><option value=\"PENDING\"{sp}>EN ATTENTE</option></select></label>\
<label>Exigence (rôle) <input name=\"role\" value=\"{role}\" placeholder=\"sanction_check\"></label>\
<label>Période — de <input name=\"from\" value=\"{from}\" placeholder=\"horodatage\"> à <input name=\"to\" value=\"{to}\"></label>\
<button class=\"btn\" type=\"submit\">Filtrer</button> <a class=\"btn secondary\" href=\"/\">Réinitialiser</a></form>",
        sc = sel("COMPLIANT"),
        sf = sel("FAIL"),
        se = sel("EXCEPTION"),
        sp = sel("PENDING"),
        role = esc(f.role.as_deref().unwrap_or("")),
        from = esc(f.from.as_deref().unwrap_or("")),
        to = esc(f.to.as_deref().unwrap_or("")),
    )
}

/// La **liste des dossiers** (ou la **file d'exceptions** si `exceptions_only`). Le filtre d'exceptions est
/// un préréglage de VUE (statut ≠ CONFORME) — pas une logique de confiance. `base_path` est la route de la
/// vue (`/` ou `/exceptions`), pour que le formulaire et la pagination y reviennent en **préservant** les filtres.
pub fn cases(list: &ListResponse, exceptions_only: bool, filters: &Filters, base_path: &str) -> String {
    let rows: Vec<&CaseSummary> = list
        .cases
        .iter()
        .filter(|c| !exceptions_only || c.status != DecisionStatus::Compliant)
        .collect();
    let mut body = String::new();
    body.push_str(&badge(&list.provenance));
    // Le formulaire de recherche/filtre n'apparaît que sur la liste générale (la file d'exceptions EST un préréglage).
    if !exceptions_only {
        body.push_str(&filter_form(filters));
    }
    if rows.is_empty() {
        body.push_str("<p class=\"muted\">Aucun dossier (pour ces filtres).</p>");
        return body;
    }
    body.push_str("<table><thead><tr><th>Dossier</th><th>Statut</th><th>Records</th><th>Dernier horodatage</th></tr></thead><tbody>");
    for c in rows {
        body.push_str(&format!(
            "<tr><td><a href=\"/cases/{id_attr}\">{id}</a></td><td>{chip}</td><td>{n}</td><td>{ts}</td></tr>",
            id_attr = esc(&c.case_id),
            id = esc(&c.case_id),
            chip = status_chip(c.status),
            n = c.n_records,
            ts = c.last_ts,
        ));
    }
    body.push_str("</tbody></table>");
    if let Some(cur) = &list.next_cursor {
        // La pagination PRÉSERVE les filtres (sinon la page 2 perdrait le filtre courant — piège classique).
        body.push_str(&format!("<p><a href=\"{}?{}after={}\">page suivante →</a></p>", esc(base_path), filters.query_prefix(), esc(cur)));
    }
    body
}

/// La **vue décision** complète : badge, statut, verdict, contrôles+évidences (chacun citant son nœud),
/// flux, oversight ; puis le panneau d'action (si `can_act`) et le bouton d'export (si `can_export`).
pub fn decision(resp: &CaseResponse, role: Role) -> String {
    let d: &DecisionView = &resp.decision;
    let mut body = String::new();
    body.push_str(&badge(&resp.provenance));
    body.push_str(&format!("<p>Statut : {} ", status_chip(d.status)));
    if let Some(label) = &d.verdict_label {
        let node = d.verdict_node.as_deref().unwrap_or("");
        body.push_str(&format!("· verdict <b>{}</b> <span class=\"node\">[{}]</span>", esc(label), esc(node)));
    }
    if d.verdict_conflict {
        body.push_str(" <span class=\"chip c-fail\">CONFLIT DE VERDICT</span>");
    }
    body.push_str("</p>");

    // Contrôles + évidences citées (la chaîne de raisonnement vérifiable).
    if !d.checks.is_empty() {
        body.push_str("<h3>Contrôles</h3><table><thead><tr><th>Exigence</th><th>Conclusion</th><th>Modèle</th><th>Quand</th><th>Évidence citée</th><th>Nœud scellé</th></tr></thead><tbody>");
        for c in &d.checks {
            let mut ev = String::new();
            for e in &c.evidence {
                ev.push_str(&format!(
                    "<div>{} <span class=\"muted\">({})</span> <span class=\"node\">{} · au temps {} · {}</span></div>",
                    esc(&e.retrieved_ref),
                    esc(&e.tool),
                    esc(&short_root(&e.content_hash)),
                    e.at,
                    esc(&e.backed_by_node),
                ));
            }
            if ev.is_empty() {
                ev.push_str("<span class=\"muted\">—</span>");
            }
            body.push_str(&format!(
                "<tr><td>{req}</td><td>{out}</td><td>{model} v{ver}</td><td>{at}</td><td>{ev}</td><td class=\"node\">{node}</td></tr>",
                req = esc(&c.requirement),
                out = esc(&c.outcome),
                model = esc(&c.model),
                ver = esc(&c.version),
                at = c.at,
                node = esc(&c.backed_by_node),
            ));
        }
        body.push_str("</tbody></table>");
    }

    // Flux (communication).
    if !d.flow.is_empty() {
        body.push_str("<h3>Flux</h3><ul>");
        for f in &d.flow {
            body.push_str(&format!(
                "<li>{} → {} <span class=\"muted\">({})</span> <span class=\"node\">{}</span></li>",
                esc(&f.from),
                esc(&f.to),
                esc(&f.artifact_type),
                esc(&f.backed_by_node),
            ));
        }
        body.push_str("</ul>");
    }

    // Oversight humain scellé.
    if !d.oversight.is_empty() {
        body.push_str("<h3>Supervision humaine (scellée)</h3><ul>");
        for o in &d.oversight {
            body.push_str(&format!(
                "<li><b>{}</b> par {} — « {} » <span class=\"node\">au temps {} · {}</span></li>",
                action_label(o.action),
                esc(&o.human_id),
                esc(&o.justification),
                o.at,
                esc(&o.backed_by_node),
            ));
        }
        body.push_str("</ul>");
    }

    // Le panneau d'action : SEULEMENT si le rôle peut agir (cosmétique ; S2 enforce de toute façon).
    if role.can_act() {
        body.push_str(&action_panel(&d.case_id));
    } else {
        body.push_str("<p class=\"muted\">Votre rôle ne scelle pas d'acte de supervision (lecture / attestation).</p>");
    }

    // L'export : SEULEMENT si le rôle peut exporter.
    if role.can_export() {
        body.push_str(&format!(
            "<h3>Exporter le dossier (preuve sans confiance)</h3>\
<p>Produit un <b>bundle autonome</b> que votre auditeur re-prouve <b>sans nous</b> (records scellés + vérifieur à re-compiler + évidence réglementaire). La <b>clé ne part jamais</b>.</p>\
<p><a class=\"btn export\" href=\"/cases/{id}/export?fw=eu_ai_act\">⬇ Exporter (EU AI Act)</a> \
<a class=\"btn export\" href=\"/cases/{id}/export?fw=gdpr\">⬇ GDPR</a> \
<a class=\"btn export\" href=\"/cases/{id}/export?fw=csddd\">⬇ CSDDD</a> \
<a class=\"btn export\" href=\"/cases/{id}/export?fw=lksg\">⬇ LkSG</a></p>",
            id = esc(&d.case_id),
        ));
    }
    body
}

/// Le panneau d'action humaine (le **seul** write — il part vers le gate scellé de S2).
fn action_panel(case_id: &str) -> String {
    format!(
        "<form class=\"panel\" method=\"post\" action=\"/cases/{id}/action\">\
<h3 style=\"margin-top:0\">Revue (scellée par le cœur — vous demandez, il scelle)</h3>\
<label>Étape visée<input name=\"target_step\" placeholder=\"aggregate\" required></label>\
<label>Action<select name=\"action\">\
<option value=\"APPROVE\">APPROUVER</option><option value=\"OVERRIDE\">OUTREPASSER</option>\
<option value=\"ESCALATE\">ESCALADER</option><option value=\"COMMENT\">COMMENTER</option></select></label>\
<label>Motif<textarea name=\"justification\" rows=\"2\" required></textarea></label>\
<button class=\"btn\" type=\"submit\">Sceller la revue</button></form>",
        id = esc(case_id),
    )
}

/// La page de confirmation après une re-preuve froide (« vérifier maintenant »).
pub fn reproved(env: &IntegrityEnvelope) -> String {
    format!("{}<p><a href=\"/\">← retour aux dossiers</a></p>", badge(env))
}

/// Une page d'erreur lisible (404/403/409…), avec un indice d'action quand utile.
pub fn error_page(status: u16, title: &str, message: &str, hint: Option<&str>) -> String {
    let h = hint.map(|x| format!("<p class=\"muted\">{}</p>", esc(x))).unwrap_or_default();
    format!(
        "<div class=\"badge bad\"><h3 class=\"bad-t\">{} — {}</h3><div class=\"muted\">{}</div>{}</div><p><a href=\"/\">← retour</a></p>",
        status,
        esc(title),
        esc(message),
        h,
    )
}

/// La page « connexion requise » (401) — aucune donnée n'est révélée.
pub fn login_required() -> String {
    format!(
        "<!DOCTYPE html><html lang=\"fr\"><head><meta charset=\"utf-8\"><title>Connexion requise</title><style>{STYLE}</style></head><body>\
<header><div class=\"nav\"><b>Console de conformité</b></div></header>\
<div class=\"badge bad\"><h3 class=\"bad-t\">401 — Connexion requise</h3>\
<div class=\"muted\">Aucune session valide. Fournissez un jeton (en-tête <code>Authorization: Bearer …</code> ou cookie <code>session=…</code>).</div></div>\
</body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_api::{AuditService, AuthContext, ProjectionConfig};
    use provenance_capture::LlmCall;
    use provenance_dag::TenantKey;

    fn case_response(broken: bool) -> CaseResponse {
        let key = TenantKey::from_bytes([4u8; 32]);
        let mut svc = AuditService::new(key, ProjectionConfig::default());
        svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
        svc.ingest_llm(&LlmCall { case_id: "SUP-A", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"i", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 2 }).unwrap();
        let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
        if broken {
            // Recharge un store altéré → enveloppe ok:false.
            let tampered = svc.store().to_jsonl().replacen("PASS", "FAIL", 1);
            let recs = provenance_dag::SealedDag::records_from_jsonl(&tampered).unwrap();
            let mut s2 = AuditService::from_records(TenantKey::from_bytes([4u8; 32]), recs, ProjectionConfig::default());
            return s2.case_view("SUP-A", &auth, 99).unwrap();
        }
        svc.case_view("SUP-A", &auth, 42).unwrap()
    }

    #[test]
    fn page_carries_the_montre_neq_prouve_honesty() {
        let html = page("Dossiers", "acme", Role::ComplianceOfficer, "<p>x</p>");
        assert!(html.contains("Montré vérifié ≠ prouvé sans confiance"), "l'honnêteté cardinale est portée");
        assert!(!html.to_lowercase().contains(">correct<"), "jamais « correct » comme verdict");
        assert!(html.contains("exportez"), "le chemin d'export est rappelé (montré → prouvé)");
    }

    #[test]
    fn badge_is_green_with_freshness() {
        let resp = case_response(false);
        let b = badge(&resp.provenance);
        assert!(b.contains("Vérifié"), "vert quand intègre");
        assert!(b.contains("as_of = 42"), "la fraîcheur est montrée (pas supposée)");
        assert!(b.contains("vérifier maintenant"), "la re-preuve froide est à un clic");
        assert!(b.contains("badge ok"));
    }

    #[test]
    fn badge_is_red_fail_loud_when_broken() {
        // I-S3-FAIL-LOUD : un store altéré rend ROUGE avec le record cassé localisé — jamais un vert.
        let resp = case_response(true);
        assert!(!resp.provenance.ok(), "le store rechargé altéré n'est pas intègre");
        let b = badge(&resp.provenance);
        assert!(b.contains("badge bad") && b.contains("Altération détectée"), "rouge fail-loud");
        assert!(b.contains("cassé au record 0"), "le record cassé est localisé");
        assert!(!b.contains("Vérifié — record authentique"), "JAMAIS un faux vert");
    }

    #[test]
    fn decision_cites_backing_nodes_and_gates_action_export() {
        let resp = case_response(false);
        let officer = decision(&resp, Role::ComplianceOfficer);
        assert!(officer.contains("SUP-A::sanction"), "chaque contrôle cite son nœud scellé");
        assert!(officer.contains("Sceller la revue"), "l'officier voit le panneau d'action");
        assert!(officer.contains("Exporter"), "et le bouton d'export");
        // Viewer : ni action ni export.
        let viewer = decision(&resp, Role::Viewer);
        assert!(!viewer.contains("Sceller la revue"), "le Viewer ne voit pas l'action (cosmétique)");
        assert!(!viewer.contains("class=\"btn export\""), "ni l'export");
        // Auditor : exporte mais n'agit pas.
        let auditor = decision(&resp, Role::Auditor);
        assert!(!auditor.contains("Sceller la revue"), "l'auditeur n'agit pas");
        assert!(auditor.contains("Exporter"), "mais il exporte (il atteste)");
    }

    #[test]
    fn escaping_blocks_injection() {
        assert_eq!(esc("<script>&\"'"), "&lt;script&gt;&amp;&quot;&#39;");
    }
}

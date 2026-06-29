// SPDX-License-Identifier: UNLICENSED
//! **L'AuditService** — la façade de LECTURE d'**un** tenant (un store, une clé, un moniteur). C'est ici
//! que se composent les quatre devoirs : **projeter** (§3), **re-prouver à la lecture** (§4), poser
//! l'**enveloppe honnête** (§6), **rédiger** par rôle (§8) — plus l'unique **gate d'écriture** : l'action
//! humaine scellée (§10), et le **seam d'export** (§9).
//!
//! Tout passe par les contrats RÉELS de S0/S1 : `CaptureStore` (le store scellé encapsulant la clé),
//! `reprove`, `record_human`. La console ne touche jamais ce module sans passer par le routeur (§11).

use crate::auth::{AuthContext, Role, REDACTED};
use crate::envelope::IntegrityEnvelope;
use crate::index::CaseIndex;
use crate::monitor::IntegrityMonitor;
use crate::project::{project, DecisionStatus, DecisionView, ProjectionConfig};
use crate::report;
use provenance_capture::{CaptureError, CaptureStore, HumanReview, LlmCall, MessageEvent, ToolCall};
use provenance_dag::{HumanAction, Record, TenantKey};
use serde::Serialize;

/// Les erreurs servies par l'API (le routeur les mappe en codes HTTP).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// Ressource absente (cas/nœud inconnu) → 404.
    NotFound(String),
    /// Rôle insuffisant → 403.
    Forbidden(String),
    /// Requête malformée (segment réservé, action inconnue, cible absente) → 400.
    BadRequest(String),
    /// Le store est altéré : on refuse d'exporter du non-prouvé → 409.
    Integrity(String),
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::NotFound(m) => write!(f, "introuvable : {m}"),
            ApiError::Forbidden(m) => write!(f, "interdit : {m}"),
            ApiError::BadRequest(m) => write!(f, "requête invalide : {m}"),
            ApiError::Integrity(m) => write!(f, "intégrité : {m}"),
        }
    }
}
impl std::error::Error for ApiError {}

// ── réponses (toutes portent l'IntegrityEnvelope) ──

/// La réponse d'une décision : la vue (rédigée selon le rôle) + l'enveloppe.
#[derive(Debug, Clone, Serialize)]
pub struct CaseResponse {
    pub decision: DecisionView,
    pub provenance: IntegrityEnvelope,
}

/// Le résumé d'un cas dans une liste.
#[derive(Debug, Clone, Serialize)]
pub struct CaseSummary {
    pub case_id: String,
    pub status: DecisionStatus,
    pub n_records: usize,
    pub last_ts: u64,
}

/// Les filtres d'une liste de cas (groupés pour une signature nette).
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    pub status: Option<DecisionStatus>,
    pub period: Option<(u64, u64)>,
    /// Curseur de pagination : ne rendre que les cas dont la position de chaîne `> after_pos` (§5.1).
    pub after_pos: Option<usize>,
    pub limit: usize,
}

/// Une page de cas + curseur + enveloppe.
#[derive(Debug, Clone, Serialize)]
pub struct ListResponse {
    pub cases: Vec<CaseSummary>,
    pub next_cursor: Option<usize>,
    pub provenance: IntegrityEnvelope,
}

/// La réponse d'une action humaine scellée.
#[derive(Debug, Clone, Serialize)]
pub struct HumanActionResp {
    /// Le nœud `Human` scellé qui vient d'être ajouté (la preuve de l'oversight).
    pub sealed_node: String,
    pub provenance: IntegrityEnvelope,
}

/// L'entrée d'une action humaine (ce que la console POSTe — elle ne scelle PAS elle-même).
#[derive(Debug, Clone)]
pub struct HumanActionInput {
    /// L'étape (du cas) visée par la supervision (ex. « aggregate »).
    pub target_step: String,
    pub action: HumanAction,
    pub justification: String,
}

/// Le contenu d'export (le seam vers S4 — qui y ajoute `verify` + README + l'empaquetage trustless).
#[derive(Debug, Clone, Serialize)]
pub struct ExportContent {
    /// Le DAG scellé COMPLET en JSONL — l'unité re-prouvable depuis la genèse (cf. §9.1).
    pub records_jsonl: String,
    pub manifest_root: String,
    pub n_records: usize,
    /// **La portée du PAYLOAD scellé** — TOUJOURS le DAG complet du tenant (re-provable depuis la genèse).
    /// Un éventuel filtre `?cases=` ne restreint QUE le récit, **pas** les records scellés : honnêteté
    /// explicite contre le piège « j'ai demandé un cas, je croyais n'exporter que lui » (le par-cas isolé
    /// sans fuite est la forêt de Merkle, §9.2 — pas encore livré).
    pub sealed_scope: &'static str,
    /// Les cas que le `report` raconte (le récit, filtrable — distinct du payload scellé ci-dessus).
    pub report_cases: Vec<String>,
    /// Le récit rendu (Markdown), portant sur `report_cases`.
    pub report: String,
    /// Toujours vrai ici (on refuse d'exporter un store cassé — cf. [`AuditService::export`]).
    pub integrity_ok: bool,
}

/// La portée constante du payload scellé d'un export (cf. [`ExportContent::sealed_scope`]).
pub const SEALED_SCOPE_FULL: &str = "full-tenant-dag (re-provable depuis la genèse ; ?cases ne filtre que le récit)";

/// La façade d'un tenant. (Pas de `Debug` dérivé : `CaptureStore` de S1 n'implémente pas `Debug` —
/// on garde S2 auto-contenu plutôt que de re-toucher S1.)
pub struct AuditService {
    store: CaptureStore,
    monitor: IntegrityMonitor,
    config: ProjectionConfig,
    /// Séquence pour nommer les actes de revue (`human-{n}`) — un id de nœud unique par action.
    human_seq: u64,
}

impl AuditService {
    pub fn new(key: TenantKey, config: ProjectionConfig) -> Self {
        AuditService { store: CaptureStore::new(key), monitor: IntegrityMonitor::new(), config, human_seq: 0 }
    }

    /// Accès en LECTURE au store (la projection l'utilise en interne ; un read externe est inoffensif).
    pub fn store(&self) -> &CaptureStore {
        &self.store
    }

    // ── ingestion S1 (capture) — ACTES, ARÊTES, ÉVIDENCES uniquement ──
    //
    // On expose l'ingestion des trois taps non-humains pour que l'hôte y branche la capture S1 (passerelle,
    // hook, tap d'évidence). On **n'expose PAS** d'ingestion humaine : un nœud `Human` ne peut naître QUE
    // via le gate [`Self::record_human_action`] (RBAC + validation). L'invariant I-S2-HUMAN-GATE devient
    // ainsi **structurel** : il n'existe aucun chemin pour sceller une supervision sans passer par le gate.

    /// Ingeste un **acte** d'agent (capture S1).
    pub fn ingest_llm(&mut self, call: &LlmCall) -> Result<(), CaptureError> {
        self.store.record_llm(call).map(|_| ())
    }
    /// Ingeste un **message** (arête, capture S1).
    pub fn ingest_message(&mut self, ev: &MessageEvent) -> Result<(), CaptureError> {
        self.store.record_message(ev).map(|_| ())
    }
    /// Ingeste une **évidence** (capture S1).
    pub fn ingest_evidence(&mut self, ev: &ToolCall) -> Result<(), CaptureError> {
        self.store.record_evidence(ev).map(|_| ())
    }

    // ── lecture ──

    /// La **décision lisible** d'un cas : projetée (pure), **rédigée** selon le rôle, sous enveloppe
    /// (badge rapide). Échoue en NotFound si le cas est inconnu. Sert même un store cassé — mais avec
    /// une enveloppe `ok:false` (échec FORT et VISIBLE, jamais masqué : la console passe au rouge).
    pub fn case_view(&mut self, case_id: &str, auth: &AuthContext, now: u64) -> Result<CaseResponse, ApiError> {
        if !auth.role.can_view() {
            return Err(ApiError::Forbidden("lecture non autorisée".into()));
        }
        let case_recs: Vec<&Record> = self.store.by_case(case_id);
        if case_recs.is_empty() {
            return Err(ApiError::NotFound(format!("cas « {case_id} »")));
        }
        let mut view = project(&case_recs, case_id, &self.config);
        redact(&mut view, auth.role);
        let (verdict, as_of) = self.monitor.badge(&self.store, now);
        Ok(CaseResponse { decision: view, provenance: IntegrityEnvelope::from_verdict(&verdict, as_of) })
    }

    /// Liste paginée de cas, filtrable par statut et période. Le curseur est la position de chaîne (§5).
    pub fn list_cases(&mut self, query: &ListQuery, auth: &AuthContext, now: u64) -> Result<ListResponse, ApiError> {
        if !auth.role.can_view() {
            return Err(ApiError::Forbidden("lecture non autorisée".into()));
        }
        let limit = query.limit.max(1);
        let idx = CaseIndex::rebuild(self.store.records(), &self.config);
        // Compose les filtres puis pagine sur l'ordre de chaîne.
        let mut entries: Vec<_> = idx.ordered().into_iter().collect();
        if let Some(s) = query.status {
            entries.retain(|e| e.status == s);
        }
        if let Some((from, to)) = query.period {
            entries.retain(|e| e.last_ts >= from && e.last_ts <= to);
        }
        let start = query.after_pos.map(|p| p.saturating_add(1)).unwrap_or(0); // saturate : pas de panique sur ?after=usize::MAX
        entries.retain(|e| e.first_pos >= start);
        let next_cursor = if entries.len() > limit { Some(entries[limit - 1].first_pos) } else { None };
        let cases: Vec<CaseSummary> = entries
            .into_iter()
            .take(limit)
            .map(|e| CaseSummary { case_id: e.case_id.clone(), status: e.status, n_records: e.n_records, last_ts: e.last_ts })
            .collect();
        let (verdict, as_of) = self.monitor.badge(&self.store, now);
        Ok(ListResponse { cases, next_cursor, provenance: IntegrityEnvelope::from_verdict(&verdict, as_of) })
    }

    /// **Re-preuve FROIDE à la demande** (le bouton « vérifier ») — l'attestation autoritaire.
    pub fn reprove_now(&mut self, auth: &AuthContext, now: u64) -> Result<IntegrityEnvelope, ApiError> {
        if !auth.role.can_view() {
            return Err(ApiError::Forbidden("lecture non autorisée".into()));
        }
        let (verdict, as_of) = self.monitor.authoritative(&self.store, now);
        Ok(IntegrityEnvelope::from_verdict(&verdict, as_of))
    }

    /// Le récit Markdown d'un cas (rédigé selon le rôle).
    pub fn report(&mut self, case_id: &str, auth: &AuthContext, now: u64) -> Result<String, ApiError> {
        let resp = self.case_view(case_id, auth, now)?;
        Ok(report::render_case(&resp.decision, &resp.provenance))
    }

    // ── la SEULE écriture : l'action humaine scellée (le gate, §10) ──

    /// Scelle un acte de **supervision humaine**. La console **demande** ; ce cœur Rust **scelle** (elle
    /// ne détient pas la clé, ne calcule pas de HMAC). Enforce le RBAC (`can_act`), valide la cible, puis
    /// `store.record_human` → un nœud `Human` scellé re-prouvable. Un override non scellé est impossible.
    pub fn record_human_action(
        &mut self,
        case_id: &str,
        input: &HumanActionInput,
        auth: &AuthContext,
        now: u64,
    ) -> Result<HumanActionResp, ApiError> {
        if !auth.role.can_act() {
            return Err(ApiError::Forbidden(format!("le rôle {:?} ne peut pas sceller d'acte de supervision", auth.role)));
        }
        // La cible doit exister DANS le cas (sinon référence pendante → l'append S1 la refuserait, mais
        // on rend une 404 explicite plutôt qu'une erreur d'intégrité opaque).
        let target_id = format!("{case_id}::{}", input.target_step);
        let case_recs = self.store.by_case(case_id);
        if case_recs.is_empty() {
            return Err(ApiError::NotFound(format!("cas « {case_id} »")));
        }
        if !case_recs.iter().any(|r| r.node.id() == target_id) {
            return Err(ApiError::NotFound(format!("nœud cible « {target_id} »")));
        }
        // Nomme l'acte de revue (id de nœud unique, distinct de l'acteur `human_id`).
        let step = format!("human-{}", self.human_seq);
        let review = HumanReview {
            case_id,
            step: &step,
            human_id: &auth.subject,
            action: input.action,
            target_step: &input.target_step,
            justification: &input.justification,
            timestamp: now,
        };
        match self.store.record_human(&review) {
            Ok(_) => {
                self.human_seq += 1;
                let sealed_node = format!("{case_id}::{step}");
                // L'action a changé le DAG → enveloppe FROIDE (autoritaire), as_of=now.
                let (verdict, as_of) = self.monitor.authoritative(&self.store, now);
                Ok(HumanActionResp { sealed_node, provenance: IntegrityEnvelope::from_verdict(&verdict, as_of) })
            }
            Err(CaptureError::Naming(m)) => Err(ApiError::BadRequest(m)),
            Err(CaptureError::Dag(e)) => Err(ApiError::BadRequest(e.to_string())),
        }
    }

    // ── le seam d'export (contenu pour S4) ──

    /// Produit le **contenu** d'export (DAG scellé complet + récit) après une re-preuve **froide**. Refuse
    /// (409) si le store est cassé — on n'exporte JAMAIS du non-prouvé. L'unité re-prouvable est le DAG
    /// COMPLET (depuis la genèse, §9.1) ; le récit est filtré aux cas demandés. Le « par-cas isolé » est
    /// la forêt de Merkle (§9.2), un raffinement coordonné S0 — non encore livré, et nommé comme tel.
    pub fn export(&mut self, only_cases: Option<&[String]>, auth: &AuthContext, now: u64) -> Result<ExportContent, ApiError> {
        if !auth.role.can_export() {
            return Err(ApiError::Forbidden(format!("le rôle {:?} ne peut pas exporter", auth.role)));
        }
        let (verdict, as_of) = self.monitor.authoritative(&self.store, now);
        if !verdict.ok {
            return Err(ApiError::Integrity(format!(
                "store altéré (record {:?}) — export refusé : on n'exporte pas du non-prouvé",
                verdict.broken_at
            )));
        }
        let env = IntegrityEnvelope::from_verdict(&verdict, as_of);
        // Quels cas raconter dans le récit (le payload scellé reste le DAG complet, re-provable).
        let idx = CaseIndex::rebuild(self.store.records(), &self.config);
        let case_ids: Vec<String> = match only_cases {
            Some(cs) => cs.to_vec(),
            None => idx.ordered().into_iter().map(|e| e.case_id.clone()).collect(),
        };
        let mut views = Vec::new();
        for cid in &case_ids {
            let recs = self.store.by_case(cid);
            if recs.is_empty() {
                return Err(ApiError::NotFound(format!("cas « {cid} »")));
            }
            let mut v = project(&recs, cid, &self.config);
            redact(&mut v, auth.role);
            views.push(v);
        }
        Ok(ExportContent {
            records_jsonl: self.store.to_jsonl(),
            manifest_root: self.store.manifest_root(),
            n_records: self.store.records().len(),
            sealed_scope: SEALED_SCOPE_FULL,
            report_cases: case_ids,
            report: report::render_export(&views, &env),
            integrity_ok: true,
        })
    }
}

/// **Rédaction** par rôle — opération de VUE (n'altère jamais le sceau). Pour un rôle sans clair (Viewer),
/// on rédige **tous les outputs free-text** susceptibles de porter des données personnelles : le
/// `verdict_label` ET les conclusions de contrôle — ce sont la **même classe** (un `AgentAct.output` brut,
/// non contraint à un token, qui peut contenir un nom) —, plus les **références d'évidence** (un doc peut
/// nommer un sujet), les **justifications**, et l'**identité** de l'acteur humain. On **garde** le `status`
/// (énum à 4 valeurs, non personnel — le résultat-titre coarse), les `verdict_node`/`backed_by_node` (de
/// simples ids), hachages, outils, types : non personnels et nécessaires à la traçabilité. (Le `status`
/// coarse NE subsume PAS le `verdict_label` free-text — d'où la rédaction de ce dernier.)
fn redact(view: &mut DecisionView, role: Role) {
    if role.sees_unredacted() {
        return;
    }
    if view.verdict_label.is_some() {
        view.verdict_label = Some(REDACTED.to_string()); // output free-text de l'agrégation → peut porter de la PII
    }
    for c in &mut view.checks {
        c.outcome = REDACTED.to_string();
        for e in &mut c.evidence {
            e.retrieved_ref = REDACTED.to_string();
        }
    }
    for o in &mut view.oversight {
        o.justification = REDACTED.to_string();
        o.human_id = REDACTED.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthContext;
    use provenance_capture::LlmCall;

    fn svc() -> AuditService {
        let mut s = AuditService::new(TenantKey::from_bytes([7u8; 32]), ProjectionConfig::default());
        // un cas via l'ingestion de capture S1.
        s.ingest_llm(&LlmCall { case_id: "SUP-A", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
        s.ingest_llm(&LlmCall { case_id: "SUP-A", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"i", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 2 }).unwrap();
        s
    }

    fn officer() -> AuthContext {
        AuthContext::new("acme", Role::ComplianceOfficer, "officer:hanna")
    }

    #[test]
    fn case_view_projects_and_attests() {
        let mut s = svc();
        let r = s.case_view("SUP-A", &officer(), 100).unwrap();
        assert_eq!(r.decision.case_id, "SUP-A");
        assert_eq!(r.decision.verdict_label.as_deref(), Some("COMPLIANT"));
        assert!(r.provenance.ok());
        assert_eq!(r.provenance.verdict.as_of, 100);
        // unknown case → 404.
        assert!(matches!(s.case_view("NOPE", &officer(), 100), Err(ApiError::NotFound(_))));
    }

    #[test]
    fn viewer_sees_coarse_status_but_all_freetext_redacted() {
        let mut s = svc();
        let viewer = AuthContext::new("acme", Role::Viewer, "v");
        let r = s.case_view("SUP-A", &viewer, 1).unwrap();
        // Le status coarse (énum non personnel) est montré — le résultat-titre.
        assert_eq!(r.decision.status, DecisionStatus::Compliant);
        // Mais TOUT output free-text est rédigé — verdict_label ET conclusions de contrôle (même classe
        // que la PII ; le status coarse ne subsume PAS le verdict free-text).
        assert_eq!(r.decision.verdict_label.as_deref(), Some(REDACTED), "verdict free-text rédigé");
        assert!(r.decision.checks.iter().all(|c| c.outcome == REDACTED), "conclusions de contrôle rédigées");
        // La traçabilité reste : verdict_node (un id) et backed_by_node intacts.
        assert_eq!(r.decision.verdict_node.as_deref(), Some("SUP-A::aggregate"));
        assert!(r.decision.checks.iter().all(|c| c.backed_by_node.starts_with("SUP-A::")));
        // Un officier (sees_unredacted) voit tout en clair.
        let full = s.case_view("SUP-A", &officer(), 2).unwrap();
        assert_eq!(full.decision.verdict_label.as_deref(), Some("COMPLIANT"));
        assert!(full.decision.checks.iter().any(|c| c.outcome == "PASS"));
    }

    #[test]
    fn human_action_is_gated_and_sealed() {
        let mut s = svc();
        // Viewer ne peut pas sceller.
        let viewer = AuthContext::new("acme", Role::Viewer, "v");
        let input = HumanActionInput { target_step: "aggregate".into(), action: HumanAction::Approve, justification: "ok".into() };
        assert!(matches!(s.record_human_action("SUP-A", &input, &viewer, 5), Err(ApiError::Forbidden(_))));
        // L'officier scelle ; le DAG reste re-prouvable et l'oversight apparaît.
        let resp = s.record_human_action("SUP-A", &input, &officer(), 6).unwrap();
        assert_eq!(resp.sealed_node, "SUP-A::human-0");
        assert!(resp.provenance.ok());
        let view = s.case_view("SUP-A", &officer(), 7).unwrap().decision;
        assert_eq!(view.oversight.len(), 1);
        assert_eq!(view.oversight[0].human_id, "officer:hanna");
        // cible inexistante → 404.
        let bad = HumanActionInput { target_step: "ghost".into(), action: HumanAction::Approve, justification: "x".into() };
        assert!(matches!(s.record_human_action("SUP-A", &bad, &officer(), 8), Err(ApiError::NotFound(_))));
    }

    #[test]
    fn export_requires_role_and_refuses_broken() {
        let mut s = svc();
        let viewer = AuthContext::new("acme", Role::Viewer, "v");
        assert!(matches!(s.export(None, &viewer, 1), Err(ApiError::Forbidden(_))));
        let exp = s.export(None, &officer(), 9).unwrap();
        assert!(exp.integrity_ok && exp.n_records == 2);
        assert!(exp.report.contains("SUP-A"));
        assert!(!exp.records_jsonl.is_empty());
        // La portée scellée est explicite (anti-piège ?cases) : toujours le DAG complet.
        assert!(exp.sealed_scope.starts_with("full-tenant-dag"));
        assert_eq!(exp.report_cases, vec!["SUP-A".to_string()]);
    }
}

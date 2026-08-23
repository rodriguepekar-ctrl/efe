// SPDX-License-Identifier: UNLICENSED
//! **L'export en un clic** (la résolution de « montré → prouvé ») — orchestre la chaîne S3 × S4 × S6 :
//! S2 `export` (re-preuve froide ; refuse un store cassé) → S6 `derive` + `reg_subtree` (l'évidence
//! réglementaire) → S4 `assemble` (le bundle autonome, avec les pièces `reg/`) → un **tar** streamé au
//! navigateur. La **clé tenant** sert à re-prouver (S6/S4) mais n'est **jamais** écrite dans le bundle ni
//! servie : elle reste dans le backend (cf. [`crate::app`]). Le répertoire de staging est **choisi par le
//! serveur** (jamais dérivé du chemin de requête → aucune traversée).

use crate::app::Console;
use crate::tar::tar_dir;
use audit_api::{ApiError, AuthContext};
use forensic_export::{assemble, BundleInput, BundleOpts};
use reg_evidence::{derive, reg_subtree, templates, CaseScope, Framework, MappingSet};
use provenance_dag::SealedDag;
use std::sync::atomic::{AtomicU64, Ordering};

/// Échec d'export — la coquille le rend en HTML (fail-loud pour l'intégrité).
#[derive(Debug)]
pub enum ExportFail {
    /// Rôle insuffisant (S2 refuse).
    Forbidden(String),
    /// Cas/tenant inconnu.
    NotFound(String),
    /// **Store altéré** — on n'exporte JAMAIS du non-prouvé (fail-loud à l'export).
    Integrity(String),
    /// Échec de construction du bundle (dérivation/assemblage/E-S/tar) ou clé absente.
    Build(String),
}
impl std::fmt::Display for ExportFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportFail::Forbidden(m) => write!(f, "{m}"),
            ExportFail::NotFound(m) => write!(f, "{m}"),
            ExportFail::Integrity(m) => write!(f, "{m}"),
            ExportFail::Build(m) => write!(f, "{m}"),
        }
    }
}

/// Le résultat prêt à streamer : le nom de fichier (alphabet sûr) + les octets du tar.
pub struct BundleTar {
    pub filename: String,
    pub bytes: Vec<u8>,
}
impl std::fmt::Debug for BundleTar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // On ne déverse pas les octets du tar dans un log de test (volumineux/binaire) — juste la taille.
        f.debug_struct("BundleTar").field("filename", &self.filename).field("bytes", &format_args!("{} octets", self.bytes.len())).finish()
    }
}

/// La lecture (mapping) pour un cadre donné — **la courante**, décidée en un seul endroit.
///
/// Elle était recopiée ici, cadre par cadre. C'était sans risque tant que les trois portaient le même
/// numéro ; ça ne l'est plus depuis que le GDPR est passé en `2026-08` (l'art. 5(1)(e), que M6 outille).
/// Un dossier exporté par la console doit porter la même lecture que celui qu'émet la passerelle — sans
/// quoi deux dossiers du même produit, sur le même cadre, se contrediraient devant un régulateur.
fn mapping_for(framework: Framework) -> MappingSet {
    templates::courant(framework)
}

static EXPORT_SEQ: AtomicU64 = AtomicU64::new(0);

/// **Construit le bundle** et le rend sous forme de tar. `scope` choisit le dossier réglementaire
/// (par-cas ou tenant complet) ; `framework` choisit le gabarit. Les records empaquetés restent **le DAG
/// complet** (re-provable depuis la genèse, cf. S2 `sealed_scope`) ; seuls le récit et l'évidence reg/
/// sont cadrés sur le `scope`.
pub fn build_bundle_tar(console: &mut Console, auth: &AuthContext, scope: &CaseScope, framework: Framework, now: u64) -> Result<BundleTar, ExportFail> {
    let tenant = auth.tenant.as_str().to_string();
    // La clé reste dans le backend (jamais servie) ; on en a besoin pour re-prouver (S6/S4).
    let key = console.key(&tenant).ok_or_else(|| ExportFail::Build("clé tenant absente du backend".into()))?.clone();

    // 1. S2 export — re-preuve froide ; refuse un store cassé. Le récit est cadré au cas si scope=Case.
    let only_cases: Option<Vec<String>> = match scope {
        CaseScope::Case(c) => Some(vec![c.clone()]),
        CaseScope::FullTenant => None,
    };
    let svc = console.registry_mut().resolve(auth).map_err(|_| ExportFail::NotFound("tenant inconnu".into()))?;
    let exp = svc.export(only_cases.as_deref(), auth, now).map_err(map_api_err)?;

    // 2. S6 — dérive l'évidence réglementaire (re-prouve aussi) et l'empaquette en sous-arbre reg/.
    let records = SealedDag::records_from_jsonl(&exp.records_jsonl).map_err(|e| ExportFail::Build(format!("records illisibles : {e}")))?;
    let mapping = mapping_for(framework);
    let artifact = derive(&records, &key, &mapping, framework, scope).map_err(|e| ExportFail::Build(format!("dérivation reg : {e}")))?;
    let reg_attachments = reg_subtree(&artifact, &mapping);

    // 3. S4 — assemble le bundle autonome (staging choisi par le SERVEUR, jamais le chemin de requête).
    let seq = EXPORT_SEQ.fetch_add(1, Ordering::Relaxed);
    let out_dir = std::env::temp_dir().join(format!("audit-console-export-{}-{}-{}", sanitize(&tenant), std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&out_dir); // garantit absent/vide pour assemble
    let input = BundleInput {
        records_jsonl: exp.records_jsonl.clone(),
        manifest_root: exp.manifest_root.clone(),
        report: exp.report.clone(),
        sealed_scope: exp.sealed_scope.to_string(),
        report_cases: exp.report_cases.clone(),
    };
    let opts = BundleOpts { tenant_label: tenant.clone(), reg_attachments, ..Default::default() };
    let assembled = assemble(&input, &key, &opts, &out_dir);

    // 4. tar (puis nettoyage du répertoire, succès ou échec).
    let result = (|| {
        assembled.map_err(|e| ExportFail::Build(format!("assemblage : {e}")))?;
        let bytes = tar_dir(&out_dir).map_err(|e| ExportFail::Build(format!("tar : {e}")))?;
        let scope_tag = match scope {
            CaseScope::Case(c) => sanitize(c),
            CaseScope::FullTenant => "tenant".to_string(),
        };
        Ok(BundleTar { filename: format!("bundle-{}-{}-{}.tar", sanitize(&tenant), scope_tag, framework.slug()), bytes })
    })();
    let _ = std::fs::remove_dir_all(&out_dir);
    result
}

fn map_api_err(e: ApiError) -> ExportFail {
    match e {
        ApiError::Forbidden(m) => ExportFail::Forbidden(m),
        ApiError::NotFound(m) => ExportFail::NotFound(m),
        ApiError::Integrity(m) => ExportFail::Integrity(m),
        ApiError::BadRequest(m) | ApiError::CursorStale(m) => ExportFail::Build(m),
    }
}

/// Réduit une chaîne à un alphabet sûr pour un nom de fichier (Content-Disposition / chemin de staging).
fn sanitize(s: &str) -> String {
    let cleaned: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    if cleaned.is_empty() {
        "x".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_api::Role;
    use provenance_capture::LlmCall;
    use provenance_dag::TenantKey;

    fn console_with_case() -> Console {
        let mut c = Console::new();
        c.provision("acme", TenantKey::from_bytes([6u8; 32]));
        c.add_session("tok", "acme", Role::ComplianceOfficer, "officer:hanna");
        {
            let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
            let svc = c.resolve_service(&auth).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "aggregate", agent_role: "verdict_aggregate", inputs: b"i", model: "phi4", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 2 }).unwrap();
        }
        c
    }

    #[test]
    fn builds_a_tar_bundle_without_the_key() {
        let mut c = console_with_case();
        let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
        let tar = build_bundle_tar(&mut c, &auth, &CaseScope::Case("SUP-2024".into()), Framework::EuAiAct, 100).unwrap();
        assert!(tar.filename.ends_with(".tar") && tar.filename.contains("eu_ai_act"));
        assert!(!tar.bytes.is_empty() && tar.bytes.len() % 512 == 0, "tar bien formé");
        // I-S3-SOUVERAIN : la clé tenant ([6u8;32] en hex) n'apparaît NULLE PART dans le bundle.
        let key_hex = "06".repeat(32);
        let haystack = String::from_utf8_lossy(&tar.bytes);
        assert!(!haystack.contains(&key_hex), "la clé ne part jamais dans le bundle");
        // Le tar contient bien les pièces attendues (noms en clair dans les en-têtes USTAR).
        assert!(haystack.contains("records.sealed.jsonl"));
        assert!(haystack.contains("reg/eu_ai_act.md"));
        assert!(haystack.contains("verify/"));
    }

    #[test]
    fn export_refuses_a_role_that_cannot_export() {
        let mut c = console_with_case();
        c.add_session("vtok", "acme", Role::Viewer, "v");
        let viewer = AuthContext::new("acme", Role::Viewer, "v");
        let r = build_bundle_tar(&mut c, &viewer, &CaseScope::FullTenant, Framework::EuAiAct, 100);
        assert!(matches!(r, Err(ExportFail::Forbidden(_))), "le Viewer ne peut pas exporter (S2 refuse)");
    }

    #[test]
    fn export_fails_loud_on_a_tampered_store() {
        // I-S3 fail-loud à l'export : un store rechargé altéré → Integrity, jamais un bundle de non-prouvé.
        let mut c = Console::new();
        let mut base = Console::new();
        base.provision("acme", TenantKey::from_bytes([6u8; 32]));
        {
            let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
            let svc = base.resolve_service(&auth).unwrap();
            svc.ingest_llm(&LlmCall { case_id: "SUP-2024", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "phi4", version: "1", output: "PASS", evidence_steps: &[], timestamp: 1 }).unwrap();
        }
        let auth = AuthContext::new("acme", Role::ComplianceOfficer, "o");
        let jsonl = base.resolve_service(&auth).unwrap().store().to_jsonl().replacen("PASS", "FAIL", 1);
        let recs = SealedDag::records_from_jsonl(&jsonl).unwrap();
        c.rehydrate("acme", TenantKey::from_bytes([6u8; 32]), recs);
        let r = build_bundle_tar(&mut c, &auth, &CaseScope::FullTenant, Framework::EuAiAct, 100);
        assert!(matches!(r, Err(ExportFail::Integrity(_))), "store altéré → export refusé (fail-loud) : {r:?}");
    }
}

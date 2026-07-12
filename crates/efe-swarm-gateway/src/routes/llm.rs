// SPDX-License-Identifier: UNLICENSED
//! **Couche 2 — M3 : la surface servie.** `POST /v1/llm/{vote,relay,debate,specialist,cascade}`
//! (+ `GET /v1/llm/audit/:id`). Chaque handler tourne une mission **sous un scope M1** (les
//! providers décorés capturent la provenance), scelle un reçu **M2** re-vérifiable, et est
//! servi en **sync uniquement**. `efe-multi-llm` et `routes/verify.rs` restent **inchangés**.
//!
//! ## Corrections de sécurité/honnêteté (cf. `docs/architecture/COUCHE2-M3-…md`)
//! - **Gate d'egress FAIL-CLOSED + ⊆ registre** ([`egress_gate`]) : le frontière est le chemin
//!   NON-souverain — policy absente ⟹ 403 ; un LLM autorisé mais non enregistré court-circuite
//!   `call_one` avant le décorateur ⟹ faux `Missing` ⟹ rejeté 400.
//! - **Rédaction du canal d'erreur** ([`map_mission_err`]) : jamais le `Display`/`reason` d'une
//!   erreur provider dans le corps HTTP (la clé Gemini y transite via l'URL) — message générique.
//! - **Un seul `mission_id`** threadé au scope ET au scellement (sinon `SwarmMismatch`).
//! - **Arité depuis la REQUÊTE** (K/rounds/N pur), jamais l'output (sauf décision cascade — Tier A).
//!
//! **Auth (§4)** : les handlers utilisent [`LlmCaller`] — sous la feature `oidc`, un JWT
//! **`aud=efe-llm`** (via `state.oidc_llm`, cf. [`crate::oidc::OidcVerifier::llm_from_env`]) est
//! exigé, sans repli keyless, et le RBAC y est **strict** (rôles vides refusés). Un jeton
//! d'essaim souverain ne déverrouille donc PAS l'egress tiers coûteux.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use efe_coordinator::Coordinator;
use efe_multi_llm::{
    run_cascade, run_debate, run_relay, run_specialist, run_vote_3llm, CascadeRequest, DebateRequest,
    MultiLlmError, RelayRequest, SpecialistRequest, Vote3LlmRequest,
};
use efe_swarm_provenance::{blake3_bytes, verify_run_offline, AgentKind};

use crate::auth::LlmCaller;
use crate::error::{GatewayError, GatewayJson};
use crate::mission::{scoped, MissionProvenanceSink, MissionScope};
use crate::mission_graph::{
    seal_and_store_mission_with_request, specialist_declared_arity, topology_cascade,
    topology_debate, topology_relay, topology_specialist, topology_vote,
};
use crate::state::{AdmitReject, SwarmGatewayState};

/// Borne dure des rounds de débat (miroir d'`efe-multi-llm`), pré-validée au handler pour un 400 propre.
const DEBATE_MAX_ROUNDS: u8 = 5;

/// Le `Router` des routes frontière (**relatif** : `vote`, `relay`, …). Monté sous `/v1/llm`.
pub fn routes_llm() -> Router<SwarmGatewayState> {
    Router::new()
        .route("/vote", post(vote))
        .route("/relay", post(relay))
        .route("/debate", post(debate))
        .route("/specialist", post(specialist))
        .route("/cascade", post(cascade))
        .route("/audit/:id", get(audit))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers (le cœur des corrections adverses).
// ─────────────────────────────────────────────────────────────────────────────

/// **Gate d'egress — FAIL-CLOSED + ⊆ registre.** Appelé AVANT tout `run_*`.
/// - Souveraineté : policy absente OU `allowed_llms=None` ⟹ **403** (le frontière n'est pas souverain).
/// - Autorisation : tout LLM hors allowlist ⟹ **403**.
/// - Intégrité : tout LLM **non enregistré** ⟹ **400** (sinon `call_one` court-circuite le
///   décorateur M1 ⟹ record absent ⟹ nœud `Missing` ⟹ faux `verified=false`).
fn egress_gate(coord: &Coordinator, tenant_uuid: &Uuid, llms: &[String]) -> Result<(), GatewayError> {
    let allowed = coord
        .policies
        .get(tenant_uuid)
        .and_then(|p| p.allowed_llms.as_ref())
        .ok_or_else(|| {
            GatewayError::Forbidden(
                "egress frontière refusé : aucune allowlist pour ce tenant (chemin non-souverain, fail-closed)".into(),
            )
        })?;
    for id in llms {
        if !allowed.iter().any(|a| a == id) {
            return Err(GatewayError::Forbidden(format!("LLM hors de l'allowlist du tenant : {id}")));
        }
        if coord.bridge.registry().get(id).is_none() {
            return Err(GatewayError::BadRequest(format!("LLM autorisé mais non enregistré (misconfiguration) : {id}")));
        }
    }
    Ok(())
}

/// **Rédaction du canal d'erreur.** NE sérialise JAMAIS le `Display`/`reason` de l'erreur
/// provider dans le corps HTTP (la clé Gemini transite via l'URL → le `Display` de
/// `reqwest::Error`). Messages **génériques** ; le reçu scellé (M1) reste propre de toute façon.
fn map_mission_err(e: MultiLlmError) -> GatewayError {
    // Le détail brut n'est PAS loggé ici (il embarquerait l'URL keyée de Gemini) — dette :
    // logging structuré rédacté server-side. Le client ne reçoit qu'un statut + message générique.
    match &e {
        MultiLlmError::DebateTooFewLlms { .. } => GatewayError::BadRequest("debate : ≥ 3 LLM distincts requis".into()),
        MultiLlmError::TooManySubTasks { .. } => GatewayError::BadRequest("specialist : trop de sous-tâches".into()),
        MultiLlmError::MonocultureViolation { .. } => {
            GatewayError::BadRequest("specialist : diversité de routage insuffisante (I11) — élargissez l'allowlist".into())
        }
        MultiLlmError::BudgetExceeded { .. } => GatewayError::Budget("budget de mission dépassé".into()),
        MultiLlmError::CrossTenantAccess { .. } => GatewayError::Forbidden("accès cross-tenant refusé".into()),
        _ if e.is_retryable() => GatewayError::Unavailable("provider LLM temporairement indisponible".into()),
        _ => GatewayError::Internal("erreur d'exécution de la mission".into()),
    }
}

/// Le 429 d'admission **INLINE** (calqué sur `routes/run.rs`) — PAS `GatewayError::TooManyRequests`.
fn too_many_response(s: &SwarmGatewayState, reject: AdmitReject) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, "1")],
        Json(json!({
            "error": "too_many_requests",
            "message": reject.message(),
            "reason": reject.reason(),
            "inflight_max": s.admission_max,
            "inflight_per_tenant_max": s.admission_per_tenant_max,
        })),
    )
        .into_response()
}

/// Le barreau de computation (R1..R4) rapporté À CÔTÉ de `verified` (intégrité ≠ correction).
#[derive(Serialize)]
struct HeldStrengthView {
    weakest_rank: Option<u8>,
    weakest_label: Option<String>,
    any_remote: bool,
}

/// Réponse 200 d'une mission servie — `held_strength` vit ICI (PAS dans `N17VerifyView`, qui
/// est partagé avec `verify.rs`/`replay.rs` et ne peut pas gagner de champ).
#[derive(Serialize)]
struct LlmMissionResponse {
    run_id: Uuid,
    output: Value,
    provenance_root: Option<String>,
    n17_root: Option<String>,
    held: HeldStrengthView,
}

/// Recharge le run scellé et le re-prouve (offline) pour surfacer roots + `held_strength`.
async fn mission_response(s: &SwarmGatewayState, run_id: Uuid, tenant: &str, output: Value) -> Result<Response, GatewayError> {
    let stored = s
        .store
        .get(run_id, tenant)
        .await?
        .ok_or_else(|| GatewayError::Internal("run scellé introuvable après persistance".into()))?;
    let key = s.derive_key(tenant)?;
    let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
    let body = LlmMissionResponse {
        run_id,
        output,
        provenance_root: stored.root,
        n17_root: stored.n17_root,
        held: HeldStrengthView {
            weakest_rank: v.held_strength.weakest_rank,
            weakest_label: v.held_strength.weakest_label,
            any_remote: v.held_strength.any_remote,
        },
    };
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// Empreinte blake3 (hex) du texte utilisateur — dans le résumé de requête, jamais le clair.
fn hash_text(t: &str) -> String {
    blake3_bytes(t.as_bytes()).to_hex()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Les 5 handlers servis (sync). Patron : role → 501-si-None → gate → admit → scope → run → seal.
// ─────────────────────────────────────────────────────────────────────────────

pub async fn vote(
    State(s): State<SwarmGatewayState>,
    caller: LlmCaller,
    GatewayJson(req): GatewayJson<Vote3LlmRequest>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    let coord = s.coordinator.as_ref().ok_or_else(|| GatewayError::NotImplemented("coordination frontière non câblée (frontier-llm)".into()))?;
    let tid = s.tenant_id(&caller.tenant)?;
    egress_gate(coord, &tid.0, &req.llms)?;
    let _permit = match s.admit(&caller.tenant) {
        Ok(p) => p,
        Err(reject) => return Ok(too_many_response(&s, reject)),
    };
    let mission_id = Uuid::new_v4();
    let sink = MissionProvenanceSink::new();
    let scope = MissionScope { tenant_id: tid, mission_id, sink: sink.clone() };
    let llms = req.llms.clone();
    let request = json!({ "kind": "llm/vote", "k": llms.len(), "user_text_hash": hash_text(&req.user_text) });
    let resp = scoped(scope, run_vote_3llm(coord, req)).await.map_err(map_mission_err)?;
    let output = serde_json::to_value(&resp).unwrap_or(Value::Null);
    let topo = topology_vote(&llms);
    let run_id = seal_and_store_mission_with_request(&s, &caller.tenant, mission_id, &topo, &sink, output.clone(), request).await?;
    mission_response(&s, run_id, &caller.tenant, output).await
}

pub async fn relay(
    State(s): State<SwarmGatewayState>,
    caller: LlmCaller,
    GatewayJson(req): GatewayJson<RelayRequest>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    let coord = s.coordinator.as_ref().ok_or_else(|| GatewayError::NotImplemented("coordination frontière non câblée (frontier-llm)".into()))?;
    let tid = s.tenant_id(&caller.tenant)?;
    let (planner, implementer, synthesizer) = (req.planner_llm.clone(), req.implementer_llm.clone(), req.synthesizer_llm.clone());
    egress_gate(coord, &tid.0, &[planner.clone(), implementer.clone(), synthesizer.clone()])?;
    let _permit = match s.admit(&caller.tenant) {
        Ok(p) => p,
        Err(reject) => return Ok(too_many_response(&s, reject)),
    };
    let mission_id = Uuid::new_v4();
    let sink = MissionProvenanceSink::new();
    let scope = MissionScope { tenant_id: tid, mission_id, sink: sink.clone() };
    let request = json!({ "kind": "llm/relay", "user_text_hash": hash_text(&req.user_text) });
    let resp = scoped(scope, run_relay(coord, req)).await.map_err(map_mission_err)?;
    let output = serde_json::to_value(&resp).unwrap_or(Value::Null);
    let topo = topology_relay(&planner, &implementer, &synthesizer);
    let run_id = seal_and_store_mission_with_request(&s, &caller.tenant, mission_id, &topo, &sink, output.clone(), request).await?;
    mission_response(&s, run_id, &caller.tenant, output).await
}

pub async fn debate(
    State(s): State<SwarmGatewayState>,
    caller: LlmCaller,
    GatewayJson(req): GatewayJson<DebateRequest>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    // Pré-validation au handler → 400 propre (sinon `run_debate` rend Config → 500).
    if !(1..=DEBATE_MAX_ROUNDS).contains(&req.rounds) {
        return Err(GatewayError::BadRequest(format!("debate : rounds ∈ 1..={DEBATE_MAX_ROUNDS} requis")));
    }
    let coord = s.coordinator.as_ref().ok_or_else(|| GatewayError::NotImplemented("coordination frontière non câblée (frontier-llm)".into()))?;
    let tid = s.tenant_id(&caller.tenant)?;
    let (pro, contra, jury, rounds) = (req.pro_llm.clone(), req.contra_llm.clone(), req.jury_llm.clone(), req.rounds);
    egress_gate(coord, &tid.0, &[pro.clone(), contra.clone(), jury.clone()])?;
    let _permit = match s.admit(&caller.tenant) {
        Ok(p) => p,
        Err(reject) => return Ok(too_many_response(&s, reject)),
    };
    let mission_id = Uuid::new_v4();
    let sink = MissionProvenanceSink::new();
    let scope = MissionScope { tenant_id: tid, mission_id, sink: sink.clone() };
    let request = json!({ "kind": "llm/debate", "rounds": rounds, "user_text_hash": hash_text(&req.user_text) });
    let resp = scoped(scope, run_debate(coord, req)).await.map_err(map_mission_err)?;
    let output = serde_json::to_value(&resp).unwrap_or(Value::Null);
    let topo = topology_debate(&pro, &contra, &jury, rounds); // arité DEPUIS req.rounds
    let run_id = seal_and_store_mission_with_request(&s, &caller.tenant, mission_id, &topo, &sink, output.clone(), request).await?;
    mission_response(&s, run_id, &caller.tenant, output).await
}

pub async fn specialist(
    State(s): State<SwarmGatewayState>,
    caller: LlmCaller,
    GatewayJson(req): GatewayJson<SpecialistRequest>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    let coord = s.coordinator.as_ref().ok_or_else(|| GatewayError::NotImplemented("coordination frontière non câblée (frontier-llm)".into()))?;
    let tid = s.tenant_id(&caller.tenant)?;
    egress_gate(coord, &tid.0, &req.available_llms)?;
    // Arité N re-dérivée par le Decomposer PUR (le même que run_specialist) DEPUIS le user_text.
    let n = specialist_declared_arity(&req.user_text);
    // Anti-500 : < 2 LLM permis + N ≥ 3 ⟹ run_specialist planterait en MonocultureViolation.
    if req.available_llms.len() < 2 && n >= 3 {
        return Err(GatewayError::BadRequest("specialist : ≥ 2 LLM permis requis pour N ≥ 3 (anti-monoculture I11)".into()));
    }
    let _permit = match s.admit(&caller.tenant) {
        Ok(p) => p,
        Err(reject) => return Ok(too_many_response(&s, reject)),
    };
    let mission_id = Uuid::new_v4();
    let sink = MissionProvenanceSink::new();
    let scope = MissionScope { tenant_id: tid, mission_id, sink: sink.clone() };
    let request = json!({ "kind": "llm/specialist", "n": n, "user_text_hash": hash_text(&req.user_text) });
    let resp = scoped(scope, run_specialist(coord, req)).await.map_err(map_mission_err)?;
    let output = serde_json::to_value(&resp).unwrap_or(Value::Null);
    let topo = topology_specialist(n);
    let run_id = seal_and_store_mission_with_request(&s, &caller.tenant, mission_id, &topo, &sink, output.clone(), request).await?;
    mission_response(&s, run_id, &caller.tenant, output).await
}

pub async fn cascade(
    State(s): State<SwarmGatewayState>,
    caller: LlmCaller,
    GatewayJson(req): GatewayJson<CascadeRequest>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    let coord = s.coordinator.as_ref().ok_or_else(|| GatewayError::NotImplemented("coordination frontière non câblée (frontier-llm)".into()))?;
    let tid = s.tenant_id(&caller.tenant)?;
    let (drafter, verifier, validator) = (req.drafter_llm.clone(), req.verifier_llm.clone(), req.validator_llm.clone());
    egress_gate(coord, &tid.0, &[drafter.clone(), verifier.clone(), validator.clone()])?;
    let _permit = match s.admit(&caller.tenant) {
        Ok(p) => p,
        Err(reject) => return Ok(too_many_response(&s, reject)),
    };
    let mission_id = Uuid::new_v4();
    let sink = MissionProvenanceSink::new();
    let scope = MissionScope { tenant_id: tid, mission_id, sink: sink.clone() };
    let user_text_hash = hash_text(&req.user_text);
    let resp = scoped(scope, run_cascade(coord, req)).await.map_err(map_mission_err)?;
    // Résidu Tier A : la décision (2 vs 3 slots) est lue dans la RÉPONSE (committée, tamper-evident,
    // mais non re-dérivable du reçu sans Tier B). Un validator appelé mais mal-déclaré → UndeclaredCall.
    let decision = resp.verifier_decision.clone();
    let output = serde_json::to_value(&resp).unwrap_or(Value::Null);
    let request = json!({ "kind": "llm/cascade", "verifier_decision": decision, "user_text_hash": user_text_hash });
    let topo = topology_cascade(&drafter, &verifier, &validator, &decision);
    let run_id = seal_and_store_mission_with_request(&s, &caller.tenant, mission_id, &topo, &sink, output.clone(), request).await?;
    mission_response(&s, run_id, &caller.tenant, output).await
}

// ─────────────────────────────────────────────────────────────────────────────
//  Le checker out-of-band : re-dérive l'arité DÉCLARÉE depuis la requête scellée, compte les
//  nœuds Mode-C, signale une sous-déclaration. L'axe INVERSE de l'Invariant #2 (surnuméraire).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct LlmAuditView {
    kind: String,
    declared_arity: usize,
    mode_c_records: usize,
    /// `true` si le graphe scellé a MOINS de nœuds Mode-C que la requête scellée n'en implique
    /// (un handler/builder qui aurait sous-déclaré la topologie relativement à la requête).
    under_declared: bool,
}

pub async fn audit(
    State(s): State<SwarmGatewayState>,
    caller: LlmCaller,
    Path(id): Path<Uuid>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;
    let stored = s.store.get(id, &caller.tenant).await?.ok_or(GatewayError::NotFound)?;
    let req = &stored.request;
    let kind = req.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // Arité DÉCLARÉE re-dérivée depuis la REQUÊTE scellée (jamais l'output producteur).
    let declared_arity = match kind.as_str() {
        "llm/vote" => req.get("k").and_then(Value::as_u64).unwrap_or(0) as usize,
        "llm/relay" => 3,
        "llm/debate" => 2 * req.get("rounds").and_then(Value::as_u64).unwrap_or(0) as usize + 1,
        // N (specialist) et la décision (cascade) sont COMMITTÉS (tamper-evident) mais non
        // re-dérivables sans le clair (privacy) / sans Tier B — on lit la valeur scellée.
        "llm/specialist" => req.get("n").and_then(Value::as_u64).unwrap_or(0) as usize,
        "llm/cascade" => 2 + usize::from(req.get("verifier_decision").and_then(Value::as_str) == Some("REGEN_ALL")),
        _ => 0,
    };
    let mode_c_records = stored.records.iter().filter(|r| matches!(r.agent_kind, AgentKind::ModeC)).count();
    Ok(Json(LlmAuditView { kind, declared_arity, mode_c_records, under_declared: declared_arity > mode_c_records }).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mission::ProvenanceProvider;
    use axum::body::Body;
    use axum::http::Request;
    use efe_coordinator::{Coordinator, CoordResult, LlmCall, LlmProvider, LlmResponse, PolicyRegistry, ProviderRegistry, TenantPolicies};
    #[cfg(not(feature = "oidc"))] // MultiLlmBridge n'est utilisé que par les tests audit dev-headers
    use efe_coordinator::MultiLlmBridge;
    use std::sync::Arc;
    use tower::ServiceExt as _;

    // ── Helpers e2e partagés (DoD §10) ──────────────────────────────────────────
    /// État dev + coordinator câblé (providers R4 décorés + allowlist optionnelle) — le
    /// point d'injection `with_coordinator` que le chemin servi utilise en prod.
    fn wired_state(registered: &[&str], allowed: Option<&[&str]>) -> SwarmGatewayState {
        SwarmGatewayState::in_memory().with_coordinator(Arc::new(test_coordinator(registered, allowed)))
    }

    /// Corps `vote` minimal (task_class SCREAMING_SNAKE).
    fn vote_body(llms: &[&str]) -> Value {
        json!({ "user_text": "quelle est la capitale de la France ?", "task_class": "FACTUAL_QA", "llms": llms })
    }

    /// `POST /v1/llm/*` en tenant système, rôle explicite (RBAC frontière strict). Chemin
    /// dev-headers uniquement — sous `oidc`, les tests présentent un JWT (cf. `vote_401_*`).
    #[cfg(not(feature = "oidc"))]
    fn post_req(uri: &str, roles: &str, body: &Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("x-efe-tenant", "system")
            .header("x-efe-roles", roles)
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[cfg(not(feature = "oidc"))]
    async fn body_json(resp: Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Provider factice réseau (endpoint distant → R4), enregistrable pour le gate/e2e.
    struct FakeProvider {
        id: String,
    }
    #[async_trait::async_trait]
    impl LlmProvider for FakeProvider {
        fn id(&self) -> String {
            self.id.clone()
        }
        fn endpoint(&self) -> Option<String> {
            Some("https://api.anthropic.com/v1/messages".into())
        }
        async fn call(&self, _req: LlmCall) -> CoordResult<LlmResponse> {
            Ok(LlmResponse {
                text: format!("answer from {}", self.id),
                tokens_generated: 3,
                cost_cents: 1,
                cost_microcents: 45,
                latency_ms: 7,
                provider: self.id.clone(),
                self_quality: 1.0,
            })
        }
    }

    fn sys_uuid() -> Uuid {
        efe_nano_sdk::TenantId::system().0
    }

    /// Coordinator de test : registre de providers décorés + policy allowlist optionnelle.
    fn test_coordinator(registered: &[&str], allowed: Option<&[&str]>) -> Coordinator {
        let reg = ProviderRegistry::new();
        for id in registered {
            reg.register(ProvenanceProvider::wrap(Arc::new(FakeProvider { id: (*id).into() })));
        }
        let mut builder = Coordinator::builder().registry(reg);
        if let Some(list) = allowed {
            let pol = TenantPolicies { allowed_llms: Some(list.iter().map(|s| s.to_string()).collect()), ..TenantPolicies::open(sys_uuid()) };
            builder = builder.policies(PolicyRegistry::new().with(pol));
        }
        builder.build().expect("coordinator de test")
    }

    #[test]
    fn egress_gate_is_fail_closed_without_policy() {
        // Souveraineté : AUCUNE policy tenant ⟹ 403 (le frontière n'est pas souverain).
        let coord = test_coordinator(&["claude_a"], None);
        let err = egress_gate(&coord, &sys_uuid(), &["claude_a".into()]).unwrap_err();
        assert!(matches!(err, GatewayError::Forbidden(_)), "policy absente ⟹ fail-closed (403)");
    }

    #[test]
    fn egress_gate_rejects_llm_outside_allowlist() {
        let coord = test_coordinator(&["claude_a", "gpt_b"], Some(&["claude_a"]));
        assert!(egress_gate(&coord, &sys_uuid(), &["claude_a".into()]).is_ok(), "autorisé + enregistré ⟹ OK");
        let err = egress_gate(&coord, &sys_uuid(), &["gpt_b".into()]).unwrap_err();
        assert!(matches!(err, GatewayError::Forbidden(_)), "hors allowlist ⟹ 403");
    }

    #[test]
    fn egress_gate_rejects_allowed_but_unregistered_llm() {
        // Intégrité : autorisé mais NON enregistré ⟹ 400 (sinon call_one court-circuite le
        // décorateur M1 → faux Missing → verified=false).
        let coord = test_coordinator(&["claude_a"], Some(&["claude_a", "ghost_c"]));
        let err = egress_gate(&coord, &sys_uuid(), &["ghost_c".into()]).unwrap_err();
        assert!(matches!(err, GatewayError::BadRequest(_)), "autorisé mais non enregistré ⟹ 400");
    }

    /// Router de test avec un état SANS coordinator (→ 501) pour éprouver le montage + le 501.
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test]
    async fn post_vote_without_coordinator_is_501() {
        let state = SwarmGatewayState::in_memory(); // coordinator = None
        let app = crate::router(state);
        let body = json!({ "user_text": "x", "task_class": "FACTUAL_QA", "llms": ["claude_a", "gpt_b", "gemini_c"] });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/llm/vote")
                    .header("content-type", "application/json")
                    .header("x-efe-roles", "operator") // RBAC frontière strict (LlmCaller)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED, "coordinator None ⟹ 501 (jamais un panic)");
    }

    /// Le checker : un run mission scellé (via la couture M2) est audité — arité re-dérivée
    /// depuis la requête scellée vs nœuds Mode-C. Sous-déclaration détectée.
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test]
    async fn audit_detects_under_declaration() {
        use crate::mission_graph::{seal_and_store_mission_with_request, topology_vote};

        let state = SwarmGatewayState::in_memory();
        let tenant = "system";
        let tid = state.tenant_id(tenant).unwrap();
        let mission_id = Uuid::from_u128(0xA0D17);

        // Peuple le puits via un fan-out réel de 2 providers décorés (sous scope).
        let reg = ProviderRegistry::new();
        for id in ["claude_a", "gpt_b"] {
            reg.register(ProvenanceProvider::wrap(Arc::new(FakeProvider { id: id.into() })));
        }
        let bridge = MultiLlmBridge::new(reg);
        let sink = MissionProvenanceSink::new();
        let scope = MissionScope { tenant_id: tid, mission_id, sink: sink.clone() };
        let ids = vec!["claude_a".to_string(), "gpt_b".to_string()];
        scoped(scope, async move { let _ = bridge.call_many_ok(&ids, LlmCall::simple("q", 8)).await; }).await;

        // Le graphe déclare 2 appels, MAIS la requête scellée prétend k=3 (sous-déclaration).
        let topo = topology_vote(&["claude_a".to_string(), "gpt_b".to_string()]);
        let request = json!({ "kind": "llm/vote", "k": 3 });
        let run_id = seal_and_store_mission_with_request(&state, tenant, mission_id, &topo, &sink, json!({}), request).await.unwrap();

        let app = crate::router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/llm/audit/{run_id}"))
                    .header("x-efe-tenant", tenant)
                    .header("x-efe-roles", "viewer") // RBAC frontière strict (LlmCaller)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["declared_arity"], json!(3), "arité re-dérivée depuis la requête scellée (k=3)");
        assert_eq!(v["mode_c_records"], json!(2), "2 nœuds Mode-C réellement scellés");
        assert_eq!(v["under_declared"], json!(true), "3 déclarés > 2 scellés ⟹ sous-déclaration détectée");
    }

    // ── DoD #1 — e2e servi (le gardien fort) ────────────────────────────────────
    /// POST vote (providers autorisés+enregistrés) ⟹ 200 + reçu M2 ; `held.any_remote`
    /// (barreau R4, non-souverain) ; puis GET /v1/swarm/verify ⟹ `verified` +
    /// `n17.bridge_consistent` (prouve le threading d'UN SEUL mission_id, §5).
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test]
    async fn vote_e2e_seals_verifies_and_reports_r4() {
        let state = wired_state(&["claude_a", "gpt_b", "gemini_c"], Some(&["claude_a", "gpt_b", "gemini_c"]));
        let app = crate::router(state);

        let resp = app
            .clone()
            .oneshot(post_req("/v1/llm/vote", "operator", &vote_body(&["claude_a", "gpt_b", "gemini_c"])))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "mission servie ⟹ 200");
        let mission = body_json(resp).await;
        assert!(mission["run_id"].is_string(), "un run scellé est renvoyé");
        assert_eq!(mission["held"]["any_remote"], json!(true), "endpoint distant ⟹ barreau R4 (non-souverain)");
        let run_id = mission["run_id"].as_str().unwrap().to_string();

        // Re-preuve INDÉPENDANTE via la route essaim INCHANGÉE (store partagé, §7).
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/swarm/verify/{run_id}"))
                    .header("x-efe-tenant", "system")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["verified"], json!(true), "mission re-prouvée depuis les records persistés");
        assert_eq!(v["n17"]["bridge_consistent"], json!(true), "pont N17 cohérent ⟹ un seul mission_id threadé (§5)");
    }

    // ── DoD #2 — souveraineté fail-closed (au niveau HTTP) ───────────────────────
    /// Coordinator SANS policy ⟹ POST vote **403** (le gate court-circuite AVANT toute
    /// exécution — aucun run scellé, l'ordre gate→admit→scoped→seal le garantit).
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test]
    async fn vote_403_when_tenant_has_no_policy() {
        let state = wired_state(&["claude_a", "gpt_b", "gemini_c"], None); // aucune allowlist ⟹ fail-closed
        let app = crate::router(state);
        let resp = app
            .oneshot(post_req("/v1/llm/vote", "operator", &vote_body(&["claude_a", "gpt_b", "gemini_c"])))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "policy absente ⟹ 403 (frontière non-souverain)");
    }

    /// Un LLM autorisé mais NON enregistré ⟹ **400** (sinon `call_one` court-circuite le
    /// décorateur M1 → faux `Missing` → `verified=false` sur un run pourtant `Ok`).
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test]
    async fn vote_400_when_llm_allowed_but_unregistered() {
        let state = wired_state(&["claude_a"], Some(&["claude_a", "ghost_c"]));
        let app = crate::router(state);
        let resp = app.oneshot(post_req("/v1/llm/vote", "operator", &vote_body(&["ghost_c"]))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "autorisé mais non enregistré ⟹ 400");
    }

    // ── DoD #3 — rédaction du canal d'erreur (fuite de credential) ───────────────
    /// Une erreur provider portant un secret dans son `Display` (la clé Gemini transite via
    /// l'URL) ⟹ le `GatewayError` mappé est **générique**, jamais le secret. Prouve §3.
    #[test]
    fn map_mission_err_redacts_provider_secret() {
        const SECRET: &str = "SUPERSECRET_gemini_key_123";
        // (a) variante retryable → 503 générique.
        let retryable = MultiLlmError::RelayPhaseFailed {
            phase: "β".into(),
            reason: format!("transport: https://generativelanguage.googleapis.com/v1?key={SECRET}"),
        };
        let mapped = map_mission_err(retryable);
        assert!(matches!(mapped, GatewayError::Unavailable(_)), "RelayPhaseFailed (retryable) ⟹ 503");
        assert!(!format!("{mapped:?}").contains(SECRET), "le secret NE fuit PAS dans l'erreur mappée");
        // (b) variante non-retryable, non-matchée → 500 générique (chemin fallthrough).
        let opaque = MultiLlmError::CanonicalValidation { schema: "plan_v1".into(), error: format!("bad ...{SECRET}...") };
        let mapped = map_mission_err(opaque);
        assert!(matches!(mapped, GatewayError::Internal(_)), "non-matchée ⟹ 500 générique");
        assert!(!format!("{mapped:?}").contains(SECRET), "le secret NE fuit PAS (fallthrough)");
    }

    // ── DoD #5 — erreurs de mission ──────────────────────────────────────────────
    /// MonocultureViolation (I11) ⟹ 400 (jamais 500).
    #[test]
    fn map_mission_err_maps_monoculture_to_400() {
        let e = MultiLlmError::MonocultureViolation { dominant_llm: "claude".into(), ratio: 0.8, limit: 0.6, total: 10 };
        assert!(matches!(map_mission_err(e), GatewayError::BadRequest(_)), "monoculture ⟹ 400, pas 500");
    }

    /// debate `rounds` hors 1..=5 ⟹ 400 (pré-validé au handler, AVANT `run_debate` — donc
    /// même sans coordinator câblé).
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test]
    async fn debate_400_when_rounds_out_of_range() {
        let app = crate::router(SwarmGatewayState::in_memory()); // coordinator=None : le check rounds passe d'abord
        let body = |rounds: u8| json!({ "user_text": "faut-il déployer ?", "pro_llm": "a", "contra_llm": "b", "jury_llm": "c", "rounds": rounds });
        for bad in [0u8, 6, 10] {
            let resp = app.clone().oneshot(post_req("/v1/llm/debate", "operator", &body(bad))).await.unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "rounds={bad} hors 1..=5 ⟹ 400");
        }
    }

    /// specialist : < 2 LLM permis + N ≥ 3 ⟹ 400 (anti-`MonocultureViolation`→500).
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test]
    async fn specialist_400_when_too_few_llms_for_arity() {
        // Texte à 3 phrases ⟹ N=3 (Decomposer heuristique, prouvé côté efe-multi-llm).
        let user_text = "Translate this to Rust. Then explain semantically. Then write a haiku.";
        assert!(specialist_declared_arity(user_text) >= 3, "pré-condition : le texte décompose en ≥ 3 sous-tâches");
        let state = wired_state(&["solo"], Some(&["solo"]));
        let app = crate::router(state);
        let body = json!({ "user_text": user_text, "available_llms": ["solo"], "capability_scores": {} });
        let resp = app.oneshot(post_req("/v1/llm/specialist", "operator", &body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "1 LLM permis + N≥3 ⟹ 400 (jamais 500)");
    }

    // ── DoD #6 — checker (cas négatif + tenant-scope) ────────────────────────────
    /// Vote HONNÊTE 2/2 ⟹ `under_declared=false` (l'axe négatif du checker).
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test]
    async fn audit_negative_when_arity_matches() {
        use crate::mission_graph::{seal_and_store_mission_with_request, topology_vote};
        let state = SwarmGatewayState::in_memory();
        let tid = state.tenant_id("system").unwrap();
        let mission_id = Uuid::from_u128(0xA0D18);
        let reg = ProviderRegistry::new();
        for id in ["claude_a", "gpt_b"] {
            reg.register(ProvenanceProvider::wrap(Arc::new(FakeProvider { id: id.into() })));
        }
        let bridge = MultiLlmBridge::new(reg);
        let sink = MissionProvenanceSink::new();
        let scope = MissionScope { tenant_id: tid, mission_id, sink: sink.clone() };
        let ids = vec!["claude_a".to_string(), "gpt_b".to_string()];
        scoped(scope, async move { let _ = bridge.call_many_ok(&ids, LlmCall::simple("q", 8)).await; }).await;
        // Requête HONNÊTE : k=2 == 2 nœuds Mode-C.
        let topo = topology_vote(&["claude_a".to_string(), "gpt_b".to_string()]);
        let run_id = seal_and_store_mission_with_request(&state, "system", mission_id, &topo, &sink, json!({}), json!({ "kind": "llm/vote", "k": 2 }))
            .await
            .unwrap();

        let app = crate::router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/llm/audit/{run_id}"))
                    .header("x-efe-tenant", "system")
                    .header("x-efe-roles", "auditor")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["declared_arity"], json!(2));
        assert_eq!(v["mode_c_records"], json!(2));
        assert_eq!(v["under_declared"], json!(false), "arité correcte ⟹ pas de sous-déclaration");
    }

    /// Un run scellé sous le tenant A n'est PAS audité par le tenant B ⟹ 404 (store tenant-scopé).
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test]
    async fn audit_is_tenant_scoped_404() {
        use crate::mission_graph::{seal_and_store_mission_with_request, topology_vote};
        let state = SwarmGatewayState::in_memory();
        let tid = state.tenant_id("system").unwrap();
        let mission_id = Uuid::from_u128(0xA0D19);
        let reg = ProviderRegistry::new();
        reg.register(ProvenanceProvider::wrap(Arc::new(FakeProvider { id: "claude_a".into() })));
        let bridge = MultiLlmBridge::new(reg);
        let sink = MissionProvenanceSink::new();
        let scope = MissionScope { tenant_id: tid, mission_id, sink: sink.clone() };
        let ids = vec!["claude_a".to_string()];
        scoped(scope, async move { let _ = bridge.call_many_ok(&ids, LlmCall::simple("q", 8)).await; }).await;
        let topo = topology_vote(&["claude_a".to_string()]);
        let run_id = seal_and_store_mission_with_request(&state, "system", mission_id, &topo, &sink, json!({}), json!({ "kind": "llm/vote", "k": 1 }))
            .await
            .unwrap();

        let app = crate::router(state);
        let other = "22222222-2222-2222-2222-222222222222"; // tenant B
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/llm/audit/{run_id}"))
                    .header("x-efe-tenant", other)
                    .header("x-efe-roles", "auditor")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "run d'un autre tenant ⟹ 404 (isolation)");
    }

    // ── DoD #4 — admission (429 + Retry-After) sur le chemin frontière ───────────
    #[cfg(not(feature = "oidc"))] // chemin dev-headers ; l'auth OIDC est couverte par vote_401_*
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn vote_429_when_at_capacity() {
        let state = wired_state(&["claude_a", "gpt_b", "gemini_c"], Some(&["claude_a", "gpt_b", "gemini_c"])).with_admission_permits(1);
        let app = crate::router(state.clone());
        let _held = state.admission.clone().try_acquire_owned().expect("tient l'unique permit");
        let resp = app.oneshot(post_req("/v1/llm/vote", "operator", &vote_body(&["claude_a", "gpt_b", "gemini_c"]))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS, "à capacité ⟹ 429");
        assert_eq!(
            resp.headers().get("retry-after").and_then(|v| v.to_str().ok()),
            Some("1"),
            "Retry-After présent (backpressure visible)"
        );
    }

    // ── DoD #4 — auth aud=efe-llm (feature `oidc`) ───────────────────────────────
    /// Un JWT d'audience ≠ `efe-llm` ⟹ **401** (LlmCaller strict, pas de repli) ; un JWT
    /// `aud=efe-llm` + rôle `operator` traverse jusqu'à l'exécution (200). Prouve
    /// `state.oidc_llm` + l'extracteur [`crate::auth::LlmCaller`].
    #[cfg(feature = "oidc")]
    #[tokio::test]
    async fn vote_401_when_wrong_audience_then_200_when_efe_llm() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        // Fixtures EC P-256 de TEST (mêmes que oidc.rs — PAS un secret).
        const TEST_PRIV: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgv4ad14ofQ83JXwDG\n\
VEWEF6e59s+JE5EvkQaqvNuxN1ShRANCAARoN8FXUFCqiAOqKjEZBXA9x83gosue\n\
1m0JJbEEeOUyiLXTD1lsnCVeQKm3hfz6Dkk06VYWCtBVfNBO13YdRLC5\n\
-----END PRIVATE KEY-----\n";
        const TEST_PUB: &str = "-----BEGIN PUBLIC KEY-----\n\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEaDfBV1BQqogDqioxGQVwPcfN4KLL\n\
ntZtCSWxBHjlMoi10w9ZbJwlXkCpt4X8+g5JNOlWFgrQVXzQTtd2HUSwuQ==\n\
-----END PUBLIC KEY-----\n";
        let sign = |aud: &str| {
            encode(
                &Header::new(Algorithm::ES256),
                &json!({ "sub": "u", "tenant": "system", "roles": ["operator"], "aud": aud, "exp": 9_999_999_999i64 }),
                &EncodingKey::from_ec_pem(TEST_PRIV.as_bytes()).unwrap(),
            )
            .unwrap()
        };
        let verifier = crate::oidc::OidcVerifier::from_pem(TEST_PUB, None, Some("efe-llm")).unwrap();
        let state = wired_state(&["claude_a", "gpt_b", "gemini_c"], Some(&["claude_a", "gpt_b", "gemini_c"])).with_oidc_llm(Arc::new(verifier));
        let app = crate::router(state);
        let mk = |aud: &str| {
            Request::builder()
                .method("POST")
                .uri("/v1/llm/vote")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", sign(aud)))
                .body(Body::from(vote_body(&["claude_a", "gpt_b", "gemini_c"]).to_string()))
                .unwrap()
        };
        let resp = app.clone().oneshot(mk("efe-agent-runtime")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "aud ≠ efe-llm ⟹ 401");
        let resp = app.oneshot(mk("efe-llm")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "aud=efe-llm + operator ⟹ mission servie");
    }
}

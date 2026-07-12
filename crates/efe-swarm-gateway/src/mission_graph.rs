// SPDX-License-Identifier: UNLICENSED
//! **Couche 2 — M2 : le reçu honnête.**
//!
//! Après une mission (M1 a drainé un `Vec<LlmProvenanceRecord>` du puits), M2 construit un
//! **graphe d'`AgentCallRecord`** depuis la **topologie DÉCLARÉE** de la mission, le scelle
//! (`SwarmChain` → `provenance_root` + `seal_n17` → `n17_root`) et le persiste (`StoredRun`).
//! `GET verify` (inchangé) re-prouve via `verify_run_offline`.
//!
//! ## Les deux invariants du builder (cf. `docs/architecture/COUCHE2-M2-…md`)
//! - **#1 anti reçu-vide** : le graphe a **exactement `arité_déclarée`** nœuds Mode-C ;
//!   l'arité vient de la **requête** (K, rounds) ou d'un **recompute pur** (specialist
//!   `Decomposer`), **jamais** de l'output producteur. On refuse de sceller une topologie vide.
//! - **#2 appariement INJECTIF** : chaque `LlmProvenanceRecord` drainé est consommé **au plus
//!   une fois**. Un nœud déclaré **sans record libre** → lien vers un `ProvenanceId` **absent**
//!   → `Missing` → `fully_proven=false`. Un record **non apparié** (appel non-déclaré) → erreur.
//!   Sans #2, K nœuds pointant le même record présent passeraient tous `Consistent` (le trou).
//!
//! ## Modèle de confiance (honnête)
//! `verify_run_offline` ne **re-dérive aucune arité** : la détection d'omission vit **au build**
//! (builder de confiance) + **tamper-evidence** (clé-détenteur). Le nœud Mode-A « dispatch »
//! committe la déclaration → tamper-evident, mais **pas** re-dérivable par un tiers sans la clé.

use efe_nano_sdk::{AgentId, Blake3Hash, LlmProvenanceRecord, ProvenanceId, SwarmId, TenantId};
use efe_swarm_provenance::{
    blake3_bytes, seal_n17, AgentCallDraft, AgentCallRecord, AgentKind, AgentRole, SwarmChain,
};
use serde_json::{json, Value};
use uuid::Uuid;

/// Un appel LLM **déclaré** par la topologie d'une mission : rôle + filtre provider optionnel.
/// `provider = Some(id)` : le nœud s'apparie à un record de CE provider (fixe-arité, vote/relay/
/// debate/cascade). `None` : s'apparie à n'importe quel record libre (specialist — routing
/// runtime ambigu ; on garantit alors le **compte**, pas l'identité).
#[derive(Debug, Clone)]
pub struct DeclaredCall {
    pub role: AgentRole,
    pub provider: Option<String>,
}

impl DeclaredCall {
    /// Appel déclaré apparié à un provider précis.
    pub fn on(role: AgentRole, provider: impl Into<String>) -> Self {
        Self { role, provider: Some(provider.into()) }
    }
    /// Appel déclaré apparié à n'importe quel record libre (compte seulement).
    pub fn any(role: AgentRole) -> Self {
        Self { role, provider: None }
    }
}

/// La topologie **déclarée** d'une mission : nom, appels attendus, et une déclaration Mode-A
/// additionnelle committée (ex. la décision cascade — Tier A).
#[derive(Debug, Clone)]
pub struct MissionTopology {
    pub kind: String,
    pub calls: Vec<DeclaredCall>,
    /// Committée dans l'`output_hash` du nœud Mode-A « dispatch » (tamper-evident).
    pub declaration: Value,
}

/// Erreur de construction : le reçu n'est **PAS** scellé si l'une survient (fail-closed).
#[derive(Debug, PartialEq, Eq)]
pub enum BuildError {
    /// Aucun appel déclaré pour une mission qui devait en faire ≥1 (Invariant #1).
    EmptyTopology,
    /// Un record drainé n'a été apparié à **aucun** appel déclaré (appel surnuméraire / non-déclaré).
    UndeclaredCall { provider: String },
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::EmptyTopology => write!(f, "topologie vide : aucun appel déclaré (Invariant #1)"),
            BuildError::UndeclaredCall { provider } => {
                write!(f, "appel non-déclaré du provider « {provider} » (Invariant #2 : record surnuméraire)")
            }
        }
    }
}
impl std::error::Error for BuildError {}

/// Le graphe scellé d'une mission — prêt pour `StoredRun::from_mission`.
#[derive(Debug)]
pub struct MissionSeal {
    pub records: Vec<AgentCallRecord>,
    pub provenance_root: Option<String>,
    pub llm_records: Vec<LlmProvenanceRecord>,
    pub n17_leaves: Vec<String>,
    pub n17_root: Option<String>,
}

fn agent_id(name: &str) -> AgentId {
    AgentId::new(name).unwrap_or_else(|_| AgentId::new("mission_node").expect("id constant valide"))
}

/// Consomme (au plus une fois) un record libre du puits appariant `provider` (ou n'importe
/// lequel si `None`). Renvoie `None` si aucun record libre ne correspond → nœud `Missing`.
fn take_matching(pool: &mut [Option<LlmProvenanceRecord>], provider: Option<&str>) -> Option<LlmProvenanceRecord> {
    let idx = pool.iter().position(|slot| match (slot, provider) {
        (Some(r), Some(p)) => r.model_identity.provider == p,
        (Some(_), None) => true,
        _ => false,
    })?;
    pool[idx].take()
}

/// **Le cœur de M2** — construit le graphe depuis la topologie **déclarée** (les 2 invariants)
/// puis scelle (`SwarmChain` + `seal_n17`) avec la **même crypto** que l'essaim (zéro ad hoc).
///
/// Cohérence du pont : chaque nœud Mode-C porte `swarm_id = SwarmId::from_uuid(mission_id)` et
/// `tenant_id = scope.tenant_id` — identiques à ce que M1 pose sur les `LlmProvenanceRecord`
/// (`swarm_request_id = mission_id`) ; sinon `SwarmMismatch`/`TenantMismatch` sur un run honnête.
pub fn build_and_seal_mission(
    tenant_id: TenantId,
    mission_id: Uuid,
    key: &[u8],
    topology: &MissionTopology,
    drained: Vec<LlmProvenanceRecord>,
    output: &Value,
) -> Result<MissionSeal, BuildError> {
    if topology.calls.is_empty() {
        return Err(BuildError::EmptyTopology); // Invariant #1
    }
    let swarm_id = SwarmId::from_uuid(mission_id);

    // Les records seront (a) scellés en N17 tels quels, (b) consommés injectivement pour l'appariement.
    let n17_records = drained.clone();
    let mut pool: Vec<Option<LlmProvenanceRecord>> = drained.into_iter().map(Some).collect();

    let mut drafts: Vec<AgentCallDraft> = Vec::with_capacity(topology.calls.len() + 2);

    // (1) Nœud racine Mode-A « dispatch » : committe la déclaration (kind, arité, providers, extra).
    let dispatch_id = Uuid::new_v4();
    let declared_summary = json!({
        "kind": topology.kind,
        "arity": topology.calls.len(),
        "providers": topology.calls.iter().map(|c| c.provider.clone()).collect::<Vec<_>>(),
        "declaration": topology.declaration,
    });
    let declared_bytes = serde_json::to_vec(&declared_summary).unwrap_or_default();
    drafts.push(
        AgentCallDraft::new(
            swarm_id,
            tenant_id,
            agent_id("dispatch"),
            AgentRole::Stage,
            AgentKind::ModeA,
            blake3_bytes(topology.kind.as_bytes()),
            blake3_bytes(&declared_bytes),
        )
        .with_record_id(dispatch_id),
    );

    // (2) Un nœud Mode-C par appel DÉCLARÉ, apparié INJECTIVEMENT (Invariant #2).
    let mut mode_c_ids: Vec<Uuid> = Vec::with_capacity(topology.calls.len());
    for (i, call) in topology.calls.iter().enumerate() {
        let matched = take_matching(&mut pool, call.provider.as_deref());
        let cid = Uuid::new_v4();
        mode_c_ids.push(cid);
        let (link, input_hash, output_hash) = match &matched {
            Some(r) => (r.provenance_id, r.user_prompt_hash, r.output_hash.unwrap_or(Blake3Hash::ZERO)),
            // Appel DÉCLARÉ mais ABSENT du puits → lien vers un ProvenanceId absent → Missing.
            None => (ProvenanceId::new(), Blake3Hash::ZERO, Blake3Hash::ZERO),
        };
        drafts.push(
            AgentCallDraft::new(swarm_id, tenant_id, agent_id(&format!("call_{i}")), call.role, AgentKind::ModeC, input_hash, output_hash)
                .with_record_id(cid)
                .caused_by(dispatch_id)
                .with_llm_provenance(link),
        );
    }

    // Invariant #2 (seconde moitié) : tout record drainé doit avoir été consommé — sinon appel non-déclaré.
    if let Some(leftover) = pool.into_iter().flatten().next() {
        return Err(BuildError::UndeclaredCall { provider: leftover.model_identity.provider });
    }

    // (3) Nœud réducteur Mode-A : committe la sortie finale, dépend de tous les Mode-C (fan-in).
    let reducer_id = Uuid::new_v4();
    let mut reducer = AgentCallDraft::new(
        swarm_id,
        tenant_id,
        agent_id("reduce"),
        AgentRole::Merger,
        AgentKind::ModeA,
        blake3_bytes(b"reduce"),
        blake3_bytes(&serde_json::to_vec(output).unwrap_or_default()),
    )
    .with_record_id(reducer_id);
    for cid in &mode_c_ids {
        reducer = reducer.caused_by(*cid);
    }
    drafts.push(reducer);

    // Scellement — SwarmChain (essaim) + seal_n17 (forensique), MÊME clé tenant HKDF.
    let mut chain = SwarmChain::new(tenant_id, swarm_id, key.to_vec());
    let records = chain.append_all_canonical(drafts);
    let provenance_root = chain.root_hex();
    let sealed = seal_n17(&n17_records, key);

    Ok(MissionSeal {
        records,
        provenance_root,
        llm_records: n17_records,
        n17_leaves: sealed.leaves_hex(),
        n17_root: sealed.root_hex(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Topologies DÉCLARÉES par mission (arité depuis la requête / recompute pur).
// ─────────────────────────────────────────────────────────────────────────────

/// **VOTE** — K = `req.llms.len()` (param, fixe). Fan-out de K générateurs.
pub fn topology_vote(llms: &[String]) -> MissionTopology {
    MissionTopology {
        kind: "llm/vote".into(),
        calls: llms.iter().map(|p| DeclaredCall::on(AgentRole::Generator, p)).collect(),
        declaration: json!({ "k": llms.len() }),
    }
}

/// **RELAY** — 3 fixe : planner → implementer → synthesizer (providers distincts par construction).
pub fn topology_relay(planner: &str, implementer: &str, synthesizer: &str) -> MissionTopology {
    MissionTopology {
        kind: "llm/relay".into(),
        calls: vec![
            DeclaredCall::on(AgentRole::Solver, planner),
            DeclaredCall::on(AgentRole::Solver, implementer),
            DeclaredCall::on(AgentRole::Solver, synthesizer),
        ],
        declaration: json!({}),
    }
}

/// **DEBATE** — 2·rounds + 1 (rounds = param). Par round : (pro, contra) ; puis 1 jury.
pub fn topology_debate(pro: &str, contra: &str, jury: &str, rounds: u8) -> MissionTopology {
    let mut calls = Vec::with_capacity(2 * rounds as usize + 1);
    for _ in 0..rounds {
        calls.push(DeclaredCall::on(AgentRole::Generator, pro));
        calls.push(DeclaredCall::on(AgentRole::Verifier, contra));
    }
    calls.push(DeclaredCall::on(AgentRole::Judge, jury));
    MissionTopology { kind: "llm/debate".into(), calls, declaration: json!({ "rounds": rounds }) }
}

/// **SPECIALIST** — N = arité **re-dérivée** par [`specialist_declared_arity`] (Decomposer pur),
/// jamais `output.assignments.len()`. Provider = `None` (routing runtime ambigu — compte seulement).
pub fn topology_specialist(sub_task_count: usize) -> MissionTopology {
    MissionTopology {
        kind: "llm/specialist".into(),
        calls: (0..sub_task_count).map(|_| DeclaredCall::any(AgentRole::Solver)).collect(),
        declaration: json!({ "n": sub_task_count }),
    }
}

/// **SPECIALIST** — re-dérive N en **ré-exécutant** le `Decomposer` PUR (même heuristique que la
/// mission) sur `user_text`. **Jamais** depuis l'output producteur. Renvoie 0 en cas d'échec pur.
#[cfg(feature = "frontier-llm")]
pub fn specialist_declared_arity(user_text: &str) -> usize {
    efe_multi_llm::Decomposer::default()
        .decompose(user_text)
        .map(|v| v.len())
        .unwrap_or(0)
}

/// **CASCADE — Tier A (3 slots fixes).** drafter + verifier **toujours** ; le validator est un
/// nœud Mode-C **SSI** `verifier_decision == "REGEN_ALL"` (keyé sur la **décision**, pas
/// `validator_used`). La décision est committée dans le nœud dispatch → présence/absence du
/// validator **tamper-evident**. (Tier B — digest d'ensemble de mots pour re-dériver le jaccard —
/// est un enrichissement M1 différé, cf. doc §9bis.)
pub fn topology_cascade(drafter: &str, verifier: &str, validator: &str, verifier_decision: &str) -> MissionTopology {
    let validator_expected = verifier_decision == "REGEN_ALL";
    let mut calls = vec![
        DeclaredCall::on(AgentRole::Solver, drafter),
        DeclaredCall::on(AgentRole::Verifier, verifier),
    ];
    if validator_expected {
        calls.push(DeclaredCall::on(AgentRole::Solver, validator));
    }
    MissionTopology {
        kind: "llm/cascade".into(),
        calls,
        declaration: json!({ "verifier_decision": verifier_decision, "validator_expected": validator_expected }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  « Le brancher » — orchestration : puits drainé → graphe scellé → StoredRun persisté.
// ─────────────────────────────────────────────────────────────────────────────

/// **Le branchement M2.** Draine le puits de mission, construit+scelle le graphe depuis la
/// topologie **déclarée**, persiste un `StoredRun` **re-vérifiable** (via `RunStore::put_stored`),
/// et renvoie le `run_id`. `GET verify/:run_id` (inchangé) re-prouve — un appel omis → `false`.
///
/// Appelée par un handler M3 après `scoped(scope, run_the_mission).await`.
#[cfg(feature = "frontier-llm")]
pub async fn seal_and_store_mission(
    state: &crate::SwarmGatewayState,
    tenant: &str,
    mission_id: Uuid,
    topology: &MissionTopology,
    sink: &crate::mission::MissionProvenanceSink,
    output: Value,
) -> Result<Uuid, crate::GatewayError> {
    // Résumé de requête minimal (callers non-handler) — le `kind` suffit au store.
    let request = json!({ "kind": topology.kind });
    seal_and_store_mission_with_request(state, tenant, mission_id, topology, sink, output, request).await
}

/// **Couche 2 (M3)** : variante qui **scelle le résumé de requête** (`request`) — la source
/// dont le checker out-of-band `/v1/llm/audit/:id` re-dérive l'arité déclarée. Les handlers M3
/// l'appellent avec `{kind, k, rounds, user_text_hash, verifier_decision}`.
#[cfg(feature = "frontier-llm")]
pub async fn seal_and_store_mission_with_request(
    state: &crate::SwarmGatewayState,
    tenant: &str,
    mission_id: Uuid,
    topology: &MissionTopology,
    sink: &crate::mission::MissionProvenanceSink,
    output: Value,
    request: Value,
) -> Result<Uuid, crate::GatewayError> {
    let tenant_id = state.tenant_id(tenant)?;
    let key = state.derive_key(tenant)?;
    let drained = sink.drain();
    let seal = build_and_seal_mission(tenant_id, mission_id, &key, topology, drained, &output)
        .map_err(|e| crate::GatewayError::Internal(format!("M2 graphe de mission : {e}")))?;
    let stored = crate::StoredRun::from_mission(
        tenant,
        seal.records,
        seal.provenance_root,
        seal.llm_records,
        seal.n17_leaves,
        seal.n17_root,
        output,
        topology.kind.clone(),
        request,
    );
    let run_id = Uuid::new_v4();
    state.store.put_stored(run_id, stored).await?;
    Ok(run_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mission::{scoped, MissionProvenanceSink, MissionScope, ProvenanceProvider};
    use crate::SwarmGatewayState;
    use efe_coordinator::{CoordResult, LlmCall, LlmProvider, LlmResponse, MultiLlmBridge, ProviderRegistry};
    use efe_swarm_provenance::verify_run_offline;
    use std::collections::HashSet;
    use std::sync::Arc;

    const TENANT: &str = "system";

    /// Provider factice réseau (endpoint distant → R4), pour peupler le puits via le décorateur M1.
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

    fn bridge_with(ids: &[&str]) -> MultiLlmBridge {
        let reg = ProviderRegistry::new();
        for id in ids {
            reg.register(ProvenanceProvider::wrap(Arc::new(FakeProvider { id: (*id).into() })));
        }
        MultiLlmBridge::new(reg)
    }

    /// Exécute un fan-out réel sur `called` sous un scope de mission, et renvoie (sink, mission_id).
    async fn run_fanout(state: &SwarmGatewayState, called: &[&str]) -> (Arc<MissionProvenanceSink>, Uuid) {
        let mission_id = Uuid::from_u128(0x5EED_C0DE);
        let tenant_id = state.tenant_id(TENANT).unwrap();
        let sink = MissionProvenanceSink::new();
        let scope = MissionScope { tenant_id, mission_id, sink: sink.clone() };
        let bridge = bridge_with(called);
        let ids: Vec<String> = called.iter().map(|s| s.to_string()).collect();
        scoped(scope, async move {
            let _ = bridge.call_many_ok(&ids, LlmCall::simple("q", 16)).await;
        })
        .await;
        (sink, mission_id)
    }

    #[tokio::test]
    async fn vote_roundtrip_is_fully_proven() {
        // Chemin M2 complet : fan-out réel → drain → build (topologie déclarée) → seal → store → verify.
        let state = SwarmGatewayState::in_memory();
        let providers = ["claude_a", "gpt_b", "gemini_c"];
        let (sink, mission_id) = run_fanout(&state, &providers).await;
        let topo = topology_vote(&providers.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({"winner": "claude_a"})).await.unwrap();

        let stored = state.store.get(run_id, TENANT).await.unwrap().expect("run persisté");
        let key = state.derive_key(TENANT).unwrap();
        let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(v.fully_proven, "un vote complet (3/3) se re-prouve");
        assert!(v.swarm.ok() && v.n17_chain.intact && v.bridge.all_mode_c_consistent());
        assert_eq!(v.bridge.consistent, 3, "3 nœuds Mode-C résolvent");
        assert_eq!(v.bridge.broken, 0);
    }

    #[tokio::test]
    async fn omitted_call_makes_verify_false() {
        // Le TEST-GARDIEN : on DÉCLARE 3 appels mais seuls 2 providers ont été appelés → 1 Missing.
        let state = SwarmGatewayState::in_memory();
        let (sink, mission_id) = run_fanout(&state, &["claude_a", "gpt_b"]).await; // 2 records seulement
        let declared = ["claude_a", "gpt_b", "gemini_c"].iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let topo = topology_vote(&declared); // 3 déclarés
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({})).await.unwrap();

        let stored = state.store.get(run_id, TENANT).await.unwrap().unwrap();
        let key = state.derive_key(TENANT).unwrap();
        let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(!v.fully_proven, "un appel DÉCLARÉ mais omis → verified=false");
        assert!(v.swarm.ok(), "la chaîne d'essaim, elle, reste intègre");
        assert_eq!(v.bridge.broken, 1, "le gemini_c déclaré-mais-absent est Missing");
    }

    #[tokio::test]
    async fn injective_matching_prevents_partial_receipt_forgery() {
        // Invariant #2 : 1 SEUL record de "claude_a" mais 3 appels déclarés du MÊME provider.
        // Sans injectivité, les 3 nœuds pointeraient le même record (Consistent) → false-positif.
        // Avec : 1 apparie, 2 → Missing → verified=false.
        let state = SwarmGatewayState::in_memory();
        let (sink, mission_id) = run_fanout(&state, &["claude_a"]).await; // 1 record
        let topo = topology_vote(&vec!["claude_a".into(), "claude_a".into(), "claude_a".into()]); // 3 déclarés, même provider
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({})).await.unwrap();

        let stored = state.store.get(run_id, TENANT).await.unwrap().unwrap();
        let key = state.derive_key(TENANT).unwrap();
        let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(!v.fully_proven, "reçu partiel (1 record, 3 déclarés) NON prouvé (appariement injectif)");
        assert_eq!(v.bridge.consistent, 1, "un seul nœud apparie le record réel");
        assert_eq!(v.bridge.broken, 2, "les 2 autres sont Missing (pas de réutilisation d'id)");
    }

    #[tokio::test]
    async fn undeclared_extra_call_is_rejected_at_build() {
        // Invariant #2 (autre sens) : un record NON-déclaré (appel surnuméraire) → build refuse.
        let state = SwarmGatewayState::in_memory();
        let (sink, mission_id) = run_fanout(&state, &["claude_a", "gpt_b"]).await; // 2 records
        let key = state.derive_key(TENANT).unwrap();
        let tid = state.tenant_id(TENANT).unwrap();
        let topo = topology_vote(&vec!["claude_a".into()]); // 1 seul déclaré → gpt_b surnuméraire
        let err = build_and_seal_mission(tid, mission_id, &key, &topo, sink.drain(), &json!({})).unwrap_err();
        assert_eq!(err, BuildError::UndeclaredCall { provider: "gpt_b".into() });
    }

    #[test]
    fn empty_topology_is_refused() {
        let topo = MissionTopology { kind: "llm/x".into(), calls: vec![], declaration: json!({}) };
        let err = build_and_seal_mission(TenantId::system(), Uuid::from_u128(1), &[0u8; 32], &topo, vec![], &json!({})).unwrap_err();
        assert_eq!(err, BuildError::EmptyTopology);
    }

    #[tokio::test]
    async fn cascade_tier_a_skips_validator_when_not_regen_all() {
        // Tier A : décision != REGEN_ALL → 2 slots Mode-C (drafter, verifier), pas de validator.
        // Le run est complet et se re-prouve (le validator N'ÉTAIT PAS dû).
        let state = SwarmGatewayState::in_memory();
        let (sink, mission_id) = run_fanout(&state, &["drafter_x", "verifier_y"]).await;
        let topo = topology_cascade("drafter_x", "verifier_y", "validator_z", "ACCEPT");
        assert_eq!(topo.calls.len(), 2, "ACCEPT ⇒ pas de validator déclaré");
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({"verifier_decision": "ACCEPT"})).await.unwrap();
        let stored = state.store.get(run_id, TENANT).await.unwrap().unwrap();
        let key = state.derive_key(TENANT).unwrap();
        let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(v.fully_proven, "cascade sans validator (décision ACCEPT) se re-prouve");
    }

    #[tokio::test]
    async fn cascade_tier_a_requires_validator_when_regen_all() {
        // Tier A : décision == REGEN_ALL déclare 3 slots ; si le validator manque → Missing → false.
        let state = SwarmGatewayState::in_memory();
        let (sink, mission_id) = run_fanout(&state, &["drafter_x", "verifier_y"]).await; // validator ABSENT
        let topo = topology_cascade("drafter_x", "verifier_y", "validator_z", "REGEN_ALL");
        assert_eq!(topo.calls.len(), 3, "REGEN_ALL ⇒ validator déclaré");
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({"verifier_decision": "REGEN_ALL"})).await.unwrap();
        let stored = state.store.get(run_id, TENANT).await.unwrap().unwrap();
        let key = state.derive_key(TENANT).unwrap();
        let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(!v.fully_proven, "validator dû (REGEN_ALL) mais absent → verified=false");
        assert_eq!(v.bridge.broken, 1);
    }

    /// Exécute une SÉQUENCE d'appels (répétitions permises) sous un scope de mission, via
    /// `call_one` — pour les topologies à provider répété (debate) ou à N sous-tâches (specialist).
    async fn run_sequence(state: &SwarmGatewayState, calls: &[&str]) -> (Arc<MissionProvenanceSink>, Uuid) {
        let mission_id = Uuid::from_u128(0x5EED_C0DE);
        let tenant_id = state.tenant_id(TENANT).unwrap();
        let sink = MissionProvenanceSink::new();
        let scope = MissionScope { tenant_id, mission_id, sink: sink.clone() };
        let distinct: Vec<&str> = calls.iter().copied().collect::<HashSet<_>>().into_iter().collect();
        let bridge = bridge_with(&distinct);
        let seq: Vec<String> = calls.iter().map(|s| s.to_string()).collect();
        scoped(scope, async move {
            for id in &seq {
                let _ = bridge.call_one(id, LlmCall::simple("q", 16)).await;
            }
        })
        .await;
        (sink, mission_id)
    }

    #[tokio::test]
    async fn tampered_mission_record_breaks_reproof() {
        // DoD #4 : muter un record du graphe de mission casse la re-preuve (content_intact), localisé.
        let state = SwarmGatewayState::in_memory();
        let providers = ["claude_a", "gpt_b", "gemini_c"];
        let (sink, mission_id) = run_fanout(&state, &providers).await;
        let topo = topology_vote(&providers.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({})).await.unwrap();
        let stored = state.store.get(run_id, TENANT).await.unwrap().unwrap();
        let key = state.derive_key(TENANT).unwrap();

        // sain avant.
        let ok = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(ok.fully_proven, "sain avant falsification");

        // un attaquant réécrit la sortie d'un record du graphe (sans la clé).
        let mut tampered = stored.records.clone();
        tampered[0].output_hash = efe_nano_sdk::Blake3Hash::from_bytes([0xFF; 32]);
        let v = verify_run_offline(&tampered, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(!v.fully_proven, "record muté → re-preuve cassée");
        assert!(!v.swarm.content_intact, "le contenu ne se recalcule plus");
        assert!(v.swarm.first_break.is_some(), "la faute est localisée");
    }

    #[tokio::test]
    async fn relay_roundtrip_is_fully_proven() {
        // DoD #5 (relay) : 3 appels séquentiels, providers distincts.
        let state = SwarmGatewayState::in_memory();
        let (sink, mission_id) = run_sequence(&state, &["planner", "implementer", "synthesizer"]).await;
        let topo = topology_relay("planner", "implementer", "synthesizer");
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({"final": "ok"})).await.unwrap();
        let stored = state.store.get(run_id, TENANT).await.unwrap().unwrap();
        let key = state.derive_key(TENANT).unwrap();
        let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(v.fully_proven, "relay 3/3 se re-prouve");
        assert_eq!(v.bridge.consistent, 3);
    }

    #[tokio::test]
    async fn debate_roundtrip_with_repeated_providers() {
        // DoD #5 (debate) : rounds=2 → 2*2+1=5 appels ; pro/contra RÉPÉTÉS → l'appariement injectif
        // doit lier chaque occurrence à un record DISTINCT (le cas ambigu de la critique).
        let state = SwarmGatewayState::in_memory();
        let (sink, mission_id) = run_sequence(&state, &["pro", "contra", "pro", "contra", "jury"]).await;
        let topo = topology_debate("pro", "contra", "jury", 2);
        assert_eq!(topo.calls.len(), 5, "arité debate = 2*rounds+1");
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({"verdict": "x"})).await.unwrap();
        let stored = state.store.get(run_id, TENANT).await.unwrap().unwrap();
        let key = state.derive_key(TENANT).unwrap();
        let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(v.fully_proven, "debate 5/5 (providers répétés) se re-prouve — injectivité OK");
        assert_eq!(v.bridge.consistent, 5);
    }

    #[tokio::test]
    async fn specialist_arity_is_pure_and_roundtrips() {
        // DoD #6 : l'arité specialist est DÉTERMINISTE (Decomposer PUR) — un egress romprait l'invariant.
        let text = "Analyse le contrat. Puis calcule le risque. Enfin rédige un résumé.";
        let n1 = specialist_declared_arity(text);
        let n2 = specialist_declared_arity(text);
        assert_eq!(n1, n2, "Decomposer pur : N re-dérivé déterministe (jamais depuis l'output producteur)");
        assert!(n1 >= 1, "un texte non vide donne ≥1 sous-tâche");

        // round-trip : N appels réels → topology_specialist(N) (appariement par consommation) → verify true.
        let state = SwarmGatewayState::in_memory();
        let calls: Vec<&str> = (0..n1).map(|_| "slm").collect();
        let (sink, mission_id) = run_sequence(&state, &calls).await;
        let topo = topology_specialist(n1);
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({})).await.unwrap();
        let stored = state.store.get(run_id, TENANT).await.unwrap().unwrap();
        let key = state.derive_key(TENANT).unwrap();
        let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(v.fully_proven, "specialist N/N (N re-dérivé pur) se re-prouve");
        assert_eq!(v.bridge.consistent, n1);
    }

    #[tokio::test]
    async fn cascade_mislabeled_present_validator_is_caught_by_injectivity() {
        // DoD #7 (partie POSITIVE) : si le validator A ÉTÉ appelé (record présent) mais que le
        // producteur déclare ACCEPT (2 slots) pour le cacher, l'Invariant #2 l'ATTRAPE (record
        // surnuméraire → erreur). Le false-ACCEPT n'est indétectable QUE si le validator n'a jamais
        // été appelé (cf. test suivant).
        let state = SwarmGatewayState::in_memory();
        let (sink, mission_id) = run_sequence(&state, &["drafter_x", "verifier_y", "validator_z"]).await; // validator APPELÉ
        let key = state.derive_key(TENANT).unwrap();
        let tid = state.tenant_id(TENANT).unwrap();
        let topo = topology_cascade("drafter_x", "verifier_y", "validator_z", "ACCEPT"); // mensonge : 2 slots déclarés
        let err = build_and_seal_mission(tid, mission_id, &key, &topo, sink.drain(), &json!({})).unwrap_err();
        assert_eq!(
            err,
            BuildError::UndeclaredCall { provider: "validator_z".into() },
            "un validator appelé mais non-déclaré est REJETÉ — le false-ACCEPT est attrapé par l'injectivité"
        );
    }

    #[tokio::test]
    async fn cascade_undue_but_uncalled_validator_is_the_documented_residual() {
        // DoD #7 (partie NÉGATIVE, honnêteté) : le RÉSIDU (§9bis). Si le validator n'a JAMAIS été
        // appelé (0 record) et que le producteur déclare ACCEPT, le reçu se re-prouve `true` — la
        // décision jaccard N'EST PAS re-dérivable offline (textes one-way). Tier B (digest
        // d'ensemble de mots, différé) recomputerait le jaccard et attraperait un ACCEPT frauduleux.
        // Ce test DOCUMENTE la limite (verified=true attendu).
        let state = SwarmGatewayState::in_memory();
        let (sink, mission_id) = run_sequence(&state, &["drafter_x", "verifier_y"]).await; // validator PAS appelé
        let topo = topology_cascade("drafter_x", "verifier_y", "validator_z", "ACCEPT");
        let run_id = seal_and_store_mission(&state, TENANT, mission_id, &topo, &sink, json!({})).await.unwrap();
        let stored = state.store.get(run_id, TENANT).await.unwrap().unwrap();
        let key = state.derive_key(TENANT).unwrap();
        let v = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(v.fully_proven, "RÉSIDU DOCUMENTÉ : ACCEPT sans validator appelé se re-prouve (non détectable sans Tier B)");
    }
}

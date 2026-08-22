// SPDX-License-Identifier: UNLICENSED
//! `GET /v1/swarm/runs/:id` (vue résumée) + `GET /v1/swarm/runs/:id/records` (export des
//! `AgentCallRecord` bruts) + `GET /v1/swarm/runs/:id/n17` (export de la forensique LLM)
//! pour **vérification hors-ligne**, l'indépendance maximale (ADR-016).

use axum::extract::{Path, State};
use axum::Json;
use efe_swarm_provenance::AgentCallRecord;
use efe_swarm_sdk::LlmProvenanceRecord;
use serde::Serialize;
use uuid::Uuid;

use crate::auth::Caller;
use crate::dto::RunStatusView;
use crate::error::GatewayError;
use crate::state::SwarmGatewayState;

/// Statut/résumé d'un run : **terminé** (dans le store) **ou en cours** (job live encore
/// actif). 404 si inconnu pour ce tenant.
pub async fn get_run(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Json<RunStatusView>, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;
    // **M2** : `peek_run` — on REGARDE sans conclure, parce que cette route a une troisième
    // réponse à donner (un job peut être en vol). La conclusion, elle, passe par
    // `absence_expliquee` en fin de fonction : sans quoi le 404 muet reviendrait par cette porte.
    if let Some(stored) = s.peek_run(id, &caller.tenant).await? {
        return Ok(Json(RunStatusView::Finished {
            run_id: id,
            output: stored.output.clone(),
            topology: stored.topology.clone(),
            root: stored.root.clone(),
            records: stored.records.len(),
            created_at_unix: stored.created_at_unix(),
        }));
    }
    // Pas encore persisté : un job live est-il en cours pour ce tenant ?
    let tid = s.tenant_id(&caller.tenant)?;
    if s.dashboards.get(id, &tid).is_some() {
        return Ok(Json(RunStatusView::Running { run_id: id, events: format!("/v1/swarm/runs/{id}/events") }));
    }
    // **M2** : ni persisté, ni en vol. Reste la troisième possibilité, que le 404 taisait —
    // le run a eu lieu et n'a PAS pu être persisté. `absence_expliquee` la distingue.
    Err(crate::capture::absence_expliquee(&s.gaps, id, &caller.tenant))
}

/// **Export hors-ligne** : les `AgentCallRecord` bruts. Un tiers les re-prouve avec sa
/// propre copie de la clé tenant (`verify_swarm`) — **sans rien croire**, même pas ce serveur.
pub async fn get_records(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AgentCallRecord>>, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;
    let stored = s.charger_run(id, &caller.tenant).await?;
    Ok(Json(stored.records))
}

/// **ADR-016** : l'export jumeau de la forensique LLM — `LlmProvenanceRecord` + les
/// feuilles scellées + `n17_root`. Avec `/records` et la clé tenant, un tiers rejoue
/// `verify_run_offline` sur sa propre machine : chaîne d'essaim + chaîne N17 + pont,
/// **sans process vivant, sans provider, sans réseau**.
#[derive(Serialize)]
pub struct N17Export {
    pub run_id: Uuid,
    /// La forensique par-appel (un record par appel LLM Mode C). Vide pour un run Mode A.
    pub llm_records: Vec<LlmProvenanceRecord>,
    /// Les feuilles SHA-256 de contenu (hex, ordre canonique) — pour localiser une mutation.
    pub n17_leaves: Vec<String>,
    /// La racine de la chaîne forensique scellée (hex).
    pub n17_root: Option<String>,
}

pub async fn get_n17(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Json<N17Export>, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;
    let stored = s.charger_run(id, &caller.tenant).await?;
    Ok(Json(N17Export {
        run_id: id,
        llm_records: stored.llm_records,
        n17_leaves: stored.n17_leaves,
        n17_root: stored.n17_root,
    }))
}

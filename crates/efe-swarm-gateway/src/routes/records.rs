// SPDX-License-Identifier: UNLICENSED
//! `GET /v1/swarm/runs/:id` (vue résumée) + `GET /v1/swarm/runs/:id/records` (export des
//! `AgentCallRecord` bruts pour **vérification hors-ligne**, l'indépendance maximale).

use axum::extract::{Path, State};
use axum::Json;
use efe_swarm_provenance::AgentCallRecord;
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
    if let Some(stored) = s.store.get(id, &caller.tenant).await? {
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
    Err(GatewayError::NotFound)
}

/// **Export hors-ligne** : les `AgentCallRecord` bruts. Un tiers les re-prouve avec sa
/// propre copie de la clé tenant (`verify_swarm`) — **sans rien croire**, même pas ce serveur.
pub async fn get_records(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AgentCallRecord>>, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;
    let stored = s.store.get(id, &caller.tenant).await?.ok_or(GatewayError::NotFound)?;
    Ok(Json(stored.records))
}
